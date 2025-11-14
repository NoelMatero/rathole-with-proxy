use crate::constants::UDP_BUFFER_SIZE;
use crate::protocol::{ControlMessage, HttpRequest, HttpResponse};
use crate::RedisManager;
use axum::body::{to_bytes, Body as AxumBody};
use axum::extract::State;
use axum::http::{
    header, HeaderMap, Method, Request as AxumRequest, Response as AxumResponse, StatusCode,
};
//use hyper::body::Body as HyperBody;
//use hyper::body::Incoming as HyperBody;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::{Body as _, Incoming as HyperBody}; // for trait methods like .collect()
use hyper_util::client::legacy::Client as HyperClient;
//use hyper_util::HttpsConnectorBuilder;
use http_body_util::BodyExt;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use std::usize;
use tokio::sync::{oneshot, RwLock};
use tokio::time::timeout;
use tracing::debug;

/// Extend your AppState to include a shared hyper client and a request timeout.
/// You already have tunnels/responses/redis/jwt_secret; add these fields.
#[derive(Clone)]
pub struct AppState {
    pub tunnels: Arc<
        RwLock<
            std::collections::HashMap<
                String,
                tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
            >,
        >,
    >,
    pub responses: Arc<RwLock<std::collections::HashMap<String, oneshot::Sender<HttpResponse>>>>,
    pub jwt_secret: Arc<String>,
    pub redis: Arc<crate::RedisManager>, // adjust path to your RedisManager
    pub hyper_client: HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>,
    pub default_cloud_backend: String, // e.g. "https://myapp-cloud.example.com"
    pub request_timeout: Duration,     // e.g. Duration::from_secs(10)
}

/// Main handler to be used as the proxy forwarder.
pub async fn proxy_handler(
    State(state): State<AppState>,
    axum_req: AxumRequest<AxumBody>,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    println!("req: {:?}", axum_req);
    // 1) resolve subdomain from Host header
    let host = axum_req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let subdomain = host.split('.').next().unwrap_or_default().to_string();
    debug!(
        "Incoming request for host={} -> subdomain={}",
        host, subdomain
    );

    // 2) If tunnel exists, forward through tunnel
    if let Some(tunnel_tx) = {
        let read = state.tunnels.read().await;
        read.get(&subdomain).cloned()
    } {
        return forward_via_tunnel(axum_req, tunnel_tx, &state).await;
    }

    // 3) Otherwise forward to cloud backend
    forward_to_cloud(axum_req, &state).await
}

/// Build ControlMessage::Request and use the oneshot pattern over the tunnel.
/// Returns an axum::Response on success or StatusCode on failure.
async fn forward_via_tunnel(
    req: AxumRequest<AxumBody>,
    tunnel_tx: tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
    state: &AppState,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    // Create request id + oneshot pair
    let request_id = uuid::Uuid::new_v4().to_string();
    let (resp_tx, resp_rx) = oneshot::channel();

    // register pending response
    {
        let mut w = state.responses.write().await;
        w.insert(request_id.clone(), resp_tx);
    }

    // Convert Axum request into HttpRequest (protocol struct)
    let (parts, body) = req.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let http_req = HttpRequest {
        method: parts.method.to_string(),
        path: parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string()),
        headers: headers_to_map(&parts.headers),
        body: body_bytes.to_vec(),
    };

    let msg = ControlMessage::Request {
        request_id: request_id.clone(),
        http: http_req,
    };

    // send to tunnel (serialize to JSON text)
    let msg_str = serde_json::to_string(&msg).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if tunnel_tx
        .send(axum::extract::ws::Message::Text(msg_str.into()))
        .await
        .is_err()
    {
        // remove pending entry
        state.responses.write().await.remove(&request_id);
        return Err(StatusCode::BAD_GATEWAY);
    }

    // wait for response with configurable timeout
    match timeout(state.request_timeout, resp_rx).await {
        Ok(Ok(http_response)) => {
            // build axum response from HttpResponse
            let mut builder = AxumResponse::builder().status(http_response.status);
            {
                let headers = builder.headers_mut().unwrap();
                for (k, v) in http_response.headers {
                    // best-effort header insertion
                    headers.insert(
                        header::HeaderName::from_bytes(k.as_bytes())
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                        header::HeaderValue::from_str(&v)
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                    );
                }
            }
            let body = AxumBody::from(http_response.body);
            let response = builder
                .body(body)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(response)
        }
        Ok(Err(_recv_err)) => {
            // oneshot canceled
            state.responses.write().await.remove(&request_id);
            Err(StatusCode::BAD_GATEWAY)
        }
        Err(_) => {
            // timeout
            state.responses.write().await.remove(&request_id);
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

/// Forward to cloud backend using the shared hyper::Client.
/// This implementation copies the request body once (safe, clear) and streams response back.
async fn forward_to_cloud(
    req: AxumRequest<AxumBody>,
    state: &AppState,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    // Convert axum body -> hyper body by extracting bytes
    let (mut parts, body) = req.into_parts();
    let bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let hyper_body = Full::new(bytes);
    //let hyper_body = HyperBody::from(bytes);

    // build target URI: use default_cloud_backend + original path+query
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = format!(
        "{}{}",
        state.default_cloud_backend.trim_end_matches('/'),
        path_and_query
    );
    let target_uri: hyper::Uri = target.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    parts.uri = target_uri; // types are compatible in axum/hyper stack

    // Rebuild hyper request
    let hyper_req = hyper::Request::from_parts(parts, hyper_body);

    // Send with timeout
    match timeout(state.request_timeout, state.hyper_client.request(hyper_req)).await {
        Ok(Ok(hyper_res)) => {
            // Convert hyper::Response<HyperBody> -> AxumResponse<AxumBody>
            let (parts, body) = hyper_res.into_parts();
            let bytes = body
                .collect()
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .to_bytes();
            let axum_resp = AxumResponse::from_parts(parts, AxumBody::from(bytes));
            Ok(axum_resp)
        }
        Ok(Err(e)) => {
            tracing::error!("Cloud backend request failed: {}", e);
            Err(StatusCode::BAD_GATEWAY)
        }
        Err(_) => {
            tracing::warn!("Cloud backend timed out");
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

/// Helper: convert HeaderMap -> Vec<(String,String)> map (best-effort)
fn headers_to_map(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|s| (k.as_str().to_string(), s.to_string()))
        })
        .collect()
}
