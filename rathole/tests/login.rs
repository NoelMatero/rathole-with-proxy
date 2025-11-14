use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use rathole::main::login;
use rathole::protocol::Claims;
use rathole::proxy::AppState;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot` testing

#[tokio::test]
async fn test_login_success() {
    let state = AppState {
        tunnels: Default::default(),
        responses: Default::default(),
        jwt_secret: Arc::new("supersecret".to_string()),
        redis: Arc::new(rathole::RedisManager {
            conn: redis::aio::MultiplexedConnection::new(),
        }), // mock if needed
        hyper_client: todo!(),
        default_cloud_backend: "localhost:4000".to_string(),
        request_timeout: std::time::Duration::from_secs(5),
    };

    let payload = json!({"username": "test", "password": "test"});
    let response = login(axum::extract::State(state), axum::Json(payload))
        .await
        .unwrap()
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_login_fail() {
    let state = AppState {
        tunnels: Default::default(),
        responses: Default::default(),
        jwt_secret: Arc::new("supersecret".to_string()),
        redis: Arc::new(rathole::RedisManager {
            conn: redis::aio::MultiplexedConnection::new(),
        }),
        hyper_client: todo!(),
        default_cloud_backend: "localhost:4000".to_string(),
        request_timeout: std::time::Duration::from_secs(5),
    };

    let payload = json!({"username": "wrong", "password": "wrong"});
    let result = login(axum::extract::State(state), axum::Json(payload)).await;

    assert!(matches!(
        result,
        Err(rathole::error::AppError::InvalidCredentials)
    ));
}
