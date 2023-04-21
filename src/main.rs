use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use nanoid::nanoid;
use serde::{Deserialize, Serialize};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use axum_extra::routing::SpaRouter;
use futures::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use nutype::nutype;
use once_cell::sync::Lazy;
use tera::Tera;
use tokio::sync::Mutex;
use tower_http::catch_panic::CatchPanicLayer;

mod card;
use card::Card;

static TERA: Lazy<Tera> = Lazy::new(|| match Tera::new("templates/**/*") {
    Ok(t) => t,
    Err(e) => {
        eprintln!("Error parsing: {e}");
        std::process::exit(1)
    }
});

#[tokio::main]
async fn main() -> Result<(), impl std::error::Error> {
    //Force initialization in the beginning to ensure all templates parse before
    // opening the server to users
    Lazy::force(&TERA);
    let state = Arc::new(Mutex::new(MyState::default()));
    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/", get(home))
        .route("/create", post(create_room))
        .route("/:id", get(ingame))
        .route("/:id/ws", get(ws_handler))
        .with_state(state)
        .merge(assets)
        .fallback(error_404)
        .layer(CatchPanicLayer::custom(|_| error_500().into_response()));

    let addr = (Ipv4Addr::LOCALHOST, 3000).into();
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn home() -> impl IntoResponse {
    Html(TERA.render("index.html", &tera::Context::new()).unwrap())
}

async fn create_room(
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    for _ in 0..10 {
        let id = new_id();
        println!("{who} is attempting to create a room with id {id}");
        let rooms = &mut state.lock().await.rooms;
        if rooms.contains_key(&id) {
            println!("Room {id} already exists");
            continue;
        } else {
            println!("Created room {id}");
            let room = Room {
                started: false,
                hands: vec![],
                dealer_hand: vec![],
                current_hand: 0,
                sockets: HashMap::new(),
                decks: Card::shuffled_decks().into(),
            };
            rooms.insert(id.clone(), room);
            return Redirect::to(&format!("/{id}"));
        }
    }
    panic!("Failed to create a unique id");
}

async fn ingame(
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    id: Option<Path<RoomId>>,
    State(state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    let Some(Path(id)) = id else {
        println!("{who} tried to join with an invalid id");
        return (
            StatusCode::BAD_REQUEST,
            Html(TERA.render("400.html", &tera::Context::new()).unwrap())
        );
    };
    if let Some(room) = state.lock().await.rooms.get(&id) {
        println!("{who} is trying to join game {id}");
        if room.sockets.len() >= 6 || room.started {
            //TODO: Error reporting
            println!("Game with id {id} is too full for {who}");
            return (
                StatusCode::BAD_REQUEST,
                Html(TERA.render("400.html", &tera::Context::new()).unwrap()),
            );
        }
    } else {
        println!("{who} joined a game that doesn't exist");
        return (
            StatusCode::NOT_FOUND,
            Html(TERA.render("404.html", &tera::Context::new()).unwrap()),
        );
    }
    let mut context = tera::Context::new();
    context.insert("id", &id.into_inner());
    return (
        StatusCode::OK,
        Html(TERA.render("game.html", &context).unwrap()),
    );
}

async fn ws_handler(
    ws: Option<WebSocketUpgrade>,
    Path(id): Path<RoomId>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    let Some(ws) = ws else {
        println!("{who} tried to load the websocket page");
        return (
            StatusCode::BAD_REQUEST,
            Html(TERA.render("400.html", &tera::Context::new()).unwrap())
        ).into_response();
    };
    ws.on_upgrade(move |socket| websocket(socket, who, id, state))
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

        if room.hands.len() == 0 {
            let msg = serde_json::to_string(&ServerAction::NewHost).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        room.hands.push(Hand::new(who, Vec::new(), false));
        assert!(room.sockets.insert(who.clone(), sender).is_none());
        room.notify_new_player().await;
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
                Ok(PlayerAction::GameStart) => {
                    //TODO: Validation
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    room.started = true;
                    let mut cards = vec![];
                    for hand in &room.hands {
                        let card1 = room.decks.pop().unwrap();
                        let card2 = room.decks.pop().unwrap();
                        let current_hand = room.hands.iter().position(|p| p == hand).unwrap();
                        deal_card(card1, current_hand, &mut room.sockets, false).await;
                        deal_card(card2, current_hand, &mut room.sockets, false).await;
                        cards.push([card1, card2]);
                    }

                    room.hands
                        .iter_mut()
                        .zip(cards.into_iter())
                        .for_each(|(hand, new_cards)| hand.hand.extend_from_slice(&new_cards));

                    let cards = room.decks.split_off(room.decks.len() - 2);
                    deal_dealer(None, &mut room.sockets).await;
                    deal_dealer(cards.get(1).copied(), &mut room.sockets).await;
                    //TODO: End game if dealer has blackjack?
                    room.dealer_hand = cards;

                    room.notify_next_turn().await;
                }
                Ok(PlayerAction::EndTurn) => {
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    //TODO: Verify it's the player's turn
                    // if !room.players.is_current(|p| p.who == who) {
                    //     println!("{who} sent their turn out of order!");
                    //     continue;
                    // }
                    if room.current_hand == room.hands.len() - 1 {
                        println!("Game is over");
                        room.notify_game_end().await;
                        //TODO: Error checking on if the room still exists
                        lock.rooms.remove(&id).unwrap();
                        return;
                    }
                    room.current_hand += 1;
                    let current = room.current().unwrap();
                    println!("It is now {}'s turn", current.who);
                    room.notify_next_turn().await;
                }
                Ok(PlayerAction::Deal) => {
                    println!("{who} has requested a deal");
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    //TODO: Verify it's the players turn
                    // if !room.players.is_current(|p| p.who == who) {
                    //     println!("{who} sent their turn out of order!");
                    //     continue;
                    // }

                    let card = room.decks.pop().unwrap();
                    room.current_mut().unwrap().hand.push(card);
                    let second = room.current().unwrap().second_hand;
                    let idx = room
                        .hands
                        .iter()
                        .enumerate()
                        .filter(|(_, hand)| !hand.second_hand && hand.who == who)
                        .next()
                        .unwrap()
                        .0;
                    deal_card(card, idx, &mut room.sockets, second).await;
                    //TODO: Busting

                    if room.current().unwrap().hand.len() == 10 {
                        println!("{who} has dealt the max hand");
                        room.notify_turn_end().await;
                    }
                }
                Ok(PlayerAction::Split) => {
                    //TODO: Verify
                    println!("{who} has requested a split");
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();

                    let cards = room.decks.split_off(room.decks.len() - 2);
                    let idx = room
                        .hands
                        .iter()
                        .enumerate()
                        .filter(|(_idx, hand)| !hand.second_hand && hand.who == who)
                        .next()
                        .unwrap()
                        .0;
                    let mv_card = room.hands[idx].hand.pop().unwrap();
                    room.hands[idx].hand.push(cards[1]);
                    room.notify_player_split(idx).await;
                    deal_card(cards[0], idx, &mut room.sockets, true).await;
                    deal_card(cards[1], idx, &mut room.sockets, false).await;
                    let hand = Hand::new(who, vec![cards[0], mv_card], true);
                    room.hands.push(hand);
                }
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                let mut lock = state.lock().await;
                if let Some(room) = lock.rooms.get_mut(&id) {
                    if room.sockets.len() == 1 {
                        lock.rooms.remove(&id).unwrap();
                        println!("The last player left the game");
                        return;
                    }

                    let current = room.current_hand();
                    let old_indexes = room
                        .hands
                        .iter()
                        .enumerate()
                        .filter(|(_idx, hand)| hand.who == who)
                        .map(|(idx, _)| idx)
                        .collect::<Vec<_>>();
                    let was_current = old_indexes.iter().any(|&idx| idx == current);

                    for &idx in &old_indexes {
                        room.hands.remove(idx);
                        room.notify_player_left(idx).await;
                    }

                    let _old_connection = room.sockets.remove(&who).unwrap();

                    if was_current {
                        if room.started {
                            if old_indexes.iter().any(|&idx| idx == room.hands.len()) {
                                room.notify_game_end().await;
                            } else {
                                room.notify_next_turn().await;
                            }
                        } else {
                            room.notify_next_host().await;
                        }
                    }
                } else {
                    println!("Player left non-existent game");
                }
                return;
            }
            _ => println!("Unknown message {msg:?}"),
        }
    }
}

impl Room {
    async fn notify_new_player(&mut self) {
        let players = &mut self.hands;
        let msg = serde_json::to_string(&ServerAction::PlayerJoin {
            player: players.len(),
        })
        .unwrap();
        for player in players {
            let socket = self.sockets.get_mut(&player.who).unwrap();
            socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }
    async fn notify_player_left(&mut self, player: usize) {
        let msg = serde_json::to_string(&ServerAction::PlayerLeave { player }).unwrap();
        for player in &mut self.hands {
            let socket = self.sockets.get_mut(&player.who).unwrap();
            socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }
    async fn notify_next_host(&mut self) {
        let msg = serde_json::to_string(&ServerAction::NewHost).unwrap();
        let current = self.current().unwrap().who;
        //TODO: Verify this is correct
        let socket = self.sockets.get_mut(&current).unwrap();
        socket.send(Message::Text(msg)).await.unwrap();
    }
    async fn notify_player_split(&mut self, player: usize) {
        let msg = serde_json::to_string(&ServerAction::PlayerSplit { player }).unwrap();
        for (_who, socket) in &mut self.sockets {
            socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }
    async fn notify_next_turn(&mut self) {
        let mut remv = vec![];
        for (idx, hand) in self.hands.iter().enumerate().skip(self.current_hand) {
            let can_split =
                !hand.second_hand && score_card(&hand.hand[0]) == score_card(&hand.hand[1]);
            let msg = serde_json::to_string(&ServerAction::YourTurn { can_split }).unwrap();
            let socket = self.sockets.get_mut(&hand.who).unwrap();
            if socket.send(Message::Text(msg)).await.is_err() {
                println!("Failed to send {} turn notification", hand.who);
                self.sockets.remove(&hand.who);
                remv.push(idx);
            } else {
                break;
            }
        }
        for hand in remv {
            self.hands.remove(hand);
        }
    }

    async fn notify_turn_end(&mut self) {
        let msg = serde_json::to_string(&ServerAction::EndTurn).unwrap();
        let current = self.current().unwrap().who;
        let socket = self.sockets.get_mut(&current).unwrap();
        socket.send(Message::Text(msg)).await.unwrap();
    }

    async fn dealer_turn(&mut self) {
        loop {
            let score = score(&self.dealer_hand);
            //TODO: Do I want to sleep here?
            tokio::time::sleep(Duration::from_millis(500)).await;
            match score {
                Score::Bust | Score::Blackjack => break,
                Score::Points(x) if x >= 17 => break,
                Score::Points(_) => {
                    let card = self.decks.pop().unwrap();
                    deal_dealer(Some(card), &mut self.sockets).await;
                    self.dealer_hand.push(card);
                }
            }
        }
    }

    async fn notify_game_end(&mut self) {
        //TODO: Find a better place than this
        self.dealer_turn().await;
        for (mut idx, hand) in self.hands.iter().enumerate() {
            if hand.second_hand {
                idx = self
                    .hands
                    .iter()
                    .enumerate()
                    .filter(|(new_idx, hand)| !hand.second_hand && new_idx != &idx)
                    .map(|(idx, _)| idx)
                    .next()
                    .unwrap();
            }
            let action = ServerAction::TotalHand {
                second_hand: hand.second_hand,
                player: idx,
                hand: hand.hand.clone(),
            };
            let msg = serde_json::to_string(&action).unwrap();
            for (_who, socket) in &mut self.sockets {
                socket.send(Message::Text(msg.clone())).await.unwrap();
            }
        }
        let action = ServerAction::TotalDealerHand {
            hand: self.dealer_hand.clone(),
        };
        let msg = serde_json::to_string(&action).unwrap();
        for (_who, socket) in &mut self.sockets {
            socket.send(Message::Text(msg.clone())).await.unwrap();
        }
        let winning_players = self.calculate_winners();
        let winner_msg = serde_json::to_string(&ServerAction::EndGame { winner: true }).unwrap();
        let loser_msg = serde_json::to_string(&ServerAction::EndGame { winner: false }).unwrap();
        //TODO: Split hand winning
        for (idx, hand) in self.hands.iter().enumerate() {
            let who = hand.who;
            let socket = self.sockets.get_mut(&who).unwrap();
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
        let dealer = score(&self.dealer_hand);
        self.hands
            .iter()
            .enumerate()
            .map(|(i, player)| (i, score(&player.hand)))
            //TODO: Pushing
            .filter(|(_idx, score)| score > &dealer)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    }
    fn current_mut(&mut self) -> Option<&mut Hand> {
        self.hands.get_mut(self.current_hand)
    }

    fn current(&self) -> Option<&Hand> {
        self.hands.get(self.current_hand)
    }

    fn current_hand(&self) -> usize {
        self.current_hand
    }
}

async fn deal_card(
    card: Card,
    hand: usize,
    sockets: &mut HashMap<SocketAddr, SplitSink<WebSocket, Message>>,
    second_hand: bool,
) {
    let action = ServerAction::Dealt {
        hand,
        card: Some(card),
        second_hand,
    };

    let msg = serde_json::to_string(&action).unwrap();
    for (_who, socket) in sockets {
        socket.send(Message::Text(msg.clone())).await.unwrap();
    }
}

async fn deal_dealer(
    card: Option<Card>,
    sockets: &mut HashMap<SocketAddr, SplitSink<WebSocket, Message>>,
) {
    let action = ServerAction::DealDealer { card };
    let msg = serde_json::to_string(&action).unwrap();
    for (_who, socket) in sockets {
        socket.send(Message::Text(msg.clone())).await.unwrap();
    }
}

async fn error_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Html(TERA.render("404.html", &tera::Context::new()).unwrap()),
    )
}

fn error_500() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(TERA.render("500.html", &tera::Context::new()).unwrap()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
enum PlayerAction {
    GameStart,
    Deal,
    EndTurn,
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum ServerAction {
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
    TotalHand {
        player: usize,
        second_hand: bool,
        hand: Vec<Card>,
    },
    TotalDealerHand {
        hand: Vec<Card>,
    },
    YourTurn {
        can_split: bool,
    },
    EndTurn,
    EndGame {
        winner: bool,
    },
    DealDealer {
        card: Option<Card>,
    },
}

#[derive(PartialEq, Eq)]
struct Hand {
    second_hand: bool,
    who: SocketAddr,
    hand: Vec<Card>,
}

impl Hand {
    pub fn new(who: SocketAddr, hand: Vec<Card>, second_hand: bool) -> Hand {
        Hand {
            who,
            hand,
            second_hand,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Score {
    Bust,
    Points(u8),
    Blackjack,
}

fn score_card(card: &Card) -> u8 {
    match card.rank {
        card::Rank::Ace => 1,
        card::Rank::Two => 2,
        card::Rank::Three => 3,
        card::Rank::Four => 4,
        card::Rank::Five => 5,
        card::Rank::Six => 6,
        card::Rank::Seven => 7,
        card::Rank::Eight => 8,
        card::Rank::Nine => 9,
        card::Rank::Ten | card::Rank::Jack | card::Rank::Queen | card::Rank::King => 10,
    }
}

fn score(hand: &Vec<Card>) -> Score {
    let mut score = 0;
    let mut found_ace = false;
    for card in hand.iter() {
        if card.rank == card::Rank::Ace {
            found_ace = true;
        }
        score += score_card(&card);
    }

    if found_ace && score < 12 {
        score += 10;
    }

    if score == 21 {
        Score::Blackjack
    } else if score < 21 {
        Score::Points(score)
    } else {
        Score::Bust
    }
}

struct Room {
    started: bool,
    current_hand: usize,
    dealer_hand: Vec<Card>,
    hands: Vec<Hand>,
    sockets: HashMap<SocketAddr, SplitSink<WebSocket, Message>>,
    decks: Vec<Card>,
}

#[derive(Default)]
struct MyState {
    rooms: HashMap<RoomId, Room>,
}

#[nutype(
    sanitize(trim, lowercase)
    validate(
        max_len = 6,
        min_len = 6,
        with = |s: &str| {
            s.chars().all(char::is_alphabetic)
        }
    )
)]
#[derive(Deserialize, Serialize, *)]
struct RoomId(String);

impl std::fmt::Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.clone().into_inner())
    }
}

fn new_id() -> RoomId {
    let alphabet = ('a'..='z').collect::<Vec<_>>();
    let id = nanoid!(6, &alphabet);
    RoomId::new(id).unwrap()
}
