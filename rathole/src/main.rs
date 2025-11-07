
use anyhow::Result;
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
    routing::{any, get},
    Router,
};
use clap::Parser;
use futures_util::StreamExt;
use rathole::{
    protocol::{ControlMessage, HttpRequest, HttpResponse},
    run, Cli,
};
use redis::AsyncCommands;
use std::net::SocketAddr;
use tokio::{net::TcpListener, signal, sync::broadcast};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    redis: redis::Client,
}

async fn register(
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    println!("Registering tunnel {}", id);
    ws.on_upgrade(move |socket| handle_socket(socket, id, state.redis))
}

async fn handle_socket(mut socket: WebSocket, id: String, redis: redis::Client) {
    println!("New tunnel {} connected", id);

    let mut con = redis.get_async_connection().await.unwrap();
    let mut pubsub = con.into_pubsub();
    pubsub.subscribe(&id).await.unwrap();

    let mut rx = pubsub.on_message();

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
                                api_key,
                                target_subdomain,
                            } => {
                                println!(
                                    "Registering tunnel for subdomain: {} with api key: {}",
                                    target_subdomain, api_key
                                );
                            }
                            ControlMessage::Response { request_id, http } => {
                                let mut con = redis.get_async_connection().await.unwrap();
                                let res_str = serde_json::to_string(&http).unwrap();
                                let _: () = con.set_ex(request_id, res_str, 10).await.unwrap();
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
            Some(msg) = rx.next() => {
                let msg_str: String = msg.get_payload().unwrap();
                if socket.send(Message::Text(msg_str)).await.is_err() {
                    // client disconnected
                    break;
                }
            }
        }
    }

    println!("Tunnel {} disconnected", id);
}

async fn tunnel(
    Path((id, path)): Path<(String, String)>,
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    body: String,
) -> Response {
    println!("Tunneling {} for {}", path, id);

    let request_id = uuid::Uuid::new_v4().to_string();
    let mut con = state.redis.get_async_connection().await.unwrap();

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
        request_id: request_id.clone(),
        http: http_req,
    };
    let msg_str = serde_json::to_string(&msg).unwrap();
    let _: () = con.publish(&id, msg_str).await.unwrap();

    for _ in 0..10 {
        let res: Option<String> = con.get(&request_id).await.unwrap();
        if let Some(res_str) = res {
            let http_res: HttpResponse = serde_json::from_str(&res_str).unwrap();
            let mut builder = Response::builder().status(http_res.status);
            for (key, value) in http_res.headers {
                builder = builder.header(key, value);
            }
            return builder.body(Body::from(http_res.body)).unwrap();
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    "Tunnel not found".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    let is_atty = atty::is(atty::Stream::Stdout);
    let level = "info"; // if RUST_LOG not present, use `info` level
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)))
        .with_ansi(is_atty)
        .init();

    let redis = redis::Client::open("redis://127.0.0.1/")?;
    let state = AppState { redis };

    let app = Router::new()
        .route("/register/:id", get(register))
        .route("/tunnel/:id/*path", any(tunnel))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

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
async fn legacy_main() -> Result<()> {
    let args = Cli::parse();

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<bool>(1);
    tokio::spawn(async move {
        if let Err(e) = signal::ctrl_c().await {
            // Something really weird happened. So just panic
            panic!("Failed to listen for the ctrl-c signal: {:?}", e);
        }

        if let Err(e) = shutdown_tx.send(true) {
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

    run(args, shutdown_rx).await
}
