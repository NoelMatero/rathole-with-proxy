use crate::protocol::{ControlMessage, HttpRequest, HttpResponse};
use axum::body::{to_bytes, Body as AxumBody};
use axum::extract::State;
use axum::http::{header, HeaderMap, Request as AxumRequest, Response as AxumResponse, StatusCode};
//use hyper::body::Body as HyperBody;
//use hyper::body::Incoming as HyperBody;
use bytes::Bytes;
use http_body_util::Full;
// for trait methods like .collect()
use hyper_util::client::legacy::Client as HyperClient;
//use hyper_util::HttpsConnectorBuilder;
use http_body_util::BodyExt;
use hyper_util::client::legacy::connect::HttpConnector;
use serde_json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::usize;
use tokio::sync::{oneshot, RwLock};
use tokio::time::timeout;
use tracing::debug;

#[derive(Clone, Debug)]
pub enum HealthStatus {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Debug)]
pub struct TunnelHealth {
    pub status: HealthStatus,
    pub last_update: Instant, // last time any health info / success recorded
    pub last_success: Option<Instant>, // last time a proxied request succeeded
    pub last_failure: Option<Instant>, // last time a proxied request failed (timeout/error)
    pub consecutive_successes: u32,
    pub consecutive_failures: u32,
    pub avg_latency_ms: Option<f64>, // rolling latency estimate (ms)
    pub error_rate: f64,             // rolling error rate 0.0..1.0
    pub last_transition: Instant,    // last time status changed (for hysteresis)
}

impl Default for TunnelHealth {
    fn default() -> Self {
        let now = Instant::now();
        TunnelHealth {
            status: HealthStatus::Normal,
            last_update: now,
            last_success: None,
            last_failure: None,
            consecutive_successes: 0,
            consecutive_failures: 0,
            avg_latency_ms: None,
            error_rate: 0.0,
            last_transition: now,
        }
    }
}

/// Routing decision
#[derive(Debug, PartialEq, Eq)]
enum RoutingDecision {
    RouteLocal,
    HybridSticky, // prefer cloud, but if cookie says local keep it
    ForceCloud,
    RouteCloud,
}

const STALE_THRESHOLD: Duration = Duration::from_secs(30); // if last_update older than this, treat stale
const WARNING_TO_CRITICAL_FAILURES: u32 = 5; // how many consecutive failures push status -> Critical
const SUCCESS_TO_NORMAL: u32 = 3; // successes required to move from Warning -> Normal
const HYSTERESIS_DURATION: Duration = Duration::from_secs(10); // minimal time to wait between transitions

fn get_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_hdr = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_hdr.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&format!("{}=", name)) {
            return Some(val.to_string());
        }
    }
    None
}

/// Set tunnel cookie on the response. value should be the tunnel id (e.g. "local-01" or "cloud").
fn insert_tunnel_cookie(resp: &mut AxumResponse<AxumBody>, value: &str) {
    // HttpOnly and Path=/; you can extend: Secure, SameSite, Expires etc.
    let cookie_value = format!("tunnel={}; Path=/; HttpOnly", value);
    resp.headers_mut().insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&cookie_value).unwrap(),
    );
}

fn decide_routing(
    id: &str,
    health_map: &std::collections::HashMap<String, TunnelHealth>,
) -> RoutingDecision {
    if let Some(h) = health_map.get(id) {
        let now = Instant::now();
        if now.duration_since(h.last_update) > STALE_THRESHOLD {
            return RoutingDecision::ForceCloud;
        }

        if matches!(h.status, HealthStatus::Critical) {
            return RoutingDecision::ForceCloud;
        }

        if matches!(h.status, HealthStatus::Warning) {
            return RoutingDecision::HybridSticky;
        }

        RoutingDecision::RouteLocal
    } else {
        RoutingDecision::RouteLocal
    }
}

/// Extend your AppState to include a shared hyper client and a request timeout.
/// You already have tunnels/responses/redis/jwt_secret; add these fields.
#[derive(Clone, Debug)]
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
    pub health_data: TunnelHealthMap,
}

pub type TunnelHealthMap = Arc<RwLock<std::collections::HashMap<String, TunnelHealth>>>;

async fn mark_tunnel_success(state: &AppState, id: &str, latency_ms: Option<f64>) {
    let mut map = state.health_data.write().await;
    let entry = map
        .entry(id.to_string())
        .or_insert_with(TunnelHealth::default);
    entry.last_update = Instant::now();
    entry.last_success = Some(Instant::now());
    entry.consecutive_successes = entry.consecutive_successes.saturating_add(1);
    entry.consecutive_failures = 0;

    // update rolling avg latency
    if let Some(lat) = latency_ms {
        entry.avg_latency_ms = Some(match entry.avg_latency_ms {
            Some(prev) => (prev * 0.8) + (lat * 0.2),
            None => lat,
        });
    }

    // decay error rate slowly
    entry.error_rate = (entry.error_rate * 0.9).max(0.0);

    // bump status towards Normal if thresholds met and hysteresis respected
    if matches!(entry.status, HealthStatus::Warning)
        && entry.consecutive_successes >= SUCCESS_TO_NORMAL
    {
        let now = Instant::now();
        if now.duration_since(entry.last_transition) >= HYSTERESIS_DURATION {
            entry.status = HealthStatus::Normal;
            entry.last_transition = now;
        }
    }
}

/// Call when a proxied request via tunnel failed (timeout/error)
async fn mark_tunnel_failure(state: &AppState, id: &str) {
    let mut map = state.health_data.write().await;
    let entry = map
        .entry(id.to_string())
        .or_insert_with(TunnelHealth::default);
    entry.last_update = Instant::now();
    entry.last_failure = Some(Instant::now());
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    entry.consecutive_successes = 0;

    // increase error rate
    entry.error_rate = (entry.error_rate * 0.8) + 0.2;

    // escalate to Warning then Critical based on counts/hysteresis
    let now = Instant::now();
    if entry.consecutive_failures >= WARNING_TO_CRITICAL_FAILURES {
        if now.duration_since(entry.last_transition) >= HYSTERESIS_DURATION {
            entry.status = HealthStatus::Critical;
            entry.last_transition = now;
        }
    } else {
        // move to Warning if not already Critical
        if !matches!(entry.status, HealthStatus::Critical)
            && now.duration_since(entry.last_transition) >= HYSTERESIS_DURATION
        {
            entry.status = HealthStatus::Warning;
            entry.last_transition = now;
        }
    }
}

/// Main handler to be used as the proxy forwarder.
pub async fn proxy_handler(
    State(state): State<AppState>,
    axum_req: AxumRequest<AxumBody>,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    println!("proxy handler");
    // 1) resolve subdomain from Host header
    let host = axum_req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default() // TODO: no unwrap
        .to_string();

    let subdomain = host.split('.').next().unwrap_or_default().to_string(); // TODO: no unwrap
    println!(
        "Incoming request for host={} -> subdomain={}",
        host, subdomain
    );

    let preferred_tunnel = get_cookie_value(axum_req.headers(), "tunnel");

    // Snapshot health map
    if let Some(pref_id) = preferred_tunnel.clone() {
        println!("preferred tunnel cookie present: {}", pref_id);
        // check if it exists in registered tunnels and is healthy
        if let Some(tx) = state.tunnels.read().await.get(&pref_id).cloned() {
            // Evaluate health for this specific tunnel
            let healthy = {
                let guard = state.health_data.read().await;
                match guard.get(&pref_id) {
                    Some(h) => {
                        // healthy if not Critical and not stale
                        !matches!(h.status, HealthStatus::Critical)
                            && h.last_update.elapsed() < STALE_THRESHOLD
                    }
                    None => true, // unknown => optimistic, allow it
                }
            };

            if healthy {
                // Forward to this exact tunnel (sticky) and keep cookie
                let mut response = forward_via_tunnel(&pref_id, axum_req, tx, &state).await?;
                insert_tunnel_cookie(&mut response, &pref_id);
                return Ok(response);
            } else {
                println!(
                    "preferred tunnel {} unhealthy or stale, looking for alternative",
                    pref_id
                );
                // Fall through to choose_best_tunnel
            }
        } else {
            println!("preferred tunnel {} not registered", pref_id);
        }
    }

    if let Some((chosen_id, tx)) = choose_best_tunnel(&state, &subdomain).await {
        // Check health for chosen_id
        let healthy = {
            let guard = state.health_data.read().await;
            match guard.get(&chosen_id) {
                Some(h) => {
                    !matches!(h.status, HealthStatus::Critical)
                        && h.last_update.elapsed() < STALE_THRESHOLD
                }
                None => true,
            }
        };

        if healthy {
            let mut response = forward_via_tunnel(&chosen_id, axum_req, tx, &state).await?;
            // set tunnel cookie so future requests stick to chosen_id
            insert_tunnel_cookie(&mut response, &chosen_id);
            return Ok(response);
        } else {
            println!(
                "chosen tunnel {} unhealthy -> falling back to cloud",
                chosen_id
            );
        }
    } else {
        println!("no local tunnels available for subdomain {}", subdomain);
    }

    // Fallback: forward to cloud. We set tunnel=cloud to mark sticky to cloud for client.
    println!("fallback happening");
    let mut cloud_resp = forward_to_cloud(axum_req, &state, false).await?;
    insert_tunnel_cookie(&mut cloud_resp, "cloud");
    Ok(cloud_resp)
}

/// Choose the best tunnel for a logical subdomain.
///
/// Selection rules:
/// 1. Consider tunnels whose key equals the subdomain, or whose key starts with `subdomain-`.
///    This allows multiple instances like `app-01`, `app-02`.
/// 2. Prefer non-Critical status, then lower error_rate, then lower avg_latency_ms.
/// 3. Returns (id, tx) if found.
async fn choose_best_tunnel(
    state: &AppState,
    subdomain: &str,
) -> Option<(
    String,
    tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
)> {
    let tunnels_guard = state.tunnels.read().await;
    let health_guard = state.health_data.read().await;

    // Collect candidate ids that match the subdomain naming convention.
    let mut candidates: Vec<String> = tunnels_guard
        .keys()
        .filter(|k| {
            // exact match or prefixed instances like "subdomain-" to support multiple instances
            k.as_str() == subdomain || k.starts_with(&format!("{}-", subdomain))
        })
        .cloned()
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Score each candidate based on health and latency/error
    candidates.sort_by(|a, b| {
        println!("candidates: a: {}, b: {}", a.clone(), b.clone());
        let ha = health_guard.get(a);
        let hb = health_guard.get(b);

        // Prefer healthier status: Normal < Warning < Critical
        let sa = match ha {
            Some(h) => match h.status {
                HealthStatus::Normal => 0,
                HealthStatus::Warning => 1,
                HealthStatus::Critical => 2,
            },
            None => 1, // unknown => treat as Warning-ish
        };
        let sb = match hb {
            Some(h) => match h.status {
                HealthStatus::Normal => 0,
                HealthStatus::Warning => 1,
                HealthStatus::Critical => 2,
            },
            None => 1,
        };
        sa.cmp(&sb)
            .then_with(|| {
                // lower error_rate better
                let ea = ha.map(|h| h.error_rate).unwrap_or(0.5);
                let eb = hb.map(|h| h.error_rate).unwrap_or(0.5);
                ea.partial_cmp(&eb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                // lower latency better
                let la = ha.and_then(|h| h.avg_latency_ms).unwrap_or(9999.0);
                let lb = hb.and_then(|h| h.avg_latency_ms).unwrap_or(9999.0);
                la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    // pick first candidate that has a sender
    for id in candidates {
        if let Some(tx) = tunnels_guard.get(&id).cloned() {
            return Some((id, tx));
        }
    }
    None
}

async fn forward_via_tunnel(
    id: &str,
    req: AxumRequest<AxumBody>,
    tunnel_tx: tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
    state: &AppState,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let (resp_tx, resp_rx) = oneshot::channel();

    state
        .responses
        .write()
        .await
        .insert(request_id.clone(), resp_tx);

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

    let msg_str = serde_json::to_string(&msg).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // send request over tunnel
    if tunnel_tx
        .send(axum::extract::ws::Message::Text(msg_str.into()))
        .await
        .is_err()
    {
        state.responses.write().await.remove(&request_id);
        mark_tunnel_failure(state, id).await;
        return Err(StatusCode::BAD_GATEWAY);
    }

    let start = Instant::now();
    match timeout(state.request_timeout, resp_rx).await {
        Ok(Ok(http_response)) => {
            let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

            // success — update health
            mark_tunnel_success(state, id, Some(latency_ms)).await;

            // build axum response
            let mut builder = AxumResponse::builder().status(http_response.status);
            {
                let headers = builder.headers_mut().unwrap();
                for (k, v) in http_response.headers {
                    headers.insert(
                        header::HeaderName::from_bytes(k.as_bytes())
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                        header::HeaderValue::from_str(&v)
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                    );
                }
            }

            let body = AxumBody::from(http_response.body);
            Ok(builder
                .body(body)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?)
        }

        Ok(Err(_)) => {
            // ws response handler dropped
            mark_tunnel_failure(state, id).await;
            state.responses.write().await.remove(&request_id);
            Err(StatusCode::BAD_GATEWAY)
        }

        Err(_) => {
            // timeout
            mark_tunnel_failure(state, id).await;
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
    is_sticky: bool,
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
    parts.uri = target_uri.clone(); // types are compatible in axum/hyper stack

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
            let mut axum_resp = AxumResponse::from_parts(parts, AxumBody::from(bytes));

            if !is_sticky {
                axum_resp.headers_mut().insert(
                    header::SET_COOKIE,
                    header::HeaderValue::from_static("backend=cloud; Path=/"),
                );
            }

            Ok(axum_resp)
        }
        Ok(Err(e)) => {
            println!("Cloud backend request failed: {}", e);
            Err(StatusCode::BAD_GATEWAY)
        }
        Err(_) => {
            println!("Cloud backend timed out");
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use hyper_util::rt::TokioExecutor;

    #[test]
    fn test_headers_to_map() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("X-Custom-Header", HeaderValue::from_static("some-value"));

        let result = headers_to_map(&headers);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&("content-type".to_string(), "application/json".to_string())));
        assert!(result.contains(&("x-custom-header".to_string(), "some-value".to_string())));
    }

    // Helper to create a mock AppState
    #[allow(dead_code)]
    async fn mock_app_state() -> AppState {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
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
        let redis_manager = crate::RedisManager { conn: redis_conn };

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
    async fn test_proxy_handler_with_tunnel() {
        let state = mock_app_state().await;
        let subdomain = "test-subdomain";
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        // Insert a mock tunnel
        state
            .tunnels
            .write()
            .await
            .insert(subdomain.to_string(), tx);

        let request = AxumRequest::builder()
            .uri("/")
            .header("Host", format!("{}.example.com", subdomain))
            .body(AxumBody::empty())
            .unwrap();

        // We expect the handler to forward to the tunnel, which will then wait for a response.
        // Since we don't send a response in this test, it will time out.
        // We just want to check that a message was sent to the tunnel.
        let result = proxy_handler(State(state.clone()), request).await;

        // The handler should return a Gateway Timeout because we don't send a response back
        assert_eq!(result.unwrap_err(), StatusCode::GATEWAY_TIMEOUT);

        // Check that a message was sent to the tunnel
        let received = rx.recv().await;
        assert!(received.is_some());
    }

    #[tokio::test]
    async fn test_proxy_handler_no_tunnel() {
        let state = mock_app_state().await;
        let subdomain = "unassigned-subdomain";

        let request = AxumRequest::builder()
            .uri("/")
            .header("Host", format!("{}.example.com", subdomain))
            .body(AxumBody::empty())
            .unwrap();

        // Expect the handler to forward to the cloud, which will fail in a test environment
        let result = proxy_handler(State(state.clone()), request).await;

        // We expect a 502 Bad Gateway or 504 Gateway Timeout because the cloud backend is not running
        let status = result.unwrap_err();
        assert!(status == StatusCode::BAD_GATEWAY || status == StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn test_forward_via_tunnel() {
        let state = mock_app_state().await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        let request = AxumRequest::builder()
            .uri("/test-path")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(AxumBody::from(r#"{"key":"value"}"#))
            .unwrap();

        let state_clone = state.clone();
        // Spawn a task to simulate the tunnel client receiving the request and sending a response
        tokio::spawn(async move {
            let received = rx.recv().await.unwrap();
            if let axum::extract::ws::Message::Text(text) = received {
                let msg: ControlMessage = serde_json::from_str(&text).unwrap();
                if let ControlMessage::Request { request_id, http } = msg {
                    assert_eq!(http.method, "POST");
                    assert_eq!(http.path, "/test-path");
                    assert_eq!(http.body, r#"{"key":"value"}"#.as_bytes());

                    // Simulate a response
                    let http_response = HttpResponse {
                        status: 200,
                        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                        body: b"response body".to_vec(),
                    };

                    // Get the response sender and send the.
                    let resp_tx = state_clone
                        .responses
                        .write()
                        .await
                        .remove(&request_id)
                        .unwrap();
                    resp_tx.send(http_response).unwrap();
                }
            }
        });

        let result = forward_via_tunnel("test", request, tx, &state).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "text/plain"
        );

        let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body_bytes, "response body");
    }

    #[tokio::test]
    async fn test_choose_best_tunnel_logic() {
        let state = mock_app_state().await;
        let subdomain = "app";
        let (tx, _rx) = tokio::sync::mpsc::channel(1);

        // Insert mock tunnels
        state
            .tunnels
            .write()
            .await
            .insert("app-01".to_string(), tx.clone());
        state
            .tunnels
            .write()
            .await
            .insert("app-02".to_string(), tx.clone());
        state
            .tunnels
            .write()
            .await
            .insert("app-03".to_string(), tx.clone());
        state
            .tunnels
            .write()
            .await
            .insert("app-warning".to_string(), tx.clone());
        state
            .tunnels
            .write()
            .await
            .insert("other-svc".to_string(), tx.clone());

        // Insert mock health data
        let mut health_data = state.health_data.write().await;

        // Good: Normal, low latency, low error rate
        health_data.insert(
            "app-01".to_string(),
            TunnelHealth {
                status: HealthStatus::Normal,
                avg_latency_ms: Some(50.0),
                error_rate: 0.1,
                ..Default::default()
            },
        );

        // Bad: Critical status
        health_data.insert(
            "app-02".to_string(),
            TunnelHealth {
                status: HealthStatus::Critical,
                ..Default::default()
            },
        );

        // OK: Normal, but higher latency and error rate
        health_data.insert(
            "app-03".to_string(),
            TunnelHealth {
                status: HealthStatus::Normal,
                avg_latency_ms: Some(200.0),
                error_rate: 0.2,
                ..Default::default()
            },
        );

        // Warning status
        health_data.insert(
            "app-warning".to_string(),
            TunnelHealth {
                status: HealthStatus::Warning,
                avg_latency_ms: Some(100.0),
                error_rate: 0.5,
                ..Default::default()
            },
        );

        // First choice should be the best "Normal" tunnel
        let result = choose_best_tunnel(&state, subdomain).await;
        assert!(result.is_some(), "Should have chosen a tunnel");
        let (chosen_id, _) = result.unwrap();
        assert_eq!(
            chosen_id, "app-01",
            "Should pick the best Normal tunnel (app-01)"
        );

        // If the best one becomes unavailable, it should pick the next best "Normal"
        state.tunnels.write().await.remove("app-01");
        let result2 = choose_best_tunnel(&state, subdomain).await;
        assert!(
            result2.is_some(),
            "Should have chosen a tunnel after removing the best"
        );
        let (chosen_id2, _) = result2.unwrap();
        assert_eq!(
            chosen_id2, "app-03",
            "Should pick the next best Normal tunnel (app-03)"
        );

        // If all Normal tunnels are gone, it should pick the Warning one
        state.tunnels.write().await.remove("app-03");
        let result3 = choose_best_tunnel(&state, subdomain).await;
        assert!(
            result3.is_some(),
            "Should have chosen a tunnel after removing all Normal"
        );
        let (chosen_id3, _) = result3.unwrap();
        assert_eq!(
            chosen_id3, "app-warning",
            "Should pick the Warning tunnel as a last resort"
        );

        // It should NEVER pick the Critical tunnel
        state.tunnels.write().await.remove("app-warning");
        let result4 = choose_best_tunnel(&state, subdomain).await;
        assert!(result4.is_none(), "Should not pick a Critical tunnel");
    }

    #[tokio::test]
    async fn test_health_transitions() {
        let state = mock_app_state().await;
        let tunnel_id = "test-tunnel";

        // Initial state is Normal (implicitly)

        // A single failure should transition to Warning, assuming hysteresis period has passed
        tokio::time::sleep(HYSTERESIS_DURATION).await;
        mark_tunnel_failure(&state, tunnel_id).await;
        {
            let health_data = state.health_data.read().await;
            let health = health_data.get(tunnel_id).unwrap();
            assert!(
                matches!(health.status, HealthStatus::Warning),
                "First failure should trigger Warning status"
            );
            assert_eq!(health.consecutive_failures, 1);
        }

        // Reach critical failure count
        tokio::time::sleep(HYSTERESIS_DURATION).await;
        for _ in 1..WARNING_TO_CRITICAL_FAILURES {
            mark_tunnel_failure(&state, tunnel_id).await;
        }

        // Status should still be Warning before the final push
        {
            let health_data = state.health_data.read().await;
            let health = health_data.get(tunnel_id).unwrap();
            assert!(
                matches!(health.status, HealthStatus::Warning),
                "Should remain Warning until final failure"
            );
            assert_eq!(health.consecutive_failures, WARNING_TO_CRITICAL_FAILURES);
        }

        // The next failure should transition to Critical
        tokio::time::sleep(HYSTERESIS_DURATION).await;
        mark_tunnel_failure(&state, tunnel_id).await;
        {
            let health_data = state.health_data.read().await;
            let health = health_data.get(tunnel_id).unwrap();
            assert!(
                matches!(health.status, HealthStatus::Critical),
                "Exceeding failure threshold should trigger Critical status"
            );
        }

        // A single success on a Critical tunnel should NOT change its status (based on current code)
        tokio::time::sleep(HYSTERESIS_DURATION).await;
        mark_tunnel_success(&state, tunnel_id, Some(50.0)).await;
        {
            let health_data = state.health_data.read().await;
            let health = health_data.get(tunnel_id).unwrap();
            assert!(
                matches!(health.status, HealthStatus::Critical),
                "A Critical tunnel should not recover to Warning on a single success"
            );
        }

        // Manually set back to Warning to test recovery to Normal
        {
            let mut health_data = state.health_data.write().await;
            let health = health_data.get_mut(tunnel_id).unwrap();
            health.status = HealthStatus::Warning;
        }

        tokio::time::sleep(HYSTERESIS_DURATION).await;
        for _ in 0..SUCCESS_TO_NORMAL {
            mark_tunnel_success(&state, tunnel_id, Some(50.0)).await;
        }
        {
            let health_data = state.health_data.read().await;
            let health = health_data.get(tunnel_id).unwrap();
            assert!(
                matches!(health.status, HealthStatus::Normal),
                "Should recover to Normal after enough successes"
            );
            assert_eq!(health.consecutive_failures, 0);
            assert_eq!(health.consecutive_successes, SUCCESS_TO_NORMAL);
        }
    }
}
