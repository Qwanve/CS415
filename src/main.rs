use std::{
    net::{Ipv4Addr, SocketAddr},
    ops::ControlFlow,
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

#[derive(Default)]
struct MyState {
    pub senders: Cycler<(SocketAddr, SplitSink<WebSocket, Message>)>,
}

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

#[nutype(
    sanitize(trim, uppercase)
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
    let alphabet = ('A'..='Z').collect::<Vec<_>>();
    let id = nanoid!(6, &alphabet);
    RoomId::new(id).unwrap()
}

async fn create_room(
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(_state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    let id = new_id();
    println!("{who} is trying to create a new room with id {id}");
    Redirect::to(&format!("/{id}"))
}

async fn ingame(
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    id: Option<Path<RoomId>>,
    State(_state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    let Some(Path(id)) = id else {
        println!("{who} tried to join with an invalid id");
        return;
    };
    println!("{who} has joined game {id}");
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
        println!("Could not send ping to {who}!");
        return;
    };

    println!("Pinged {who}...");

    let (mut sender, mut socket) = socket.split();
    {
        let mut lock = state.lock().await;

        if lock.senders.len() == 0 {
            let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
            let Ok(_) = sender.send(Message::Text(msg)).await else {
                println!("Failed to send message to {who}");
                return;
            };
        }
        lock.senders.add((who, sender));
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
                    while state.lock().await.senders.len() != 0 {
                        let success = notify_next_player(Arc::clone(&state)).await.is_break();
                        if success {
                            break;
                        }
                    }
                }
                Ok(PlayerAction::JoinRoom(id)) => {
                    println!("{who} is trying to join room {id}");
                }
                Ok(PlayerAction::NewRoom) => {}
                Err(_) => println!("{who} sent an invalid action: {msg}"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                let was_current = {
                    let mut state = state.lock().await;
                    let was_current = state.senders.is_current(|(v, _conn)| *v == who);
                    let _old_connection = state.senders.remove(|(v, _conn)| *v == who).unwrap();
                    was_current
                };
                if was_current {
                    while state.lock().await.senders.len() != 0 {
                        let success = notify_next_player(Arc::clone(&state)).await.is_break();
                        if success {
                            break;
                        }
                    }
                }
                return;
            }
            _ => println!("Unknown message {msg:?}"),
        }
    }
}

async fn notify_next_player(state: Arc<Mutex<MyState>>) -> ControlFlow<()> {
    let msg = serde_json::to_string(&ServerAction::YourTurn).unwrap();
    let mut lock = state.lock().await;
    let (who, socket) = lock.senders.next_mut().unwrap();
    let Ok(_) = socket.send(Message::Text(msg)).await else {
        println!("Failed to send message to {who}");
        let who = who.clone();
        let _old_connection = lock.senders.remove(|(v, _)| *v == who);
        return ControlFlow::Continue(());
    };
    return ControlFlow::Break(());
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
    NewRoom,
    JoinRoom(String),
    Click,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ServerAction {
    YourTurn,
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
