use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
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
                players: Cycler::default(),
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
        if room.players.len() >= 6 || room.started {
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

        if room.players.len() == 0 {
            let msg = serde_json::to_string(&ServerAction::NewHost).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        room.players.add(Player::new(who, sender, Vec::new()));
        room.players.notify_new_player().await;
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
                    for _ in 0..room.players.len() {
                        let card1 = room.decks.pop().unwrap();
                        let card2 = room.decks.pop().unwrap();
                        room.players.notify_deal_face_up(card1).await;
                        room.players.notify_deal_face_up(card2).await;
                        room.players.current().unwrap().hand.push(card1);
                        room.players.current().unwrap().hand.push(card2);
                        let _next = room.players.next_mut().unwrap();
                    }
                    room.players.notify_next_turn().await;
                }
                Ok(PlayerAction::EndTurn) => {
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    if !room.players.is_current(|p| p.who == who) {
                        println!("{who} sent their turn out of order!");
                        continue;
                    }
                    let _next = room.players.next_mut().unwrap();
                    if room.players.current_index() == 0 {
                        println!("Game is over");
                        room.players.notify_game_end().await;
                        //TODO: Error checking on if the room still exists
                        lock.rooms.remove(&id).unwrap();
                        return;
                    }
                    let current = room.players.current().unwrap();
                    println!("It is now {}'s turn", current.who);
                    room.players.notify_next_turn().await;
                }
                Ok(PlayerAction::Deal) => {
                    println!("{who} has requested a deal");
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    if !room.players.is_current(|p| p.who == who) {
                        println!("{who} sent their turn out of order!");
                        continue;
                    }
                    let card = room.decks.pop().unwrap();
                    room.players.current().unwrap().hand.push(card);
                    room.players.notify_deal_face_up(card).await;

                    if room.players.current().unwrap().hand.len() == 10 {
                        println!("{who} has dealt the max hand");
                        room.players.notify_turn_end().await;
                    }
                }
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                let mut lock = state.lock().await;
                if let Some(room) = lock.rooms.get_mut(&id) {
                    if room.players.len() == 1 {
                        lock.rooms.remove(&id).unwrap();
                        println!("The last player left the game");
                        return;
                    }

                    let old_index = room.players.find(|p| p.who == who).unwrap();
                    let was_current = old_index == room.players.current_index();
                    let _old_connection = room.players.remove(|p| p.who == who).unwrap();

                    room.players.notify_player_left(old_index).await;

                    if was_current {
                        if room.started {
                            if old_index == room.players.len() {
                                room.players.notify_game_end().await;
                            } else {
                                room.players.notify_next_turn().await;
                            }
                        } else {
                            room.players.notify_next_host().await;
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

impl Cycler<Player> {
    async fn notify_new_player(&mut self) {
        let msg = serde_json::to_string(&ServerAction::PlayerJoin { player: self.len() }).unwrap();
        for _ in 0..self.len() {
            let next = self.next_mut().unwrap();
            next.socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }
    async fn notify_player_left(&mut self, player: usize) {
        let msg = serde_json::to_string(&ServerAction::PlayerLeave { player }).unwrap();
        for _ in 0..self.len() {
            let next = self.next_mut().unwrap();
            next.socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }
    async fn notify_next_host(&mut self) {
        let msg = serde_json::to_string(&ServerAction::NewHost).unwrap();
        let current = self.current().unwrap();
        current.socket.send(Message::Text(msg)).await.unwrap();
    }
    async fn notify_next_turn(&mut self) {
        while self.len() > 0 {
            let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
            let current = self.current().unwrap();
            if current.socket.send(Message::Text(msg)).await.is_ok() {
                break;
            } else {
                let who = current.who.clone();
                println!("Failed to send {} notification", current.who);
                let _old_conn = self.remove(|p| p.who == who).unwrap();
            }
        }
    }

    async fn notify_deal_face_down(&mut self, card: Card) {
        let current_index = self.current_index();
        let action = ServerAction::Dealt {
            player: current_index,
            card: Some(card),
        };

        let msg = serde_json::to_string(&action).unwrap();
        let current = self.current().unwrap();
        current.socket.send(Message::Text(msg)).await.unwrap();

        let action = ServerAction::Dealt {
            player: current_index,
            card: None,
        };
        let msg = serde_json::to_string(&action).unwrap();
        for _ in 1..self.len() {
            let next = self.next_mut().unwrap();
            next.socket.send(Message::Text(msg.clone())).await.unwrap();
        }
        let _current = self.next_mut().unwrap();

        assert_eq!(current_index, self.current_index());
    }

    async fn notify_deal_face_up(&mut self, card: Card) {
        let current_index = self.current_index();
        let action = ServerAction::Dealt {
            player: current_index,
            card: Some(card),
        };

        let msg = serde_json::to_string(&action).unwrap();
        for _ in 0..self.len() {
            let next = self.next_mut().unwrap();
            next.socket.send(Message::Text(msg.clone())).await.unwrap();
        }

        assert_eq!(current_index, self.current_index());
    }

    async fn notify_turn_end(&mut self) {
        let msg = serde_json::to_string(&ServerAction::EndTurn).unwrap();
        let current = self.current().unwrap();
        current.socket.send(Message::Text(msg)).await.unwrap();
    }

    async fn notify_game_end(&mut self) {
        let current_index = self.current_index();
        for _ in 0..self.len() {
            let action = ServerAction::TotalHand {
                player: self.current_index(),
                hand: self.current().unwrap().hand.clone(),
            };
            let msg = serde_json::to_string(&action).unwrap();
            for _ in 1..self.len() {
                let next = self.next_mut().unwrap();
                next.socket.send(Message::Text(msg.clone())).await.unwrap();
            }
        }
        let winning_players = self.caculate_winners();
        let winner_msg = serde_json::to_string(&ServerAction::EndGame { winner: true }).unwrap();
        let loser_msg = serde_json::to_string(&ServerAction::EndGame { winner: false }).unwrap();
        for _ in 0..self.len() {
            let _ = self.next_mut().unwrap();
            let next_index = self.current_index();
            let next = self.current().unwrap();
            if winning_players.contains(&next_index) {
                next.socket
                    .send(Message::Text(winner_msg.clone()))
                    .await
                    .unwrap();
            } else {
                next.socket
                    .send(Message::Text(loser_msg.clone()))
                    .await
                    .unwrap();
            }
        }
        assert_eq!(current_index, self.current_index());
    }

    fn caculate_winners(&mut self) -> Vec<usize> {
        let mut scores = vec![];
        for _ in 0..self.len() {
            let current_index = self.current_index();
            let current = self.current().unwrap();
            scores.push((current_index, score(&current.hand)));
        }
        scores.sort_unstable_by_key(|x| x.1);
        let max = scores.first().unwrap().1;
        scores.retain(|x| x.1 == max);
        scores.into_iter().map(|(player, _score)| player).collect()
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum ServerAction {
    PlayerJoin { player: usize },
    PlayerLeave { player: usize },
    NewHost,
    Dealt { player: usize, card: Option<Card> },
    TotalHand { player: usize, hand: Vec<Card> },
    YourTurn,
    EndTurn,
    EndGame { winner: bool },
}

struct Player {
    who: SocketAddr,
    socket: SplitSink<WebSocket, Message>,
    hand: Vec<Card>,
}

impl Player {
    pub fn new(who: SocketAddr, socket: SplitSink<WebSocket, Message>, hand: Vec<Card>) -> Player {
        Player { who, socket, hand }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Score {
    Bust,
    Points(u8),
    Blackjack,
}

impl Score {
    fn to_points(&self) -> Self {
        match self {
            Self::Blackjack => Self::Points(21),
            Self::Bust => Self::Points(0),
            Self::Points(_) => *self,
        }
    }
}

fn score(hand: &Vec<Card>) -> Score {
    let mut score = 0;
    let mut found_ace = false;
    for card in hand.iter() {
        score += match card.rank {
            card::Rank::Ace => {
                found_ace = true;
                1
            }
            card::Rank::Two => 2,
            card::Rank::Three => 3,
            card::Rank::Four => 4,
            card::Rank::Five => 5,
            card::Rank::Six => 6,
            card::Rank::Seven => 7,
            card::Rank::Eight => 8,
            card::Rank::Nine => 9,
            card::Rank::Ten | card::Rank::Jack | card::Rank::Queen | card::Rank::King => 10,
        };
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
    players: Cycler<Player>,
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

struct Cycler<T> {
    index: usize,
    inner: Vec<T>,
}

impl<T> Default for Cycler<T> {
    fn default() -> Self {
        Cycler {
            inner: Vec::new(),
            index: 0,
        }
    }
}

impl<T> Cycler<T> {
    fn add(&mut self, value: T) {
        self.inner.push(value)
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn find(&self, predicate: impl FnMut(&T) -> bool) -> Option<usize> {
        self.inner.iter().position(predicate)
    }
    fn is_current(&self, predicate: impl FnMut(&T) -> bool) -> bool {
        let index = self.find(predicate);
        let Some(index) = index else {
            return false;
        };
        index == self.index
    }
    fn current(&mut self) -> Option<&mut T> {
        self.inner.get_mut(self.index)
    }
    fn current_index(&self) -> usize {
        self.index
    }
    fn remove(&mut self, predicate: impl FnMut(&T) -> bool) -> Option<T> {
        let index = self.inner.iter().position(predicate)?;
        if index == self.len() - 1 {
            self.index = 0;
        }
        Some(self.inner.remove(index))
    }
    fn next_mut(&mut self) -> Option<&mut T> {
        self.index = (self.index + 1) % self.inner.len();
        self.inner.get_mut(self.index)
    }
}
