# Implementing Burst Scaling with Connection Draining and Sticky Sessions

This guide provides a step-by-step plan for implementing a robust burst scaling strategy for the `rathole` proxy. The goal is to allow the proxy to automatically redirect traffic to a cloud backend when the local service is under heavy load, without dropping in-flight requests or breaking user sessions.

We will implement two main strategies:
1.  **Connection Draining**: For stateless applications, this ensures that ongoing requests are allowed to complete on the local service before new traffic is redirected.
2.  **Sticky Sessions**: For stateful applications, this ensures that a user's session remains on the same backend (local or cloud) where it was initiated.

---

## Step 1: Protocol Changes

The first step is to extend the communication protocol between the client and the server to include health metrics.

In `src/protocol.rs`, we will add a new variant to the `ControlMessage` enum:

```rust
// in src/protocol.rs

#[derive(Serialize, Deserialize, Debug)]
pub enum ControlMessage {
    // ... existing messages
    Request { /* ... */ },
    Response { /* ... */ },
    Register { /* ... */ },

    /// Sent by the client to the server to report its health status.
    HealthUpdate {
        /// A simple flag indicating if the client is considered overloaded.
        is_overloaded: bool,
        /// Optional: CPU usage (e.g., 0.8 for 80%).
        cpu_usage: Option<f32>,
        /// Optional: Memory usage (e.g., 0.9 for 90%).
        memory_usage: Option<f32>,
    },
}
```

This new `HealthUpdate` message will be sent periodically from the client to the server.

---

## Step 2: Client-side Implementation (Health Monitor)

The `rathole` client needs a background task that monitors system resources and sends the `HealthUpdate` message.

In `src/client.rs`, you can spawn a new asynchronous task when the client starts:

```rust
// in src/client.rs (pseudo-code)

pub async fn run_client(/* ... */) {
    // ... existing client connection logic ...

    // Spawn the health monitor task
    let tunnel_tx_clone = tunnel_tx.clone(); // The sender for the WebSocket tunnel
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;

            // In a real implementation, you would use a crate like `psutil`
            // or `sysinfo` to get actual CPU and memory usage.
            let is_overloaded = check_if_system_is_overloaded(); // Your custom logic
            
            let health_msg = ControlMessage::HealthUpdate {
                is_overloaded,
                cpu_usage: None, // Optional
                memory_usage: None, // Optional
            };

            let msg_str = serde_json::to_string(&health_msg).unwrap();
            if tunnel_tx_clone.send(Message::Text(msg_str)).await.is_err() {
                // The tunnel is closed, so we can stop the health monitor
                break;
            }
        }
    });

    // ... rest of the client logic ...
}

fn check_if_system_is_overloaded() -> bool {
    // TODO: Implement your system monitoring logic here.
    // This could involve checking CPU usage, memory, or other metrics.
    // For now, we can simulate it.
    false
}
```

---

## Step 3: Server-side Implementation (Tracking Health and Switching Logic)

This is the most involved part. We need to update the server to track the health of each tunnel and implement the smart routing logic.

### 3.1. Update `AppState`

In `src/proxy.rs`, we'll update `AppState` to store health data and the last used backend for each user (for sticky sessions).

```rust
// in src/proxy.rs

use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::RwLock;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::HeaderValue;

pub struct TunnelHealth {
    pub is_overloaded: bool,
    pub last_update: Instant,
}

#[derive(Clone)]
pub struct AppState {
    // ... existing fields
    pub tunnels: Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>,
    pub responses: Arc<RwLock<HashMap<String, oneshot::Sender<HttpResponse>>>>,
    pub jwt_secret: Arc<String>,
    pub redis: Arc<crate::RedisManager>,
    pub hyper_client: HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>,
    pub default_cloud_backend: String,
    pub request_timeout: Duration,

    // New fields for burst scaling
    pub health_data: Arc<RwLock<HashMap<String, TunnelHealth>>>,
}
```

### 3.2. Handle `HealthUpdate` Messages

In the server's WebSocket handler (`handle_socket` in `src/main.rs`), you need to process the `HealthUpdate` message from the client.

```rust
// in src/main.rs (pseudo-code for handle_socket)

async fn handle_socket(/* ... */) {
    // ... inside the message processing loop ...
    match msg {
        ControlMessage::HealthUpdate { is_overloaded, .. } => {
            let mut health_data = state.health_data.write().await;
            health_data.insert(id.clone(), TunnelHealth {
                is_overloaded,
                last_update: Instant::now(),
            });
        }
        // ... other message types ...
    }
}
```

### 3.3. Implement the Smart `proxy_handler`

Now, we'll update the `proxy_handler` in `src/proxy.rs` to implement the connection draining and sticky session logic.

```rust
// in src/proxy.rs (pseudo-code)

pub async fn proxy_handler(
    State(state): State<AppState>,
    mut axum_req: AxumRequest<AxumBody>,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    
    let subdomain = /* ... get subdomain from host header ... */;

    // --- Sticky Session Logic ---
    let cookies = axum_req.headers().get(COOKIE).and_then(|h| h.to_str().ok()).unwrap_or("");
    let backend_cookie = cookies.split(';').find(|c| c.trim().starts_with("backend="));

    if let Some(cookie) = backend_cookie {
        if cookie.contains("cloud") {
            return forward_to_cloud(axum_req, &state, true).await; // Forward to cloud and keep the cookie
        }
        // If backend=local, we continue with the health check logic
    }

    // --- Connection Draining Logic ---
    let is_tunnel_healthy = {
        let health_data = state.health_data.read().await;
        if let Some(health) = health_data.get(&subdomain) {
            !health.is_overloaded && health.last_update.elapsed() < Duration::from_secs(30)
        } else {
            true // No health data yet, assume healthy
        }
    };

    if is_tunnel_healthy {
        if let Some(tunnel_tx) = state.tunnels.read().await.get(&subdomain).cloned() {
            // Forward to local and set backend=local cookie
            let mut response = forward_via_tunnel(axum_req, tunnel_tx, &state).await?;
            response.headers_mut().insert(SET_COOKIE, HeaderValue::from_static("backend=local; Path=/"));
            return Ok(response);
        }
    }

    // If tunnel is not healthy, doesn't exist, or sticky session is for cloud
    forward_to_cloud(axum_req, &state, false).await
}

// You'll need to modify forward_to_cloud to handle the cookie
async fn forward_to_cloud(
    req: AxumRequest<AxumBody>,
    state: &AppState,
    is_sticky: bool,
) -> Result<AxumResponse<AxumBody>, StatusCode> {
    // ... existing forwarding logic ...
    
    // After getting the response from the cloud backend
    let mut response = /* ... axum_resp ... */;

    if !is_sticky { // Only set the cookie for new sessions
        response.headers_mut().insert(SET_COOKIE, HeaderValue::from_static("backend=cloud; Path=/"));
    }
    
    Ok(response)
}
```

---

## Summary of the New Request Flow

1.  A request comes into the `proxy_handler`.
2.  It checks for a `backend` cookie. If `backend=cloud`, it forwards to the cloud.
3.  If there's no cookie or `backend=local`, it checks the health of the tunnel for the requested subdomain.
4.  If the tunnel is healthy, it forwards the request to the local client and sets the `backend=local` cookie in the response.
5.  If the tunnel is overloaded or doesn't exist, it forwards the request to the cloud backend and sets the `backend=cloud` cookie in the response.

This guide provides a solid foundation for implementing the burst scaling feature. You can start by implementing the protocol changes and then move on to the client and server-side logic.
