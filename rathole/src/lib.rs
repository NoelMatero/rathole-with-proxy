mod cli;
mod client;
mod config;
mod config_watcher;
pub mod constants;
mod helper;
mod server;
mod transport;

pub mod error;
pub mod hardware;
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

use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};

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

    if args.server {
        run_server(config, &mut shutdown_rx, update_rx).await?;
    } else if args.client {
        run_client(config, &mut shutdown_rx, update_rx).await?;
    }

    if watcher.is_some() {
        let _ = shutdown_tx.send(true); // tell watcher task to exit
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<proxy::AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.username == "test" && payload.password == "test" {
        let my_claims = Claims {
            sub: "test".to_owned(),
            exp: (Utc::now() + Duration::days(1)).timestamp() as usize,
        };
        let token = encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
        )
        .map_err(AppError::JwtError)?;
        Ok((StatusCode::OK, token))
    } else {
        Err(AppError::InvalidCredentials)
    }
}

