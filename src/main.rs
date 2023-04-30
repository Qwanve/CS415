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
    http::StatusCode,
    response::IntoResponse,
    response::Redirect,
    response::Response,
    routing::{get, post},
    Router,
};
use axum_extra::routing::SpaRouter;
use axum_login::{
    axum_sessions::{async_session::MemoryStore as SessionMemoryStore, SessionLayer},
    extractors::AuthContext,
    secrecy::SecretVec,
    AuthLayer, AuthUser, RequireAuthorizationLayer, SqliteStore,
};
use futures::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tower::builder::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;

mod card;
use card::Card;

mod data;
use data::Hand;
mod routes;

type Who = SocketAddr;
type Socket = SplitSink<WebSocket, Message>;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    id: i64,
    username: String,
    password: String,
    balance: i64,
}

impl AuthUser<i64, ()> for User {
    fn get_id(&self) -> i64 {
        self.id
    }

    fn get_password_hash(&self) -> SecretVec<u8> {
        SecretVec::new(self.password.clone().into())
    }

    fn get_role(&self) -> Option<()> {
        None
    }
}

type Auth = AuthContext<i64, User, SqliteStore<User>, ()>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    routes::template_force();
    let secret = std::array::from_fn::<u8, 64, _>(|_| fastrand::u8(0..u8::MAX));
    let session_store = SessionMemoryStore::new();
    let session_layer = SessionLayer::new(session_store, &secret);
    let connection = SqlitePool::connect("sqlite://database").await.unwrap();

    sqlx::query!(
        "CREATE TABLE IF NOT EXISTS Users (
            id int NOT NULL UNIQUE PRIMARY KEY,
            username varchar(255) NOT NULL UNIQUE,
            password varchar(255) NOT NULL,
            balance int NOT NULL
        )"
    )
    .execute(&connection)
    .await?;

    sqlx::query!(
        "UPDATE Users
        SET balance = 5000"
    )
    .execute(&connection)
    .await?;

    let database = Arc::new(Mutex::new(connection.clone()));
    let sqlite_store = SqliteStore::<User>::new(connection);
    let auth_layer = AuthLayer::new(sqlite_store, &secret);

    let state = Arc::new(Mutex::new(data::MyState::new()));
    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/create", post(routes::create_room))
        .route("/:id", get(routes::ingame))
        .route("/:id/ws", get(routes::ws_handler))
        .route("/", get(routes::home))
        .route("/logout", post(routes::logout).get(routes::logout))
        .route_layer(RequireAuthorizationLayer::<i64, User, ()>::login())
        .route("/login", get(routes::login).post(routes::recieve_login))
        .with_state((state, database))
        .merge(assets)
        .layer(
            //Redirect to login if unauthorized
            ServiceBuilder::new()
                .layer(session_layer)
                .layer(auth_layer)
                .map_response(|r: Response<_>| {
                    if r.status() == StatusCode::UNAUTHORIZED {
                        Redirect::to("/login").into_response()
                    } else {
                        r
                    }
                }),
        )
        .fallback(routes::error_404)
        .layer(CatchPanicLayer::custom(|_| {
            routes::error_500().into_response()
        }));

    let addr = (Ipv4Addr::LOCALHOST, 3000).into();
    Ok(axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?)
}

async fn websocket(
    mut socket: WebSocket,
    who: SocketAddr,
    id: RoomId,
    state: Arc<Mutex<MyState>>,
    user: User,
) {
    let Ok(_) = socket.send(Message::Ping(vec![1, 2, 3, 4, 5, 6])).await else {
        println!("Could not send ping to {} ({who})", user.username);
        return;
    };

    println!("Pinged {} ({who})", user.username);

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
        room.hands.push(Hand::new(who, Vec::new(), false, user.id));
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
                Ok(PlayerAction::Split) => split(&state, &id, who, user.id).await,
                Ok(PlayerAction::Bet { amount }) => {
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    //TODO: Verify that it's your turn
                    println!("{} ({who}) bet {}", user.username, amount);
                    if amount > 0 {
                        let hand = room.hands.iter_mut().find(|p| *p.who() == who).unwrap();
                        hand.bet = amount;
                    } else {
                        println!("Bad bet amount");
                    }
                    let database = room.database().lock_owned().await;
                    sqlx::query!(
                        "UPDATE Users
                        SET balance = balance - ?
                        WHERE id = ?",
                        amount,
                        user.id
                    )
                    .execute(&*database)
                    .await
                    .unwrap();
                    drop(database);
                    let current = room.current();
                    let can_split = !current.is_second()
                        && current.hand[0].score_card() == current.hand[1].score_card();
                    let action = ServerAction::YourTurn { can_split };
                    println!("It is now {}'s turn", current.who());
                    room.notify_current(&action).await;
                }
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

    let action = ServerAction::RequestBet;
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
        return;
    }
    let current = room.current();
    if !current.is_second() {
        let action = ServerAction::RequestBet;
        room.notify_current(&action).await;
    } else {
        let action = ServerAction::YourTurn { can_split: false };
        room.notify_current(&action).await;
    }
    println!("It is now {}'s turn", current.who());
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

    if room.current().hand.len() == 10 || room.current().score().is_bust() {
        println!("{who} has dealt the max hand");
        let action = ServerAction::EndTurn;
        room.notify_current(&action).await;
    }
}

async fn split(state: &Arc<Mutex<MyState>>, id: &RoomId, who: Who, account_id: i64) {
    //TODO: Verify
    println!("{who} has requested a split");
    let mut lock = state.lock().await;
    let room = lock.rooms.get_mut(id).unwrap();

    let cards = room.decks.split_off(room.decks.len() - 2);
    let mut hand = Hand::new(who, vec![], true, account_id);
    let idx = room.find_first_hand(&hand);
    let mv_card = room.hands[idx].hand.pop().unwrap();
    room.hands[idx].hand.push(cards[1]);
    hand.bet = room.hands[idx].bet;

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
        let _old_connection = room.sockets.remove(&who).unwrap();

        let hands = std::mem::replace(&mut room.hands, vec![]);
        let (old_indexes, remaining_hands): (_, Vec<_>) = hands
            .into_iter()
            .enumerate()
            .partition(|(_, hand)| hand.who() == &who);
        let remaining_hands = remaining_hands.into_iter().map(|p| p.1).collect();
        room.hands = remaining_hands;
        let current = room.current_hand();
        let was_current = old_indexes
            .iter()
            .position(|(idx, _)| *idx == current)
            .is_some();

        for (idx, _) in &old_indexes {
            let action = ServerAction::PlayerLeave { player: *idx };
            room.notify_all(&action).await;
        }

        if was_current {
            if room.started {
                if old_indexes.iter().any(|(idx, _)| *idx == room.hands.len()) {
                    room.notify_game_end().await;
                } else {
                    let action = ServerAction::RequestBet;
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
        for (hand, &result) in self.hands.iter().zip(winning_players.iter()) {
            let amount = i64::try_from(hand.bet).unwrap();
            let diff: i64 = match result {
                GameResult::Lose => 0,
                GameResult::Win => amount * 2,
                GameResult::Push => amount,
                GameResult::Blackjack => (2 * amount) + (amount / 2),
            };
            let database = self.database();
            let database = database.lock().await;
            let id = hand.account_id();
            sqlx::query!(
                "UPDATE Users
                SET balance = balance + ?
                WHERE id = ?",
                diff,
                id,
            )
            .execute(&*database)
            .await
            .unwrap();
            let who = hand.who();
            let socket = self.sockets.get_mut(who).unwrap();
            let message = ServerAction::EndGame {
                result,
                dealer_hand: self.dealer_hand.clone(),
            };
            let message = serde_json::to_string(&message).unwrap();
            socket.send(Message::Text(message)).await.unwrap();
        }
    }

    fn calculate_winners(&mut self) -> Vec<GameResult> {
        let dealer = self.dealer_hand_dummy().score();
        self.hands
            .iter()
            .map(|player| player.score())
            .map(|score| {
                if score.is_bust() {
                    return GameResult::Lose;
                }
                match dealer {
                    Score::Blackjack => {
                        if score.is_blackjack() {
                            GameResult::Push
                        } else {
                            GameResult::Lose
                        }
                    }
                    Score::Bust => {
                        if score.is_blackjack() {
                            GameResult::Blackjack
                        } else {
                            GameResult::Win
                        }
                    }
                    Score::Points(points) => match score {
                        Score::Blackjack => GameResult::Blackjack,
                        Score::Points(p) if p > points => GameResult::Win,
                        Score::Points(p) if p < points => GameResult::Lose,
                        Score::Bust => unreachable!(),
                        _ => GameResult::Push,
                    },
                }
            })
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    Lose,
    Win,
    Push,
    Blackjack,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum PlayerAction {
    GameStart,
    Deal,
    EndTurn,
    Split,
    Bet,
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
    RequestBet,
    YourTurn {
        can_split: bool,
    },
    EndTurn,
    EndGame {
        result: GameResult,
        dealer_hand: Vec<Card>,
    },
    DealDealer {
        card: Option<Card>,
    },
}
