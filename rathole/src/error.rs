
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Invalid token")]
    InvalidToken,
    #[error("Tunnel not found")]
    TunnelNotFound,
    #[error("Failed to send request to tunnel")]
    TunnelSendError,
    #[error("Failed to get response from tunnel")]
    TunnelResponseError,
    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] axum::Error),
    #[error("JSON serialization/deserialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error),
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}
