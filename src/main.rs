use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Extension, Form, Router,
};
use axum_extra::routing::SpaRouter;
use axum_login::{
    axum_sessions::{async_session::MemoryStore as SessionMemoryStore, SessionLayer},
    extractors::AuthContext,
    secrecy::SecretVec,
    AuthLayer, AuthUser, RequireAuthorizationLayer, SqliteStore,
};
use once_cell::sync::Lazy;
use serde::Deserialize;
use sqlx::{Pool, SqlitePool};
use tera::Tera;
use tower_http::catch_panic::CatchPanicLayer;

static TERA: Lazy<Tera> = Lazy::new(|| match Tera::new("templates/**/*") {
    Ok(t) => t,
    Err(e) => {
        eprintln!("Error parsing: {e}");
        std::process::exit(1)
    }
});

#[derive(Debug, Clone, sqlx::FromRow)]
struct User {
    id: u32,
    username: String,
    password_hash: String,
    balance: u32,
}

impl AuthUser<()> for User {
    fn get_id(&self) -> String {
        self.id.to_string()
    }

    fn get_password_hash(&self) -> SecretVec<u8> {
        SecretVec::new(self.password_hash.clone().into())
    }

    fn get_role(&self) -> Option<()> {
        None
    }
}

type Auth = AuthContext<User, SqliteStore<User>, ()>;

#[tokio::main]
async fn main() -> Result<(), impl std::error::Error> {
    //Force initialization in the beginning to ensure all templates parse before
    // opening the server to users
    Lazy::force(&TERA);

    let secret = std::array::from_fn::<u8, 64, _>(|_| fastrand::u8(0..u8::MAX));
    let session_store = SessionMemoryStore::new();
    let session_layer = SessionLayer::new(session_store, &secret);
    let connection = SqlitePool::connect("sqlite://database").await.unwrap();
    let state = connection.clone();
    let sqlite_store = SqliteStore::<User>::new(connection);
    let auth_layer = AuthLayer::new(sqlite_store, &secret);

    // sqlx::query!(
    //     "CREATE TABLE users (
    //         id int NOT NULL UNIQUE PRIMARY KEY,
    //         username varchar(255) NOT NULL UNIQUE,
    //         password varchar(255) NOT NULL,
    //         balance int NOT NULL
    //     )"
    // )
    // .execute(&state)
    // .await
    // .unwrap();

    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/gamble", get(gamble).post(recieve_gamble))
        .route_layer(RequireAuthorizationLayer::<User, ()>::login())
        .route("/", get(home))
        .route("/login", get(login_form).post(recieve_login))
        .route("/logout", get(logout))
        .with_state(state)
        .layer(auth_layer)
        .layer(session_layer)
        .merge(assets)
        .fallback(error_404)
        .layer(CatchPanicLayer::custom(|_| error_500().into_response()));

    let addr = (Ipv4Addr::LOCALHOST, 3000).into();
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn home(auth: Auth) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert(
        "username",
        &auth
            .current_user
            .map(|user| user.username)
            .unwrap_or_default(),
    );
    Html(TERA.render("index.html", &context).unwrap())
}

async fn login_form() -> impl IntoResponse {
    Html(TERA.render("login.html", &tera::Context::new()).unwrap())
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn recieve_login(
    mut auth: Auth,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    State(db): State<SqlitePool>,
    Form(request): Form<LoginRequest>,
) {
    println!("Recieved login request from {who}");
    sqlx::query_as!(
        User,
        "select * from users where username=? AND password_hash=?",
        request.username,
        request.password
    );
    let user: User = sqlx::query_as("SELECT * FROM users WHERE username=$1 AND password=$2")
        .bind(request.username)
        .bind(request.password)
        .fetch_one(&db)
        .await
        .unwrap();
    println!("Found user with balance: {}", user.balance);
}

async fn logout(mut auth: Auth, ConnectInfo(who): ConnectInfo<SocketAddr>) {
    todo!()
}

async fn gamble(Extension(user): Extension<User>) -> impl IntoResponse {
    todo!()
}

async fn recieve_gamble(Extension(user): Extension<User>) -> impl IntoResponse {
    todo!()
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
