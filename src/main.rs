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
    if state.lock().await.rooms.contains_key(&id) {
        println!("{who} has joined game {id}");
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
            let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        room.players.add(Player::new(who, sender, Vec::new()));
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
                Ok(PlayerAction::EndTurn) => {
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    if !room.players.is_current(|p| p.who == who) {
                        println!("{who} sent their turn out of order!");
                        continue;
                    }
                    let next = room.players.next_mut().unwrap();
                    println!("It is now {}'s turn", next.who);
                    notify_player_turn(&mut room.players).await;
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
                    notify_players_dealt(&mut room.players, card).await;
                }
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                let mut lock = state.lock().await;
                let room = lock.rooms.get_mut(&id).unwrap();
                let was_current = {
                    let was_current = room.players.is_current(|p| p.who == who);
                    let _old_connection = room.players.remove(|p| p.who == who).unwrap();
                    was_current
                };
                if was_current {
                    notify_player_turn(&mut room.players).await;
                }
                return;
            }
            _ => println!("Unknown message {msg:?}"),
        }
    }
}

async fn notify_player_turn(room: &mut Cycler<Player>) {
    while room.len() > 0 {
        let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
        let current = room.current().unwrap();
        if current.socket.send(Message::Text(msg)).await.is_ok() {
            break;
        } else {
            let who = current.who.clone();
            println!("Failed to send {} notification", current.who);
            let _old_conn = room.remove(|p| p.who == who).unwrap();
        }
    }
}

async fn notify_players_dealt(room: &mut Cycler<Player>, card: Card) {
    let action = ServerAction::Dealt {
        player: room.current_index(),
        card,
    };
    let current = room.current_index();
    let msg = serde_json::to_string(&action).unwrap();
    for _ in 0..room.len() {
        let next = room.next_mut().unwrap();
        next.socket.send(Message::Text(msg.clone())).await.unwrap();
    }

    assert_eq!(current, room.current_index());
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
    Deal,
    EndTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum ServerAction {
    Dealt { player: usize, card: Card },
    YourTurn,
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

struct Room {
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
    fn is_current(&self, predicate: impl FnMut(&T) -> bool) -> bool {
        let index = self.inner.iter().position(predicate);
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
