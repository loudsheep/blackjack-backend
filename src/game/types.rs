use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum GamePhase {
    Lobby,
    Betting,
    Playing,
    DealerTurn,
    Payout,
    GameOver,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Player {
    pub id: Uuid,
    pub name: String,
    pub chips: u32,
    pub hands: Vec<Hand>,
    pub active_hand_index: usize,
    pub status: PlayerStatus,
    pub is_admin: bool,
    #[serde(skip)]
    pub secret: String,
    pub is_connected: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Hand {
    pub cards: Vec<Card>,
    pub bet: u32,
    pub status: HandStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum HandStatus {
    Playing,
    Stood,
    Busted,
    Blackjack,
    Doubled,
    Won,
    Lost,
    Push,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum PlayerStatus {
    Spectating,      // Just joined or sitting out
    Sitting,         // Waiting for round
    Playing,         // Currently in a round
    PendingApproval, // Waiting for admin to let them in
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GameSettings {
    pub initial_chips: u32,
    pub max_players: usize,
    pub deck_count: usize,
    pub approval_required: bool,
    pub chat_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum Suit {
    Hearts,
    Diamonds,
    Clubs,
    Spades,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum Rank {
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
    Ace,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Card {
    pub suit: Suit,
    pub rank: Rank,
}

impl Card {
    pub fn value(&self) -> u8 {
        match self.rank {
            Rank::Two => 2,
            Rank::Three => 3,
            Rank::Four => 4,
            Rank::Five => 5,
            Rank::Six => 6,
            Rank::Seven => 7,
            Rank::Eight => 8,
            Rank::Nine => 9,
            Rank::Ten | Rank::Jack | Rank::Queen | Rank::King => 10,
            Rank::Ace => 11, // ace can be 1 or 11, but we'll handle that in the game logic
        }
    }

    pub fn new_deck() -> Vec<Card> {
        let mut deck = Vec::new();
        for suit in [Suit::Hearts, Suit::Diamonds, Suit::Clubs, Suit::Spades] {
            for rank in [
                Rank::Two,
                Rank::Three,
                Rank::Four,
                Rank::Five,
                Rank::Six,
                Rank::Seven,
                Rank::Eight,
                Rank::Nine,
                Rank::Ten,
                Rank::Jack,
                Rank::Queen,
                Rank::King,
                Rank::Ace,
            ] {
                deck.push(Card {
                    suit: suit.clone(),
                    rank: rank.clone(),
                });
            }
        }
        deck
    }
}

pub fn calculate_hand_value(hand: &[Card]) -> u8 {
    let mut score = 0;
    let mut aces = 0;

    for card in hand {
        let value = card.value();
        score += value;
        if card.rank == Rank::Ace {
            aces += 1;
        }
    }

    while score > 21 && aces > 0 {
        score -= 10;
        aces -= 1;
    }

    score
}
