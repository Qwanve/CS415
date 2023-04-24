use enum_iterator::{all, Sequence};
use serde::Serialize;

#[derive(Sequence, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Suit {
    Hearts,
    Diamonds,
    Clubs,
    Spades,
}

#[derive(Sequence, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rank {
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

#[derive(Serialize, Sequence, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Card {
    pub suit: Suit,
    pub rank: Rank,
}

impl Card {
    pub fn one_deck() -> [Card; 52] {
        all::<Card>().collect::<Vec<_>>().try_into().unwrap()
    }

    pub fn decks() -> [Card; 52 * 8] {
        [Card::one_deck(); 8].concat().try_into().unwrap()
    }

    pub fn shuffled_decks() -> [Card; 52 * 8] {
        let mut decks = Card::decks();
        fastrand::shuffle(&mut decks);
        decks
    }
    pub fn score_card(&self) -> u8 {
        match self.rank {
            Rank::Ace => 1,
            Rank::Two => 2,
            Rank::Three => 3,
            Rank::Four => 4,
            Rank::Five => 5,
            Rank::Six => 6,
            Rank::Seven => 7,
            Rank::Eight => 8,
            Rank::Nine => 9,
            Rank::Ten | Rank::Jack | Rank::Queen | Rank::King => 10,
        }
    }
}
