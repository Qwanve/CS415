use crate::data::Room;
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use data::{MyState, RoomId, Score};
use serde::{Deserialize, Serialize};

use axum::{
    extract::ws::{Message, WebSocket},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use axum_extra::routing::SpaRouter;
use futures::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use tokio::sync::Mutex;
use tower_http::catch_panic::CatchPanicLayer;

mod card;
use card::Card;

mod data;
use data::Hand;
mod routes;

type Who = SocketAddr;
type Socket = SplitSink<WebSocket, Message>;

#[tokio::main]
async fn main() -> Result<(), impl std::error::Error> {
    //Force initialization in the beginning to ensure all templates parse before
    // opening the server to users
    // Lazy::force(&TERA);
    let state = Arc::new(Mutex::new(data::MyState::default()));
    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/", get(routes::home))
        .route("/create", post(routes::create_room))
        .route("/:id", get(routes::ingame))
        .route("/:id/ws", get(routes::ws_handler))
        .with_state(state)
        .merge(assets)
        .fallback(routes::error_404)
        .layer(CatchPanicLayer::custom(|_| {
            routes::error_500().into_response()
        }));

    let addr = (Ipv4Addr::LOCALHOST, 3000).into();
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn websocket(mut socket: WebSocket, who: SocketAddr, id: RoomId, state: Arc<Mutex<MyState>>) {
    let Ok(_) = socket.send(Message::Ping(vec![1, 2, 3, 4, 5, 6])).await else {
        println!("Could not send ping to {who}");
        return;
    };

    println!("Pinged {who}...");

    let (mut sender, mut socket) = socket.split();

    {
        let lock = &mut state.lock().await.rooms;
        let room = lock.get_mut(&id).unwrap();

        if room.sockets.len() == 0 {
            let msg = serde_json::to_string(&ServerAction::NewHost).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        room.hands.push(Hand::new(who, Vec::new(), false));
        room.sockets.insert(who, sender);
        let action = ServerAction::PlayerJoin {
            player: room.sockets.len(),
        };
        room.notify_all(&action).await;
    }

    loop {
        let Some(msg) = socket.next().await else {
            println!("Connection with {who} closed abruptly");
            return;
        };

        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                println!("Error {e} while recieving from {who}");
                return;
            }
        };

        match msg {
            Message::Text(msg) => match serde_json::from_str(&msg) {
                Ok(PlayerAction::GameStart) => start_game(&state, &id, who).await,
                Ok(PlayerAction::EndTurn) => end_turn(&state, &id, who).await,
                Ok(PlayerAction::Deal) => deal(&state, &id, who).await,
                Ok(PlayerAction::Split) => split(&state, &id, who).await,
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                leave(&state, &id, who).await;
                return;
            }
            _ => println!("Unknown message {msg:?}"),
        }
    }
}

async fn start_game(state: &Arc<Mutex<MyState>>, id: &RoomId, _who: Who) {
    //TODO: Validation
    let mut lock = state.lock().await;
    let room = lock.rooms.get_mut(id).unwrap();
    room.started = true;
    let mut cards = vec![];
    for (index, _hand) in room.hands.iter().enumerate() {
        let card1 = room.decks.pop().unwrap();
        let card2 = room.decks.pop().unwrap();
        let action = ServerAction::Dealt {
            hand: index,
            card: Some(card1),
            second_hand: false,
        };
        room.sockets.notify(&action).await;
        let action = ServerAction::Dealt {
            hand: index,
            card: Some(card2),
            second_hand: false,
        };
        room.sockets.notify(&action).await;
        cards.push([card1, card2]);
    }

    room.hands
        .iter_mut()
        .zip(cards.into_iter())
        .for_each(|(hand, new_cards)| hand.hand.extend_from_slice(&new_cards));

    let cards = room.decks.split_off(room.decks.len() - 2);
    let action = ServerAction::DealDealer { card: None };
    room.notify_all(&action).await;
    let action = ServerAction::DealDealer {
        card: cards.get(1).copied(),
    };
    room.notify_all(&action).await;
    room.dealer_hand.extend_from_slice(&cards);
    //TODO: End game if dealer has blackjack?

    let current = room.current();
    let can_split = current.hand[0].score_card() == current.hand[1].score_card();
    let action = ServerAction::YourTurn { can_split };
    room.notify_current(&action).await;
}

async fn end_turn(state: &Arc<Mutex<MyState>>, id: &RoomId, _who: Who) {
    let mut lock = state.lock().await;
    let room = lock.rooms.get_mut(id).unwrap();
    //TODO: Verify it's the player's turn
    // if !room.players.is_current(|p| p.who == who) {
    //     println!("{who} sent their turn out of order!");
    //     continue;
    // }
    let was_last_player = room.next_hand();
    if was_last_player {
        println!("Game is over");
        room.notify_game_end().await;
        //TODO: Error checking on if the room still exists
        lock.rooms.remove(id).unwrap();
        return;
    }
    let current = room.current();
    let can_split =
        !current.is_second() && current.hand[0].score_card() == current.hand[1].score_card();
    let action = ServerAction::YourTurn { can_split };
    println!("It is now {}'s turn", current.who());
    room.notify_current(&action).await;
}

async fn deal(state: &Arc<Mutex<MyState>>, id: &RoomId, who: Who) {
    println!("{who} has requested a deal");
    let mut lock = state.lock().await;
    let room = lock.rooms.get_mut(id).unwrap();
    //TODO: Verify it's the players turn
    // if !room.players.is_current(|p| p.who == who) {
    //     println!("{who} sent their turn out of order!");
    //     continue;
    // }

    let card = room.decks.pop().unwrap();
    room.current_mut().hand.push(card);
    let second = room.current().is_second();
    let hand = room.find_first_hand(room.current());
    let action = ServerAction::Dealt {
        hand,
        card: Some(card),
        second_hand: second,
    };
    room.notify_all(&action).await;
    //TODO: Busting

    if room.current().hand.len() == 10 {
        println!("{who} has dealt the max hand");
        let action = ServerAction::EndTurn;
        room.notify_current(&action).await;
    }
}

async fn split(state: &Arc<Mutex<MyState>>, id: &RoomId, who: Who) {
    //TODO: Verify
    println!("{who} has requested a split");
    let mut lock = state.lock().await;
    let room = lock.rooms.get_mut(id).unwrap();

    let cards = room.decks.split_off(room.decks.len() - 2);
    let mut hand = Hand::new(who, vec![], true);
    let idx = room.find_first_hand(&hand);
    let mv_card = room.hands[idx].hand.pop().unwrap();
    room.hands[idx].hand.push(cards[1]);

    let action = ServerAction::PlayerSplit { player: idx };
    room.notify_all(&action).await;
    let action = ServerAction::Dealt {
        hand: idx,
        card: Some(cards[0]),
        second_hand: true,
    };
    room.notify_all(&action).await;
    let action = ServerAction::Dealt {
        hand: idx,
        card: Some(cards[1]),
        second_hand: false,
    };
    room.notify_all(&action).await;

    hand.hand.push(mv_card);
    hand.hand.push(cards[0]);
    room.hands.push(hand);
}

async fn leave(state: &Arc<Mutex<MyState>>, id: &RoomId, who: Who) {
    println!("{who} has closed the connection");
    let mut lock = state.lock().await;
    if let Some(room) = lock.rooms.get_mut(id) {
        if room.sockets.len() == 1 {
            lock.rooms.remove(id).unwrap();
            println!("The last player left the game");
            return;
        }

        let current = room.current_hand();
        let old_indexes = room
            .hands
            .iter()
            .enumerate()
            .filter(|(_idx, hand)| hand.who() == &who)
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        let was_current = old_indexes.iter().any(|&idx| idx == current);

        for &idx in &old_indexes {
            let action = ServerAction::PlayerLeave { player: idx };
            room.notify_all(&action).await;
            room.hands.remove(idx);
        }

        let _old_connection = room.sockets.remove(&who).unwrap();

        if was_current {
            if room.started {
                if old_indexes.iter().any(|&idx| idx == room.hands.len()) {
                    room.notify_game_end().await;
                } else {
                    //TODO: call next_hand here?
                    let current = room.current();
                    let can_split = !current.is_second()
                        && current.hand[0].score_card() == current.hand[1].score_card();
                    let action = ServerAction::YourTurn { can_split };
                    room.notify_current(&action).await;
                }
            } else {
                let action = ServerAction::NewHost;
                room.notify_current(&action).await;
            }
        }
    } else {
        println!("Player left non-existent game");
    }
}

impl Room {
    async fn dealer_turn(&mut self) {
        loop {
            let score = self.dealer_hand_dummy().score();
            //TODO: Do I want to sleep here?
            tokio::time::sleep(Duration::from_millis(500)).await;
            match score {
                Score::Bust | Score::Blackjack => break,
                Score::Points(x) if x >= 17 => break,
                Score::Points(_) => {
                    let card = self.decks.pop().unwrap();
                    let action = ServerAction::DealDealer { card: Some(card) };
                    self.notify_all(&action).await;
                    self.dealer_hand.push(card);
                }
            }
        }
    }

    async fn notify_game_end(&mut self) {
        //TODO: Find a better place than this
        self.dealer_turn().await;
        let winning_players = self.calculate_winners();
        let winner_msg = serde_json::to_string(&ServerAction::EndGame {
            winner: true,
            dealer_hand: self.dealer_hand.clone(),
        })
        .unwrap();
        let loser_msg = serde_json::to_string(&ServerAction::EndGame {
            winner: false,
            dealer_hand: self.dealer_hand.clone(),
        })
        .unwrap();
        for (idx, hand) in self.hands.iter().enumerate() {
            let who = hand.who();
            let socket = self.sockets.get_mut(who).unwrap();
            if winning_players.contains(&idx) {
                socket
                    .send(Message::Text(winner_msg.clone()))
                    .await
                    .unwrap();
            } else {
                socket.send(Message::Text(loser_msg.clone())).await.unwrap();
            }
        }
    }

    fn calculate_winners(&mut self) -> Vec<usize> {
        let dealer = self.dealer_hand_dummy().score();
        self.hands
            .iter()
            .enumerate()
            .map(|(i, player)| (i, player.score()))
            //TODO: Pushing
            .filter(|(_idx, score)| score > &dealer)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum PlayerAction {
    GameStart,
    Deal,
    EndTurn,
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ServerAction {
    PlayerJoin {
        player: usize,
    },
    PlayerLeave {
        player: usize,
    },
    NewHost,
    Dealt {
        hand: usize,
        card: Option<Card>,
        second_hand: bool,
    },
    PlayerSplit {
        player: usize,
    },
    // TotalHand {
    //     player: usize,
    //     second_hand: bool,
    //     hand: Vec<Card>,
    // },
    // TotalDealerHand {
    //     hand: Vec<Card>,
    // },
    YourTurn {
        can_split: bool,
    },
    EndTurn,
    EndGame {
        winner: bool,
        dealer_hand: Vec<Card>,
    },
    DealDealer {
        card: Option<Card>,
    },
}
