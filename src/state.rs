use crate::{
    game::{actor::GameActor, types::GameSettings},
    messages::{ClientMessage, BroadcastMessage},
};
use rand::RngExt;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

#[derive(Clone)]
pub struct GameHandle {
    pub sender: mpsc::Sender<(Uuid, ClientMessage)>,
    pub state_sender: broadcast::Sender<BroadcastMessage>,
    pub settings: GameSettings,
    pub player_count: std::sync::Arc<std::sync::atomic::AtomicUsize>
}

pub struct AppState {
    pub games: Mutex<HashMap<String, GameHandle>>,
    cleanup_sender: mpsc::Sender<String>,
}

impl AppState {
    pub fn new(cleanup_sender: mpsc::Sender<String>) -> Self {
        AppState {
            games: Mutex::new(HashMap::new()),
            cleanup_sender,
        }
    }

    pub fn remove_game(&self, game_id: &str) {
        let mut games = self.games.lock().unwrap();
        if games.remove(game_id).is_some() {
            tracing::info!("Game {} removed from AppState", game_id);
        }
    }

    pub async fn get_game_handle(&self, game_id: &str) -> Option<GameHandle> {
        let games = self.games.lock().unwrap();
        games.get(game_id).cloned()
    }

    pub fn create_game(&self, settings: GameSettings) -> String {
        let mut games = self.games.lock().unwrap();
        let mut rng = rand::rng();
        let id = loop {
            let id = format!("{:06}", rng.random_range(0..999999));
            if !games.contains_key(&id) {
                break id;
            }
        };

        let (tx, rx) = mpsc::channel(100);
        let (tx_state, _) = broadcast::channel(100);

        let player_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut actor = GameActor::new(
            id.clone(),
            settings.clone(),
            rx,
            tx_state.clone(),
            player_count.clone(),
            self.cleanup_sender.clone(),
        );

        let game_id_owned = id.clone();
        tokio::spawn(async move {
            actor.run().await;
            tracing::info!("Game {} ended", game_id_owned);
        });

        let handle = GameHandle {
            sender: tx,
            state_sender: tx_state,
            settings: settings,
            player_count: player_count,
        };

        games.insert(id.clone(), handle);
        id
    }

    pub async fn get_game_sender(
        &self,
        game_id: &str,
    ) -> Option<mpsc::Sender<(Uuid, ClientMessage)>> {
        self.get_game_handle(game_id).await.map(|h| h.sender)
    }

    pub async fn subscribe_to_game(
        &self,
        game_id: &str,
    ) -> Option<broadcast::Receiver<BroadcastMessage>> {
        self.get_game_handle(game_id)
            .await
            .map(|h| h.state_sender.subscribe())
    }
}
