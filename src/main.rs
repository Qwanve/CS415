use parking_lot::Mutex;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
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
        .route("/deal", post(deal))
        .route("/shuffle", post(shuffle))
        .with_state(deck)
        .merge(assets)
        .fallback(error_404);

    axum::Server::bind(&([0; 4], 3000).into())
        .serve(app.into_make_service())
        .await
}

async fn home() -> impl IntoResponse {
    Html(TERA.render("index.html", &tera::Context::new()).unwrap())
}

async fn error_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Html(TERA.render("404.html", &tera::Context::new()).unwrap()),
    )
}

async fn deal(State(deck): State<Arc<Mutex<Vec<Card>>>>) -> Json<Option<Card>> {
    let card = deck.lock().pop();
    Json(card)
}

async fn shuffle(State(state): State<Arc<Mutex<Vec<Card>>>>) -> impl IntoResponse {
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
