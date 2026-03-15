use crate::game::types::GameSettings;
use crate::state::AppState;
use axum::{
    extract::State,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub struct CreateGameResponse {
    pub game_id: String,
}

#[derive(Deserialize)]
pub struct CreateGameRequest {
    #[serde(flatten)]
    pub settings: GameSettings,
}

pub async fn create_game_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateGameRequest>,
) -> impl IntoResponse {
    let game_id = state.create_game(payload.settings);
    tracing::info!(%game_id, "Created new game");
    Json(CreateGameResponse { game_id })
}
