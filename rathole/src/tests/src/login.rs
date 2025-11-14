#[tokio::test]
async fn test_login_success() {
    let jwt_secret = Arc::new("secret".to_string());
    let state = AppState {
        tunnels: TunnelMap::default(),
        responses: ResponseMap::default(),
        jwt_secret,
        redis: Arc::new(mock_redis()),
        hyper_client: mock_hyper_client(),
        default_cloud_backend: "http://localhost:4000".to_string(),
        request_timeout: std::time::Duration::from_secs(5),
    };

    let payload = LoginPayload {
        username: "test".into(),
        password: "test".into(),
    };

    let response = login(axum::extract::State(state), axum::Json(payload))
        .await
        .unwrap();
    assert!(matches!(response.0, StatusCode::OK));
}

