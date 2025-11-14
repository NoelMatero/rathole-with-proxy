mod cli;
mod client;
mod config;
mod config_watcher;
mod constants;
mod helper;
mod server;
mod transport;

pub mod error;
pub mod multi_map;
pub mod protocol;
pub mod proxy;

pub use cli::Cli;
pub use client::run_client;
pub use config::Config;
pub use error::AppError;
pub use server::run_server;

use crate::config_watcher::ConfigChange;
use anyhow::Result;
use tokio::sync::{broadcast, mpsc};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Clone)]
pub struct RedisManager {
    pub conn: redis::aio::MultiplexedConnection,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, "Invalid credentials".to_string())
            }
            AppError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token".to_string()),
            AppError::TunnelNotFound => (StatusCode::NOT_FOUND, "Tunnel not found".to_string()),
            AppError::TunnelSendError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to send request to tunnel".to_string(),
            ),
            AppError::TunnelResponseError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get response from tunnel".to_string(),
            ),
            AppError::WebSocketError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("WebSocket error: {}", e),
            ),
            AppError::JsonError(e) => (StatusCode::BAD_REQUEST, format!("JSON error: {}", e)),
            AppError::HyperError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Hyper error: {}", e),
            ),
            AppError::ReqwestError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Reqwest error: {}", e),
            ),
            AppError::JwtError(e) => (StatusCode::UNAUTHORIZED, format!("JWT error: {}", e)),
            AppError::IoError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {}", e),
            ),
            AppError::Other(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Other error: {}", e),
            ),
        };

        (status, error_message).into_response()
    }
}

pub async fn run(
    args: Cli,
    mut shutdown_rx: broadcast::Receiver<bool>,
    shutdown_tx: broadcast::Sender<bool>,
    update_rx: &mut mpsc::Receiver<ConfigChange>,
) -> Result<()> {
    //let (update_tx, update_rx) = mpsc::channel(1);
    let watcher: Option<config_watcher::ConfigWatcherHandle> = if let Some(path) = &args.config_path
    {
        Some(config_watcher::ConfigWatcherHandle::new(path, shutdown_rx.resubscribe()).await?)
    } else {
        None
    };

    let config = Config::from_file(
        args.config_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("config is not specified"))?,
    )
    .await?;

    if config.server.is_some() {
        run_server(config, &mut shutdown_rx, update_rx).await?;
    } else if config.client.is_some() {
        loop {
            let mut client_shutdown_rx = shutdown_rx.resubscribe();

            tokio::select! {
                _ = run_client(config.clone(), &mut client_shutdown_rx, update_rx) => {},
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    }

    if watcher.is_some() {
        let _ = shutdown_tx.send(true); // tell watcher task to exit
    }

    Ok(())
}
