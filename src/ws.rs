use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{State, Path},
    response::IntoResponse,
};
use crate::state::AppState;
use crate::messages::ClientMessage;
use std::sync::Arc;
use futures::{sink::SinkExt, stream::StreamExt};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(game_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let player_id = uuid::Uuid::new_v4();

    tracing::info!("New WebSocket connection: player_id={}, game_id={}", player_id, game_id);

    ws.on_upgrade(move |socket| handle_socket(socket, game_id, player_id, state))
}

async fn handle_socket(socket: WebSocket, game_id: String, player_id: uuid::Uuid, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    let tx = state.get_game_sender(&game_id).await;
    let mut rx = state.subscribe_to_game(&game_id).await;

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            // Fix: Convert String to Utf8Bytes using .into()
            if sender.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                // This now matches the type expected by state.rs
                if tx.send((player_id, client_msg)).await.is_err() {
                    break;
                }
            }
        }
    }

    send_task.abort();
}
