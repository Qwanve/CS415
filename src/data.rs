use sqlx::SqlitePool;
use std::{collections::HashMap, net::SocketAddr};

use axum::extract::ws::Message;
use futures::SinkExt;
use nanoid::nanoid;
use nutype::nutype;

use crate::{
    card::{Card, Rank},
    ServerAction, Socket, Who,
};

pub struct Sockets(pub HashMap<Who, Socket>);

impl Sockets {
    pub async fn notify(&mut self, action: &ServerAction) {
        let msg = serde_json::to_string(action).unwrap();
        for socket in self.0.values_mut() {
            socket.send(Message::Text(msg.clone())).await.unwrap();
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn insert(&mut self, who: Who, socket: Socket) -> Option<Socket> {
        self.0.insert(who, socket)
    }

    pub fn get_mut(&mut self, who: &Who) -> Option<&mut Socket> {
        self.0.get_mut(who)
    }

    pub fn remove(&mut self, who: &Who) -> Option<Socket> {
        self.0.remove(who)
    }
}

pub struct Room {
    pub started: bool,
    current_hand: usize,
    pub dealer_hand: Vec<Card>,
    pub hands: Vec<Hand>,
    pub sockets: Sockets,
    pub decks: Vec<Card>,
}

impl Room {
    pub fn new(decks: Vec<Card>) -> Self {
        Room {
            started: false,
            current_hand: 0,
            dealer_hand: vec![],
            hands: vec![],
            sockets: Sockets(HashMap::new()),
            decks,
        }
    }
    pub fn current_mut(&mut self) -> &mut Hand {
        self.hands.get_mut(self.current_hand).unwrap()
    }

    pub fn current(&self) -> &Hand {
        self.hands.get(self.current_hand).unwrap()
    }

    pub fn current_hand(&self) -> usize {
        self.current_hand
    }

    pub fn next_hand(&mut self) -> bool {
        self.current_hand += 1;
        self.current_hand == self.hands.len()
    }

    pub fn find_first_hand(&self, second: &Hand) -> usize {
        if !second.is_second() {
            return self.hands.iter().position(|p| p == second).unwrap();
        }
        self.hands
            .iter()
            .enumerate()
            .find(|(_, hand)| !hand.second_hand && hand.who == second.who)
            .unwrap()
            .0
    }

    pub async fn notify_current(&mut self, action: &ServerAction) {
        let who = *self.current().who();
        let socket = self.sockets.get_mut(&who).unwrap();
        let msg = serde_json::to_string(action).unwrap();
        socket.send(Message::Text(msg)).await.unwrap();
    }

    pub async fn notify_all(&mut self, action: &ServerAction) {
        self.sockets.notify(action).await
    }

    pub fn dealer_hand_dummy(&self) -> Hand {
        Hand {
            second_hand: false,
            //TODO: This is hacky as hell
            who: self.hands.first().unwrap().who,
            hand: self.dealer_hand.clone(),
            account_id: 0,
        }
    }
}

pub struct MyState {
    pub rooms: HashMap<RoomId, Room>,
    pub database: SqlitePool,
}

impl MyState {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            database: pool,
            rooms: HashMap::new(),
        }
    }
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
pub struct RoomId(String);

impl std::fmt::Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.clone().into_inner())
    }
}

pub fn new_id() -> RoomId {
    let alphabet = ('a'..='z').collect::<Vec<_>>();
    let id = nanoid!(6, &alphabet);
    RoomId::new(id).unwrap()
}

#[derive(PartialEq, Eq)]
pub struct Hand {
    second_hand: bool,
    who: SocketAddr,
    pub hand: Vec<Card>,
    account_id: i64,
}

impl Hand {
    pub fn new(who: SocketAddr, hand: Vec<Card>, second_hand: bool, account_id: i64) -> Hand {
        Hand {
            who,
            hand,
            second_hand,
            account_id,
        }
    }
    pub fn score(&self) -> Score {
        let mut score = 0;
        let mut found_ace = false;
        for card in self.hand.iter() {
            if card.rank == Rank::Ace {
                found_ace = true;
            }
            score += card.score_card();
        }

        if found_ace && score < 12 {
            score += 10;
        }

        match score.cmp(&21) {
            std::cmp::Ordering::Equal => Score::Blackjack,
            std::cmp::Ordering::Less => Score::Points(score),
            std::cmp::Ordering::Greater => Score::Bust,
        }
    }
    pub fn who(&self) -> &SocketAddr {
        &self.who
    }

    pub fn is_second(&self) -> bool {
        self.second_hand
    }

    pub fn account_id(&self) -> i64 {
        self.account_id
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Score {
    Bust,
    Points(u8),
    Blackjack,
}

impl Score {
    pub fn is_blackjack(&self) -> bool {
        *self == Score::Blackjack
    }

    pub fn is_bust(&self) -> bool {
        *self == Score::Bust
    }
}
