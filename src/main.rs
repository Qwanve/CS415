use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{Response, StatusCode},
    response::{Html, IntoResponse, Redirect},
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
use sqlx::SqlitePool;
use tera::Tera;
use tower::builder::ServiceBuilder;
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
    id: i64,
    username: String,
    password_hash: String,
    balance: i64,
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

    let assets = SpaRouter::new("/static", "static");
    let app = Router::new()
        .route("/gamble", get(gamble).post(recieve_gamble))
        .route("/logout", get(logout).post(logout))
        .route("/modify", get(modify).post(recieve_modify))
        .route_layer(RequireAuthorizationLayer::<User, ()>::login())
        .route("/", get(home))
        .route("/login", get(login_form).post(recieve_login))
        .with_state(state)
        .merge(assets)
        .fallback(error_404)
        .layer(
            //Redirect to login if unauthorized
            ServiceBuilder::new()
                .layer(session_layer)
                .layer(auth_layer)
                .map_response(|r: Response<_>| {
                    if r.status() == StatusCode::UNAUTHORIZED {
                        Redirect::to("/login").into_response()
                    } else {
                        r
                    }
                }),
        )
        // .layer(auth_layer)
        // .layer(session_layer)
        .layer(CatchPanicLayer::custom(|_| error_500().into_response()));

    let addr = (Ipv4Addr::LOCALHOST, 3000).into();
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

async fn home(auth: Auth) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("is_logged_in", &auth.current_user.is_some());
    context.insert(
        "username",
        &auth
            .current_user
            .map(|user| user.username)
            .unwrap_or_default(),
    );
    Html(TERA.render("index.html", &context).unwrap())
}

#[derive(Deserialize)]
struct Failed {
    failed: bool,
}

async fn login_form(failed: Option<Query<Failed>>, auth: Auth) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("is_logged_in", &false);

    if let Some(Query(failed)) = failed {
        context.insert("failed", &failed.failed);
    }
    Html(TERA.render("login.html", &context).unwrap()).into_response()
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
) -> Redirect {
    println!("Recieved login request from {who}");

    let user = sqlx::query_as!(
        User,
        "select * from users where username=? AND password_hash=?",
        request.username,
        request.password
    )
    .fetch_optional(&db)
    .await
    .unwrap();

    match user {
        Some(user) => {
            println!("Loggin in {who} as {}", user.username);
            auth.login(&user).await.unwrap();
            return Redirect::to("/gamble");
        }
        None => {
            println!("User not found");
            return Redirect::to("/login?failed=true");
        }
    }
}

async fn logout(mut auth: Auth, ConnectInfo(who): ConnectInfo<SocketAddr>) -> Redirect {
    println!("Recieved logout request from {who}");
    auth.logout().await;
    Redirect::to("/")
}

async fn gamble(Extension(user): Extension<User>) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("balance", &user.balance);
    context.insert("is_logged_in", &true);
    Html(TERA.render("gamble.html", &context).unwrap())
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum Coin {
    Heads,
    Tails,
}

#[derive(Debug, Deserialize)]
struct Bet {
    choice: Coin,
    amount: u32,
}

async fn recieve_gamble(
    Extension(mut user): Extension<User>,
    Form(bet): Form<Bet>,
) -> impl IntoResponse {
    println!("Recieved bet of {:?}, amount: {}", bet.choice, bet.amount);
    let res = match fastrand::bool() {
        true => Coin::Heads,
        false => Coin::Tails,
    };
    if bet.choice == res {
        println!("{} won", user.username);
        let new_value = user.balance + i64::from(bet.amount);
        sqlx::query!(
            "UPDATE users
             SET balance = ?
             WHERE id = ?",
            new_value,
            user.id
        );
        user.balance += i64::from(bet.amount);
    } else {
        println!("{} lost", user.username);
        user.balance -= i64::from(bet.amount);
    }
}

async fn modify(Extension(user): Extension<User>) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("is_logged_in", &true);
    context.insert("balance", &user.balance);
    Html(TERA.render("modify.html", &context).unwrap())
}

#[derive(Deserialize, Debug, Clone, Copy)]
struct Modify {
    value: u32,
}

async fn recieve_modify(
    Extension(user): Extension<User>,
    State(db): State<SqlitePool>,
    Form(request): Form<Modify>,
) -> impl IntoResponse {
    sqlx::query!(
        "UPDATE users
         SET balance = ?
         WHERE id = ?",
        request.value,
        user.id
    )
    .execute(&db)
    .await
    .unwrap();
    return Redirect::to("/gamble");
}

async fn error_404(auth: Auth) -> impl IntoResponse {
    let mut context = tera::Context::new();
    context.insert("is_logged_in", &auth.current_user.is_some());
    (
        StatusCode::NOT_FOUND,
        Html(TERA.render("404.html", &context).unwrap()),
    )
}

fn error_500() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(TERA.render("500.html", &tera::Context::new()).unwrap()),
    )
}
