use async_mutex::Mutex;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use axum_extra::routing::SpaRouter;
use futures::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use once_cell::sync::Lazy;
use tera::Tera;
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
    pub numbers: Vec<u8>,
    pub senders: HashMap<SocketAddr, Option<SplitSink<WebSocket, Message>>>,
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
        .route("/ws", get(ws_handler))
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

async fn ws_handler(
    ws: Option<WebSocketUpgrade>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<MyState>>>,
) -> impl IntoResponse {
    let Some(ws) = ws else {
        return (
            StatusCode::BAD_REQUEST,
            Html(TERA.render("400.html", &tera::Context::new()).unwrap())
        ).into_response();
    };
    ws.on_upgrade(move |socket| websocket(socket, who, state))
}

async fn websocket(mut socket: WebSocket, who: SocketAddr, state: Arc<Mutex<MyState>>) {
    let Ok(_) = socket.send(Message::Ping(vec![1, 2, 3, 4, 5, 6])).await else {
        println!("Could not send ping to {who}!");
        return;
    };

    println!("Pinged {who}...");

    let json_state = serde_json::to_string(&state.lock().await.numbers.clone()).unwrap();
    let Ok(_) = socket.send(Message::Text(json_state)).await else {
        println!("Failed to send state to {who}");
        return;
    };

    let (sender, mut socket) = socket.split();
    state.lock().await.senders.insert(who, Some(sender));

    loop {
        if state.lock().await.senders.get(&who).unwrap().is_none() {
            state.lock().await.senders.remove(&who).unwrap();
            return;
        }
        let Some(msg) = socket.next().await else {
            println!("Connection with {who} closed abruptly");
            return;
        };

        let Ok(msg) = msg else {
            println!("Error {} while recieving from {who}", msg.unwrap_err());
            return;
        };

        match msg {
            Message::Text(msg) => match serde_json::from_str(&msg) {
                Ok(Action::Next) => send_num(who, &state).await,
                Ok(Action::Clear) => send_clear(who, &state).await,
                Err(_) => println!("{who} sent an invalid action"),
            },
            Message::Pong(_) => println!("Recieved pong from {who}"),
            Message::Close(_) => {
                println!("{who} has closed the connection");
                return;
            }
            _ => println!("Unknown message {msg:?}"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Action {
    Next,
    Clear,
}

async fn send_num(who: SocketAddr, state: &Arc<Mutex<MyState>>) {
    println!("{who} requested the next num");
    let num = fastrand::u8(0..=100);
    state.lock().await.numbers.push(num);
    for (who, socket) in state.lock().await.senders.iter_mut() {
        let Some(socket) = socket else {
            continue;
        };
        if socket
            .send(Message::Text(serde_json::to_string(&num).unwrap()))
            .await
            .is_err()
        {
            state.lock().await.senders.get_mut(who).unwrap().take();
            println!("Failed to send next number to {who}");
        };
    }
}

async fn send_clear(who: SocketAddr, state: &Arc<Mutex<MyState>>) {
    println!("{who} requested a clear");
    state.lock().await.numbers.clear();
    for (who, socket) in state.lock().await.senders.iter_mut() {
        let Some(socket) = socket else {
            continue;
        };
        if socket
            .send(Message::Text(
                serde_json::to_string(&Action::Clear).unwrap(),
            ))
            .await
            .is_err()
        {
            state.lock().await.senders.get_mut(who).take();
            println!("Failed to send 'clear' command to {who}");
        }
    }
}
