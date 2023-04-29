use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    Extension,
};
use once_cell::sync::Lazy;
use tera::Tera;
use tokio::sync::Mutex;

use crate::{
    card::Card,
    data::{new_id, MyState, Room, RoomId},
    websocket, Auth, User,
};

static TERA: Lazy<Tera> = Lazy::new(|| match Tera::new("templates/**/*") {
    Ok(t) => t,
    Err(e) => {
        eprintln!("Error parsing: {e}");
        std::process::exit(1)
    }
});

pub async fn home(auth: Auth) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("is_logged_in", &auth.current_user.is_some());
    Html(TERA.render("index.html", &context).unwrap())
}

pub async fn create_room(
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<MyState>>>,
    Extension(user): Extension<User>,
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
            let room = Room::new(Card::shuffled_decks().into());
            rooms.insert(id.clone(), room);
            return Redirect::to(&format!("/{id}"));
        }
    }
    panic!("Failed to create a unique id");
}

pub async fn ingame(
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
    (
        StatusCode::OK,
        Html(TERA.render("game.html", &context).unwrap()),
    )
}

pub async fn ws_handler(
    ws: Option<WebSocketUpgrade>,
    Path(id): Path<RoomId>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(state): State<Arc<Mutex<MyState>>>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let Some(ws) = ws else {
        println!("{who} tried to load the websocket page");
        return (
            StatusCode::BAD_REQUEST,
            Html(TERA.render("400.html", &tera::Context::new()).unwrap())
        ).into_response();
    };
    ws.on_upgrade(move |socket| websocket(socket, who, id, state, user))
}

pub async fn error_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Html(TERA.render("404.html", &tera::Context::new()).unwrap()),
    )
}

pub fn error_500() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(TERA.render("500.html", &tera::Context::new()).unwrap()),
    )
}
