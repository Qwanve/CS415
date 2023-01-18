use parking_lot::Mutex;
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
use enum_iterator::{all, Sequence};
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

#[tokio::main]
async fn main() -> Result<(), impl std::error::Error> {
    //Force initialization in the beginning to ensure all templates parse before
    // opening the server to users
    Lazy::force(&TERA);
    let decks = Arc::new(Mutex::new(HashMap::new()));
    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/", get(home))
        .route("/ws", get(ws_handler))
        .with_state(decks)
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
    State(state): State<Arc<Mutex<HashMap<SocketAddr, Vec<Card>>>>>,
) -> impl IntoResponse {
    let Some(ws) = ws else {
        return (
            StatusCode::BAD_REQUEST,
            Html(TERA.render("400.html", &tera::Context::new()).unwrap())
        ).into_response();
    };
    ws.on_upgrade(move |socket| websocket(socket, who, state))
}

async fn websocket(
    mut socket: WebSocket,
    who: SocketAddr,
    deck: Arc<Mutex<HashMap<SocketAddr, Vec<Card>>>>,
) {
    let Ok(_) = socket.send(Message::Ping(vec![1, 2, 3, 4, 5, 6])).await else {
        println!("Could not send ping to {who}!");
        return;
    };

    println!("Pinged {who}...");

    loop {
        let Some(msg) = socket.recv().await else {
            println!("Connection with {who} closed abruptly");
            return;
        };

        let Ok(msg) = msg else {
            println!("Error {} while recieving from {who}", msg.unwrap_err());
            return;
        };

        match msg {
            Message::Text(msg) => match serde_json::from_str(&msg) {
                Ok(Action::Deal) => {
                    let card = deal(who, Arc::clone(&deck));
                    let msg = serde_json::to_string(&card).expect("Failed to serialize card");
                    let Ok(_) = socket.send(Message::Text(msg)).await else {
                        println!("Failed to send {who} the next card");
                        return;
                    };
                }
                Ok(Action::Shuffle) => shuffle(who, Arc::clone(&deck)),
                Err(_) => println!("Client {who} sent invalid data"),
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
    Deal,
    Shuffle,
}

fn deal(who: SocketAddr, decks: Arc<Mutex<HashMap<SocketAddr, Vec<Card>>>>) -> Option<Card> {
    let card = decks
        .lock()
        .entry(who)
        .or_insert_with(new_shuffled_deck)
        .pop();
    card
}

fn shuffle(who: SocketAddr, decks: Arc<Mutex<HashMap<SocketAddr, Vec<Card>>>>) {
    decks.lock().insert(who, new_shuffled_deck());
}

fn new_shuffled_deck() -> Vec<Card> {
    let mut deck = new_deck();
    fastrand::shuffle(&mut deck);
    deck
}

fn new_deck() -> Vec<Card> {
    all::<Card>().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Sequence)]
enum Rank {
    Ace,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Sequence)]
enum Suit {
    Diamonds,
    Hearts,
    Spades,
    Clubs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Sequence)]
struct Card {
    rank: Rank,
    suit: Suit,
}
