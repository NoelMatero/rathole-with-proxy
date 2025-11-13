use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use bytes::Bytes;
use chrono::{Duration, Utc};
use http_body_util::Full;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rathole::error::AppError;
use rathole::protocol::{ControlMessage, HttpRequest, HttpResponse};
use rathole::proxy::AppState;
use rathole::RedisManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::timeout;
use tracing_subscriber::EnvFilter;

type TunnelMap = Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>;
type ResponseMap = Arc<RwLock<HashMap<String, oneshot::Sender<HttpResponse>>>>;

/*#[derive(Clone)]
struct RedisManager {
    conn: redis::aio::MultiplexedConnection,
}*/

/*#[derive(Clone)]
struct AppState {
    tunnels: TunnelMap,
    responses: ResponseMap,
    jwt_secret: Arc<String>,
    redis: Arc<RedisManager>,
}*/

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
    cleanup_tx: mpsc::Sender<String>,
) -> Response {
    println!("Registering tunnel {}", id);
    ws.on_upgrade(move |socket| handle_socket(socket, id, state, cleanup_tx))
}

async fn handle_socket(
    mut socket: WebSocket,
    id: String,
    state: AppState,
    cleanup_tx: mpsc::Sender<String>,
) {
    println!("New tunnel {} connected", id);

    // The first message from the client must be a Register message
    if let Some(Ok(Message::Text(text))) = socket.recv().await {
        let msg: ControlMessage = match serde_json::from_str(&text) {
            Ok(msg) => msg,
            Err(err) => {
                println!("Failed to parse register message: {}", err);
                return;
            }
        };

        if let ControlMessage::Register {
            token,
            target_subdomain,
        } = msg
        {
            let validation = Validation::default();
            match decode::<Claims>(
                &token,
                &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
                &validation,
            ) {
                Ok(claims) => {
                    println!(
                        "Registering tunnel for subdomain: {} with claims: {:?}",
                        target_subdomain, claims
                    );

                    let mut redis_conn = state.redis.conn.clone();
                    let _: () = redis_conn
                        .hset(format!("tunnel:{}", id), "status", "online")
                        .await
                        .unwrap();

                    let (tx, mut rx) = mpsc::channel(100);
                    state.tunnels.write().await.insert(id.clone(), tx);

                    // Main loop to forward messages
                    loop {
                        tokio::select! {
                            // Handle messages from the local tunnel handler → client
                            Some(msg) = rx.recv() => {
                                if socket.send(msg.clone()).await.is_err() {
                                    println!("Failed to send message to {id}, disconnecting.");
                                    break;
                                }
                            },

                            // Handle messages from client → tunnel handler
                            res = async { timeout(StdDuration::from_secs(30), socket.recv()).await } => {
                                match res {
                                    // Received message successfully
                                    Ok(Some(Ok(Message::Text(text)))) => {
                                        println!("Received message from {id}: {}", text);
                                        // handle message normally
                                    }

                                    // Client closed, timeout, or error
                                    Ok(Some(Err(e))) => {
                                        println!("Error from {id}: {}", e);
                                        break;
                                    }
                                    Ok(None) => {
                                        println!("Client {id} disconnected.");
                                        break;
                                    }
                                    Err(_) => {
                                        println!("Timeout waiting for {id}, closing.");
                                        break;
                                    }

                                    _ => {
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    println!("Invalid token for tunnel {}: {}", id, err);
                }
            }
        } else {
            println!("Tunnel {} failed to send a register message", id);
        }
    }

    // When the loop breaks, the client has disconnected.
    // Send the ID to the cleanup task.
    let _ = cleanup_tx.send(id).await;
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
        if tx.send(Message::Text(msg_str.into())).await.is_ok() {
            match res_rx.await {
                Ok(res) => {
                    let mut builder = Response::builder().status(res.status);
                    for (key, value) in res.headers {
                        builder = builder.header(key, value);
                        //.header(|e| AppError::Other(e.into()))?;
                    }
                    return builder
                        .body(axum::body::Body::from(res.body))
                        .map_err(|e| AppError::Other(e.into()));
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
    println!("works");
    let is_atty = atty::is(atty::Stream::Stdout);
    let level = "info"; // if RUST_LOG not present, use `info` level
    println!("works");

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)),
        )
        .with_ansi(is_atty)
        .init();

    println!("works");

    let config = rathole::Config::from_file(&std::path::PathBuf::from("config.toml"))
        .await
        .map_err(|e| AppError::Other(e.into()))?;

    println!("works");

    let jwt_secret = Arc::new(config.server.unwrap().jwt_secret.to_string());

    let redis_client =
        redis::Client::open("redis://127.0.0.1/").map_err(|e| AppError::Other(e.into()))?;
    println!("works");

    let redis_conn_manager = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| AppError::Other(e.into()))?;
    println!("works");

    let redis_manager = Arc::new(RedisManager {
        conn: redis_conn_manager,
    });

    println!("works");

    let (cleanup_tx, mut cleanup_rx) = mpsc::channel::<String>(100);

    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .unwrap() // TODO:
        .https_or_http()
        .enable_http1()
        .build();

    let hyper_client =
        HyperClient::builder(TokioExecutor::default()).build::<_, Full<Bytes>>(https);

    let state = AppState {
        tunnels: TunnelMap::default(),
        responses: ResponseMap::default(),
        jwt_secret,
        redis: redis_manager,
        hyper_client,
        default_cloud_backend: "localhost:4000".to_string(),
        request_timeout: StdDuration::from_secs(10),
    };

    let state_clone = state.clone();
    let cleanup_tx_clone = cleanup_tx.clone();
    tokio::spawn(async move {
        println!("Cleanup task started");
        while let Some(id) = cleanup_rx.recv().await {
            println!("Cleaning up tunnel: {}", id);
            let mut redis_conn = state_clone.redis.conn.clone();
            let _: () = redis_conn
                .hset(format!("tunnel:{}", id), "status", "offline")
                .await
                .unwrap();
            state_clone.tunnels.write().await.remove(&id);
        }
    });

    let app = Router::new()
        .route("/login", post(login))
        .route(
            "/register/:id",
            get(move |path, ws, state| register(path, ws, state, cleanup_tx)),
        )
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

/*#[tokio::main]
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
}*/
