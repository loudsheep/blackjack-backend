mod game;
mod ws;
mod messages;
mod state;

use axum::{Router, routing::get};
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

    let shared_state = std::sync::Arc::new(state::AppState::new());

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/ws/{game_id}", get(ws::ws_handler))
        .with_state(shared_state);

    // get address and port from environment variables or use defaults
    let addr = std::env::var("APP_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    
    tracing::info!("Starting server on {}", addr);
    axum::serve(listener, app).await.unwrap();
}
