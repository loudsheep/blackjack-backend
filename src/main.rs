mod game;
mod messages;
mod state;
mod ws;

use axum::{
    Router,
    routing::{get, post},
};
use game::handlers::create_game_handler;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "blackjack_backend=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let (cleanup_tx, mut cleanup_rx) = tokio::sync::mpsc::channel(100);
    let shared_state = std::sync::Arc::new(state::AppState::new(cleanup_tx));

    // Cleanup task
    let state_weak = std::sync::Arc::downgrade(&shared_state);
    tokio::spawn(async move {
        while let Some(game_id) = cleanup_rx.recv().await {
            if let Some(state) = state_weak.upgrade() {
                state.remove_game(&game_id);
            } else {
                break;
            }
        }
    });

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/game/create", post(create_game_handler))
        .route("/ws/{game_id}", get(ws::ws_handler))
        .with_state(shared_state)
        .layer(CorsLayer::permissive());
    let addr = std::env::var("APP_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    tracing::info!(%addr, "Starting server");
    axum::serve(listener, app).await.unwrap();
}
