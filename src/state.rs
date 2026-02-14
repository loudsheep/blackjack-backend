use crate::{
    game::actor::GameActor,
    messages::{ClientMessage, ServerMessage},
};
use std::{collections::HashMap, sync::Mutex};
use rand::RngExt;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

#[derive(Clone)]
pub struct GameHandle {
    pub sender: mpsc::Sender<(Uuid, ClientMessage)>,
    pub state_sender: broadcast::Sender<ServerMessage>,
}

pub struct AppState {
    pub games: Mutex<HashMap<String, GameHandle>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            games: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_game_handle(&self, game_id: &str) -> GameHandle {
        let mut games = self.games.lock().unwrap();

        if let Some(handle) = games.get(game_id) {
            return handle.clone();
        }

        let (tx, rx) = mpsc::channel(100);
        let (tx_state, _) = broadcast::channel(100);

        let mut actor = GameActor::new(game_id.to_string(), rx, tx_state.clone());

        let game_id_owned = game_id.to_string(); 
        tokio::spawn(async move {
            actor.run().await;
            tracing::info!("Game {} ended", game_id_owned);
        });

        let handle = GameHandle {
            sender: tx,
            state_sender: tx_state,
        };

        games.insert(game_id.to_string(), handle.clone());
        handle
    }

    pub fn generate_game_id(&self) -> String {
        let mut rng = rand::rng();
        loop {
            let id = format!("{:06}", rng.random_range(0..999999)); // rng.gen_range or random_range
            if !self.games.lock().unwrap().contains_key(&id) {
                return id;
            }
        }
    }

    pub async fn get_game_sender(&self, game_id: &str) -> mpsc::Sender<(Uuid, ClientMessage)> {
        self.get_game_handle(game_id).await.sender
    }

    pub async fn subscribe_to_game(&self, game_id: &str) -> broadcast::Receiver<ServerMessage> {
        self.get_game_handle(game_id).await.state_sender.subscribe()
    }
}
