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
        .route("/favicon.ico", get(error_404))
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
    let id = new_id();
    println!("{who} is trying to create a new room with id {id}");
    let rooms = &mut state.lock().await.rooms;
    if rooms.contains_key(&id) {
        println!("Room {id} already exists");
        //TODO: Display error
        Redirect::to("/")
    } else {
        println!("Created room {id}");
        rooms.insert(id.clone(), Cycler::default());
        Redirect::to(&format!("/{id}"))
    }
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
    return (
        StatusCode::OK,
        Html(TERA.render("game.html", &tera::Context::new()).unwrap()),
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

        if room.len() == 0 {
            let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        room.add((who, sender));
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
                Ok(PlayerAction::Click) => {
                    let mut lock = state.lock().await;
                    let room = lock.rooms.get_mut(&id).unwrap();
                    if !room.is_current(|(v, _)| *v == who) {
                        println!("{who} sent their turn out of order!");
                        continue;
                    }
                    let next = room.next_mut().unwrap();
                    println!("It is now {}'s turn", next.0);
                    notify_player(room).await;
                }
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                let mut lock = state.lock().await;
                let room = lock.rooms.get_mut(&id).unwrap();
                let was_current = {
                    let was_current = room.is_current(|(v, _conn)| *v == who);
                    let _old_connection = room.remove(|(v, _conn)| *v == who).unwrap();
                    was_current
                };
                if was_current {
                    notify_player(room).await;
                }
                return;
            }
            _ => println!("Unknown message {msg:?}"),
        }
    }
}

async fn notify_player(room: &mut Cycler<Connection>) {
    while room.len() > 0 {
        let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
        let current = room.current().unwrap();
        if current.1.send(Message::Text(msg)).await.is_ok() {
            break;
        } else {
            let who = current.0.clone();
            println!("Failed to send {} notification", current.0);
            let _old_conn = room.remove(|(v, _)| *v == who).unwrap();
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum PlayerAction {
    Click,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ServerAction {
    YourTurn,
}

type Connection = (SocketAddr, SplitSink<WebSocket, Message>);

#[derive(Default)]
struct MyState {
    rooms: HashMap<RoomId, Cycler<Connection>>,
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
