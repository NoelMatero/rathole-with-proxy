use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use http_body_util::Full;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use rathole::proxy::AppState;
use rathole::{login, Claims, LoginPayload, RedisManager};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower::ServiceExt; // for `oneshot` testing

// Helper to create a mock AppState
async fn mock_app_state() -> AppState {
    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .expect("no native root CA certificates found")
        .https_or_http()
        .enable_http1()
        .build();
    let hyper_client = HyperClient::builder(TokioExecutor::new()).build(https);

    let redis_client = redis::Client::open("redis://127.0.0.1/").unwrap();
    let redis_conn = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let redis_manager = RedisManager { conn: redis_conn };

    AppState {
        tunnels: Arc::new(RwLock::new(std::collections::HashMap::new())),
        responses: Arc::new(RwLock::new(std::collections::HashMap::new())),
        jwt_secret: Arc::new("test_secret".to_string()),
        redis: Arc::new(redis_manager),
        hyper_client,
        default_cloud_backend: "http://localhost:8080".to_string(),
        request_timeout: Duration::from_secs(1),
        health_data: Arc::new(RwLock::new(std::collections::HashMap::new())),
    }
}

#[tokio::test]
async fn test_login_success() {
    let state = mock_app_state().await;

    let payload = LoginPayload {
        username: "test".to_string(),
        password: "test".to_string(),
    };
    let response = login(axum::extract::State(state), axum::Json(payload))
        .await
        .unwrap()
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_login_fail() {
    let state = mock_app_state().await;

    let payload = LoginPayload {
        username: "wrong".to_string(),
        password: "wrong".to_string(),
    };
    let result = login(axum::extract::State(state), axum::Json(payload)).await;

    assert!(matches!(
        result,
        Err(rathole::error::AppError::InvalidCredentials)
    ));
}
