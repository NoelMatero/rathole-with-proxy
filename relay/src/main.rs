use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{stream::StreamExt, SinkExt};
use hyper::{Request as HyperRequest, Response as HyperResponse, StatusCode};
use hyper_reverse_proxy::ReverseProxy;
use hyper_trust_dns::{RustlsHttpsConnector, TrustDnsResolver};
use lazy_static::lazy_static;
use shared::ControlMessage;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;

lazy_static! {
    static ref PROXY_CLIENT: ReverseProxy<RustlsHttpsConnector> = {
        ReverseProxy::new(
            hyper::Client::builder().build::<_, hyper::Body>(TrustDnsResolver::default().into_rustls_webpki_https_connector()),
        )
    };
}

#[derive(Default, Clone)]
struct TunnelRegistry {
    tunnels: Arc<Mutex<std::collections::HashMap<String, mpsc::Sender<ControlMessage>>>>,
    pending_requests: Arc<Mutex<std::collections::HashMap<String, mpsc::Sender<HyperResponse<hyper::Body>>>>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let registry = TunnelRegistry::default();

    let app = Router::new()
        .route("/register/:id", get(ws_handler))
        .fallback(proxy_handler)
        .with_state(registry);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
    info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn ws_handler(
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
    State(registry): State<TunnelRegistry>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(id, socket, registry))
}

async fn handle_socket(id: String, mut socket: WebSocket, registry: TunnelRegistry) {
    info!("Tunnel {} registered", &id);
    let (tx_to_tunnel, mut rx_from_proxy) = mpsc::channel(100);
    registry.tunnels.lock().await.insert(id.clone(), tx_to_tunnel);

    loop {
        tokio::select! {
            Some(msg) = rx_from_proxy.recv() => {
                if let Ok(msg_text) = serde_json::to_string(&msg) {
                    if socket.send(Message::Text(msg_text)).await.is_err() {
                        break;
                    }
                }
            }
            Some(Ok(msg)) = socket.next() => {
                if let Message::Text(text) = msg {
                    if let Ok(control_msg) = serde_json::from_str::<ControlMessage>(&text) {
                        if let ControlMessage::Response { request_id, status, headers, body } = control_msg {
                            if let Some(sender) = registry.pending_requests.lock().await.remove(&request_id) {
                                let mut builder = HyperResponse::builder().status(status);
                                for (key, value) in headers {
                                    builder = builder.header(key, value);
                                }
                                if sender.send(builder.body(body.into()).unwrap()).is_err() {
                                    tracing::error!("Failed to send response to pending request {}", request_id);
                                }
                            }
                        }
                    }
                }
            }
            else => break,
        }
    }

    info!("Tunnel {} disconnected", &id);
    registry.tunnels.lock().await.remove(&id);
}

async fn proxy_handler(
    State(registry): State<TunnelRegistry>,
    req: HyperRequest<hyper::Body>,
) -> Result<HyperResponse<hyper::Body>, StatusCode> {
    let client_ip = IpAddr::from([127, 0, 0, 1]);
    let backend_url = "http://httpbin.org";

    if let Some(host) = req.headers().get("host").and_then(|h| h.to_str().ok()) {
        let host_parts: Vec<&str> = host.split('.').collect();
        if !host_parts.is_empty() {
            let subdomain = host_parts[0];
            if let Some(tunnel_sender) = registry.tunnels.lock().await.get(subdomain).cloned() {
                return forward_to_tunnel(req, tunnel_sender, registry, subdomain).await;
            }
        }
    }

    forward_to_fallback(client_ip, backend_url, req).await
}

async fn forward_to_tunnel(
    req: HyperRequest<hyper::Body>,
    tunnel_sender: mpsc::Sender<ControlMessage>,
    registry: TunnelRegistry,
    subdomain: &str,
) -> Result<HyperResponse<hyper::Body>, StatusCode> {
    info!("Tunnel found for subdomain: {}. Forwarding via WebSocket.", subdomain);
    let (response_tx, mut response_rx) = mpsc::channel(1);
    let request_id = Uuid::new_v4().to_string();
    registry.pending_requests.lock().await.insert(request_id.clone(), response_tx);

    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await.unwrap().to_vec();

    let control_message = ControlMessage::Request {
        request_id: request_id.clone(),
        method: parts.method.to_string(),
        path: parts.uri.to_string(),
        headers: parts.headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string())).collect(),
        body: body_bytes,
    };

    if tunnel_sender.send(control_message).await.is_err() {
        tracing::error!("Failed to send request to tunnel {}. Falling back to HTTP.", subdomain);
        registry.pending_requests.lock().await.remove(&request_id);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Some(response) = response_rx.recv().await {
        Ok(response)
    } else {
        tracing::error!("Did not receive response from tunnel {}. Falling back to HTTP.", subdomain);
        registry.pending_requests.lock().await.remove(&request_id);
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

async fn forward_to_fallback(
    client_ip: IpAddr,
    backend_url: &str,
    req: HyperRequest<hyper::Body>,
) -> Result<HyperResponse<hyper::Body>, StatusCode> {
    info!("No tunnel found for host. Forwarding to fallback URL: {}", backend_url);
    match PROXY_CLIENT.call(client_ip, backend_url, req).await {
        Ok(response) => Ok(response),
        Err(error) => {
            tracing::error!("Failed to forward request: {:?}", error);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}