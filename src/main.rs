use parking_lot::Mutex;
use std::{net::SocketAddr, sync::Arc};

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

static TERA: Lazy<Tera> = Lazy::new(|| match Tera::new("templates/**/*") {
    Ok(t) => t,
    Err(e) => {
        eprintln!("Error parsing: {e}");
        std::process::exit(1)
    }
});

#[tokio::main]
async fn main() -> Result<(), impl std::error::Error> {
    let deck = Arc::new(Mutex::new(new_deck()));
    fastrand::shuffle(&mut deck.lock());
    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/", get(home))
        .route("/ws", get(ws_handler))
        .with_state(deck)
        .merge(assets)
        .fallback(error_404);

    let addr = ([0; 4], 3000).into();
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn home() -> impl IntoResponse {
    Html(TERA.render("index.html", &tera::Context::new()).unwrap())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<Vec<Card>>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| websocket(socket, who, state))
}

async fn websocket(mut socket: WebSocket, who: SocketAddr, deck: Arc<Mutex<Vec<Card>>>) {
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
                    let card = deal(Arc::clone(&deck));
                    let msg = serde_json::to_string(&card).expect("Failed to serialize card");
                    let Ok(_) = socket.send(Message::Text(msg)).await else {
                        println!("Failed to send {who} the next card");
                        return;
                    };
                }
                Ok(Action::Shuffle) => shuffle(Arc::clone(&deck)),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Action {
    Deal,
    Shuffle,
}

fn deal(deck: Arc<Mutex<Vec<Card>>>) -> Option<Card> {
    let card = deck.lock().pop();
    card
}

fn shuffle(state: Arc<Mutex<Vec<Card>>>) {
    let mut deck = new_deck();
    fastrand::shuffle(&mut deck);
    *state.lock() = deck;
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
