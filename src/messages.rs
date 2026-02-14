use crate::game::types::{Card, GamePhase, GameSettings, Player};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize, Debug)]
#[serde(tag = "action", content = "payload")]
pub enum ClientMessage {
    // Lobby Actions
    JoinGame { username: String },
    Reconnect { player_id: Uuid, secret: String },

    // Admin Actions
    StartGame, // Transitions Lobby -> Betting
    ApprovePlayer { player_id: Uuid },
    KickPlayer { player_id: Uuid },
    UpdateSettings { settings: GameSettings }, // Mid-game change
    AdminUpdateBalance { target_id: Uuid, change_chips: i32 }, // Admin cheat/fix
    NextRound,                                 // Transitions Payout -> Betting

    // Player Actions
    PlaceBet { amount: u32 },
    GameAction { action_type: ActionType }, // Hit, Stand, Double, Split
    Chat { message: String },
    Ping,

    // Internal/System
    #[serde(skip_deserializing)]
    Disconnect,
}

#[derive(Deserialize, Debug)]
pub enum ActionType {
    Hit,
    Stand,
    Double,
    Split,
}

#[derive(Clone, Debug)]
pub struct BroadcastMessage {
    pub target: Option<Uuid>, // If None, Broadcast. If Some, Unicast to Connection ID.
    pub message: ServerMessage,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "event", content = "data")]
pub enum ServerMessage {
    Error {
        msg: String,
    },
    JoinedLobby {
        game_id: String,
        your_id: Uuid,
        secret: String,
        is_admin: bool,
    },

    GameStateSnapshot {
        phase: GamePhase,
        dealer_hand: Vec<Card>,
        players: Vec<Player>,
        deck_remaining: usize,
        current_turn_player_id: Option<Uuid>,
        settings: GameSettings,
    },

    ChatBroadcast {
        from: String,
        msg: String,
    },
    Pong,
    PlayerRequest {
        id: Uuid,
        name: String,
    }, // Sent to admin only
}
