use anyhow::Result;
use axum::body::{to_bytes, Body};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use clap::Parser;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rathole::protocol::{ControlMessage, HttpRequest, HttpResponse};
use rathole::{run, Cli};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tracing_subscriber::EnvFilter;

use rathole::error::AppError;

type TunnelMap = Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>;
type ResponseMap = Arc<RwLock<HashMap<String, oneshot::Sender<HttpResponse>>>>;

#[derive(Clone)]
struct AppState {
    tunnels: TunnelMap,
    responses: ResponseMap,
    jwt_secret: Arc<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

#[derive(Debug, Deserialize)]
struct LoginPayload {
    username: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
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

async fn register(
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    println!("Registering tunnel {}", id);
    ws.on_upgrade(move |socket| {
        handle_socket(socket, id, state.tunnels, state.responses, state.jwt_secret)
    })
}

async fn handle_socket(
    mut socket: WebSocket,
    id: String,
    tunnels: TunnelMap,
    responses: ResponseMap,
    jwt_secret: Arc<String>,
) {
    println!("New tunnel {} connected", id);

    let (tx, mut rx) = mpsc::channel(100);

    loop {
        tokio::select! {
            Some(msg) = socket.recv() => {
                if let Ok(msg) = msg {
                    if let Message::Text(text) = msg {
                        let msg: ControlMessage = match serde_json::from_str(&text) {
                            Ok(msg) => msg,
                            Err(err) => {
                                println!("Failed to parse message: {}", err);
                                continue;
                            }
                        };

                        match msg {
                            ControlMessage::Register {
                                token,
                                target_subdomain,
                            } => {
                                let validation = Validation::default();
                                match decode::<Claims>(&token, &DecodingKey::from_secret(jwt_secret.as_bytes()), &validation) {
                                    Ok(claims) => {
                                        println!(
                                            "Registering tunnel for subdomain: {} with claims: {:?}",
                                            target_subdomain, claims
                                        );
                                        tunnels.write().await.insert(id.clone(), tx.clone());
                                    }
                                    Err(err) => {
                                        println!("Invalid token: {}", err);
                                        return;
                                    }
                                }
                            }
                            ControlMessage::Response { request_id, http } => {
                                if let Some(tx) = responses.write().await.remove(&request_id) {
                                    tx.send(http).unwrap();
                                }
                            }
                            _ => {
                                todo!();
                            }
                        }
                    }
                } else {
                    // client disconnected
                    break;
                }
            },
            Some(msg) = rx.recv() => {
                if socket.send(msg).await.is_err() {
                    // client disconnected
                    break;
                }
            }
        }
    }

    println!("Tunnel {} disconnected", id);
    tunnels.write().await.remove(&id);
}

async fn tunnel(
    Path((id, path)): Path<(String, String)>,
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    println!("Tunneling {} for {}", path, id);

    let request_id = uuid::Uuid::new_v4().to_string();

    if let Some(tx) = state.tunnels.read().await.get(&id) {
        let (res_tx, res_rx) = oneshot::channel();
        state
            .responses
            .write()
            .await
            .insert(request_id.clone(), res_tx);

        let http_req = HttpRequest {
            method: method.to_string(),
            path,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
                .collect(),
            body: body.into_bytes(),
        };
        let msg = ControlMessage::Request {
            request_id,
            http: http_req,
        };
        let msg_str = serde_json::to_string(&msg).map_err(AppError::JsonError)?;
        if tx.send(Message::Text(msg_str)).await.is_ok() {
            match res_rx.await {
                Ok(res) => {
                    let mut builder = Response::builder().status(res.status);
                    for (key, value) in res.headers {
                        builder = builder.header(key, value);
                        //.header(|e| AppError::Other(e.into()))?;
                    }
                    return Ok(builder
                        .body(axum::body::Body::from(res.body))
                        .map_err(|e| AppError::Other(e.into()))?);
                }
                Err(_) => {
                    return Err(AppError::TunnelResponseError);
                }
            }
        } else {
            return Err(AppError::TunnelSendError);
        }
    }

    Err(AppError::TunnelNotFound)
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let is_atty = atty::is(atty::Stream::Stdout);
    let level = "info"; // if RUST_LOG not present, use `info` level
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)),
        )
        .with_ansi(is_atty)
        .init();

    let config = rathole::Config::from_file(&std::path::PathBuf::from("config.toml"))
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    let jwt_secret = Arc::new(config.server.unwrap().jwt_secret.to_string());

    let state = AppState {
        tunnels: TunnelMap::default(),
        responses: ResponseMap::default(),
        jwt_secret,
    };

    let app = Router::new()
        .route("/login", post(login))
        .route("/register/:id", get(register))
        .route("/tunnel/:id/*path", any(tunnel))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::IoError(e))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("signal received, starting graceful shutdown");
}

#[tokio::main]
#[allow(dead_code)]
async fn legacy_main() -> Result<(), AppError> {
    let args = Cli::parse();

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<bool>(1);
    let (_update_tx, mut update_rx) = mpsc::channel(1);

    let cloned_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = signal::ctrl_c().await {
            // Something really weird happened. So just panic
            panic!("Failed to listen for the ctrl-c signal: {:?}", e);
        }

        if let Err(e) = cloned_shutdown_tx.send(true) {
            // shutdown signal must be catched and handle properly
            // `rx` must not be dropped
            panic!("Failed to send shutdown signal: {:?}", e);
        }
    });

    #[cfg(feature = "console")]
    {
        console_subscriber::init();

        tracing::info!("console_subscriber enabled");
    }
    #[cfg(not(feature = "console"))]
    {
        let is_atty = atty::is(atty::Stream::Stdout);

        let level = "info"; // if RUST_LOG not present, use `info` level
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)),
            )
            .with_ansi(is_atty)
            .init();
    }

    run(args, shutdown_rx, shutdown_tx.clone(), &mut update_rx)
        .await
        .map_err(|e| AppError::Other(e.into()))
}
