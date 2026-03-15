use crate::messages::ClientMessage;
use crate::state::AppState;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use std::sync::Arc;

#[derive(serde::Deserialize)]
pub struct ConnectParams {
    pub player_id: Option<uuid::Uuid>,
    pub secret: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(game_id): Path<String>,
    Query(params): Query<ConnectParams>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let handle = match state.get_game_handle(&game_id).await {
        Some(h) => h,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let current_players = handle
        .player_count
        .load(std::sync::atomic::Ordering::Relaxed);

    if params.player_id.is_none() && current_players >= handle.settings.max_players {
        return StatusCode::FORBIDDEN.into_response(); // 403 if full
    }

    let connection_id = uuid::Uuid::new_v4();

    tracing::debug!(
        %connection_id,
        %game_id,
        "New WebSocket connection",
    );

    ws.on_upgrade(move |socket| handle_socket(socket, game_id, connection_id, state, params))
}

async fn handle_socket(
    socket: WebSocket,
    game_id: String,
    connection_id: uuid::Uuid,
    state: Arc<AppState>,
    params: ConnectParams,
) {
    let (mut sender, mut receiver) = socket.split();

    let tx = if let Some(tx) = state.get_game_sender(&game_id).await {
        tx
    } else {
        tracing::warn!(%game_id, "Game not found during socket setup");
        return;
    };

    // If reconnect params are present, send Reconnect message immediately
    if let (Some(pid), Some(secret)) = (params.player_id, params.secret) {
        if tx
            .send((
                connection_id,
                ClientMessage::Reconnect {
                    player_id: pid,
                    secret,
                },
            ))
            .await
            .is_err()
        {
            return;
        }
    }

    let mut rx = if let Some(rx) = state.subscribe_to_game(&game_id).await {
        rx
    } else {
        return;
    };

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Some(target) = msg.target {
                if target != connection_id {
                    continue;
                }
            }

            let json = serde_json::to_string(&msg.message).unwrap();
            if sender.send(Message::Text(json.into())).await.is_err() {
                break;
            }

            if let crate::messages::ServerMessage::Kicked = msg.message {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                if tx.send((connection_id, client_msg)).await.is_err() {
                    break;
                }
            }
        }
    }

    let _ = tx.send((connection_id, ClientMessage::Disconnect)).await;
    send_task.abort();
}
