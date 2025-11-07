
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path,
    },
    response::Response,
    routing::get,
    Router,
};
use clap::Parser;
use rathole::{protocol::ControlMessage, run, Cli};
use std::net::SocketAddr;
use tokio::{net::TcpListener, signal, sync::broadcast};
use tracing_subscriber::EnvFilter;

async fn register(Path(id): Path<String>, ws: WebSocketUpgrade) -> Response {
    println!("Registering tunnel {}", id);
    ws.on_upgrade(move |socket| handle_socket(socket, id))
}

async fn handle_socket(mut socket: WebSocket, id: String) {
    println!("New tunnel {} connected", id);
    while let Some(msg) = socket.recv().await {
        let msg = if let Ok(msg) = msg {
            msg
        } else {
            // client disconnected
            return;
        };

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
                _ => {
                    todo!();
                }
            }
        }
    }
}

async fn tunnel(Path((id, path)): Path<(String, String)>) -> String {
    format!("Tunneling {} for {}", path, id)
}

#[tokio::main]
async fn main() -> Result<()> {
    let is_atty = atty::is(atty::Stream::Stdout);
    let level = "info"; // if RUST_LOG not present, use `info` level
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)))
        .with_ansi(is_atty)
        .init();

    let app = Router::new()
        .route("/register/:id", get(register))
        .route("/tunnel/:id/*path", get(tunnel));

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
