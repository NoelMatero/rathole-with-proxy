Absolutely — here’s a **clear, detailed, AI-friendly description** you can drop into your GitHub README, your `traffic-switch-proxy` repo description, or even as a system prompt when scaffolding code.

It explains *exactly* what this proxy is supposed to do, what components it includes, and how it should behave — written for both humans **and** AI-assisted code generation.

---

## 🧩 Project Description — `traffic-switch-proxy`

### 📘 Summary

`traffic-switch-proxy` is a **reverse proxy and tunneling system** that allows public HTTP traffic to be routed to a developer’s **local machine** by maintaining a **persistent outbound tunnel** between a local client and a remote relay server.

It also supports **traffic switching** — the ability to dynamically redirect incoming requests between multiple backends, such as:

* the developer’s **local server**, and
* a **remote cloud instance** (for failover or load-based bursting).

This project forms the foundation of a **hybrid local-cloud hosting system**, where apps run locally by default and automatically switch to the cloud when needed.

---

### ⚙️ Architecture Overview

The system has two main parts:

1. **Proxy Relay Server (runs on your public VPS)**

   * Publicly accessible over HTTPS (e.g., `relay.yourdomain.run`)
   * Handles:

     * Assigning public URLs/subdomains to connected clients
     * Receiving incoming HTTP requests
     * Forwarding them through tunnels to the correct client
     * Optionally rerouting traffic to a cloud backend if the client is unavailable or under load

2. **Local Tunnel Client (runs on the user’s machine)**

   * Establishes an **outbound persistent connection** (WebSocket or TCP) to the relay
   * Forwards requests from the relay to a **local server** (like `http://127.0.0.1:8000`)
   * Sends the local server’s responses back to the relay through the same tunnel
   * Periodically sends health/status metrics (CPU, latency, etc.) for load monitoring

Together, these components make it possible for a user to expose a local development or production server to the internet **without port forwarding**, and to have the proxy automatically **switch traffic** to a backup (cloud) server if the local server is overloaded or offline.

---

### 🔄 Traffic Flow

1. The user starts the CLI:

   ```bash
   traffic-switch-cli connect --target http://127.0.0.1:8000
   ```

   The CLI:

   * Opens a WebSocket connection to the relay.
   * Authenticates with an API token.
   * Receives a public URL such as `https://user123.traffic-switch.run`.

2. When someone visits that URL:

   * The relay receives the request.
   * It looks up which tunnel corresponds to `user123.traffic-switch.run`.
   * It forwards the request data through the tunnel to the user’s local client.
   * The client proxies it to the local target (`127.0.0.1:8000`), captures the response, and streams it back through the tunnel.

3. If the local tunnel disconnects or reports high load:

   * The relay automatically **switches** traffic to the user’s configured **cloud backup endpoint**, maintaining uptime.

---

### 🧠 Key Concepts

| Concept                      | Description                                                                                                                                                |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tunnel**                   | A persistent WebSocket or TCP connection from the local client to the relay. Used for bidirectional HTTP message forwarding.                               |
| **Subdomain mapping**        | Each connected client is assigned a public subdomain (e.g., `xyz.traffic-switch.run`). The relay maps requests for this subdomain to that client’s tunnel. |
| **Switching**                | The relay can dynamically route traffic between multiple destinations (local tunnel, cloud backend) based on availability or performance.                  |
| **Control plane (optional)** | A lightweight orchestrator service that monitors client health, manages credentials, and triggers bursting/failover actions.                               |

---

### 🧱 Minimal Functional Requirements

**Relay Server**

* Accepts WebSocket connections from clients.
* Assigns each client a unique public URL/subdomain.
* Keeps track of active tunnels in memory.
* Forwards HTTP requests to the correct tunnel.
* Forwards responses back to the original HTTP requester.
* Supports configurable fallback URL (for failover/burst mode).

**Client Agent**

* Connects to the relay using a persistent WebSocket connection.
* For each incoming request:

  * Forwards it to the local server.
  * Streams the response back to the relay.
* Sends heartbeat pings and basic health metrics.

**Optional (advanced):**

* Multiplex multiple concurrent HTTP requests over a single tunnel.
* Authenticate using API tokens or signed keys.
* Use TLS for secure communication (via Rustls or Caddy proxy).

---

### 🔧 Technologies (Recommended)

* **Language:** Rust
* **Networking:** `tokio`, `axum`, `tokio-tungstenite`, `hyper`
* **Async runtime:** Tokio
* **State management:** in-memory `HashMap` for tunnel registry
* **Serialization:** `serde_json` for message framing over WebSocket
* **Optional scaling:** Redis or PostgreSQL to persist active tunnels

---

### 🧩 Example Components

```
traffic-switch-proxy/
├── relay/                 # Proxy relay (server)
│   ├── src/main.rs
│   └── Cargo.toml
├── client/                # Local tunnel client
│   ├── src/main.rs
│   └── Cargo.toml
├── shared/                # Shared message definitions & protocol structs
│   └── src/lib.rs
└── README.md
```

---

### 💬 Example Description (for GitHub)

> **traffic-switch-proxy** is a lightweight Rust-based reverse proxy and tunneling system that lets you expose your local server to the public internet and automatically switch traffic between your local and cloud environments.
>
> It’s designed for developers who want to host apps locally by default but still handle bursts of cloud traffic seamlessly — combining the convenience of ngrok with the reliability of auto-failover proxies.

🧩 Project Description — traffic-switch-proxy
📘 Summary

traffic-switch-proxy is a reverse proxy and tunneling system that allows public HTTP traffic to be routed to a developer’s local machine by maintaining a persistent outbound tunnel between a local client and a remote relay server.

It supports traffic switching — dynamically redirecting incoming requests between:

a local backend (developer’s local server), and

a cloud backend (for failover or burst scaling).

This enables a hybrid local-cloud hosting setup where apps run locally by default but failover seamlessly to a remote instance.

⚙️ Architecture Overview
Components

Relay Server (runs publicly)

Receives incoming HTTP requests

Maps them to active tunnels (WebSocket connections)

Forwards the HTTP payload to the correct local client

Supports cloud fallback routing if the local client is unavailable

Local Client (runs on the developer’s machine)

Maintains a persistent WebSocket connection to the relay

Proxies HTTP requests to a local backend (http://127.0.0.1:8000)

Sends back the response over the same connection

Periodically reports system metrics (CPU, latency, etc.)

🔄 Traffic Flow

The user runs:

    traffic-switch-cli connect --target http://127.0.0.1:8000

    The client:

        Authenticates to the relay

        Establishes a WebSocket tunnel

        Receives a public URL (e.g. https://user123.traffic-switch.run)

    When an external user sends a request to that URL:

        The relay matches user123 → client tunnel

        Streams the request over WebSocket to the client

        The client forwards it to the local backend

        Response flows back through the tunnel to the original requester

    If the client disconnects or reports high load:

        Relay reroutes requests to a configured cloud fallback URL

🧠 Key Concepts
Concept	Description
Tunnel	Persistent WebSocket or TCP connection carrying bidirectional HTTP traffic.
Subdomain mapping	Maps each connected client to a public URL like xyz.traffic-switch.run.
Switching	Relay dynamically routes traffic between local tunnel and fallback cloud endpoint.
Health metrics	Periodic pings from the client with load data to help relay decide when to switch traffic.
🧱 Minimal Functional Requirements
Relay Server

    Accept WebSocket client connections

    Assign subdomains to clients

    Maintain tunnel registry (client_id → sender/receiver)

    Accept incoming HTTP requests, map by subdomain

    Serialize request → send via WebSocket

    Deserialize and forward response

    Support cloud fallback endpoint

Client Agent

    Connect to relay and authenticate

    Listen for incoming proxied HTTP requests

    Proxy requests to --target URL (local backend)

    Stream response data back

    Send periodic health/heartbeat messages

Optional

    Multiplex concurrent requests over one tunnel

    TLS (via Rustls)

    Authentication (signed token or HMAC)

🔧 Technologies (Recommended)
Category	Technology
Language	Rust
Runtime	Tokio
Networking	hyper, tokio-tungstenite, axum
Serialization	serde, serde_json
TLS	rustls
State	In-memory HashMap or Redis for multi-instance relay
🧩 Directory Layout

traffic-switch-proxy/
├── relay/
│   ├── src/main.rs
│   └── Cargo.toml
├── client/
│   ├── src/main.rs
│   └── Cargo.toml
├── shared/
│   └── src/lib.rs
└── README.md

🧠 Technical Implementation Guide (for AI-assisted coding)

This section explicitly defines how each component should be implemented — suitable for automated code generation or pair-programming with an LLM.
1. Shared Protocol Layer (shared/)
Purpose

Defines all message types, serialization formats, and protocol behavior between relay and client.
Types (in Rust-like pseudocode)

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlMessage {
    Register { api_key: String, target_subdomain: String },
    Request { request_id: String, method: String, path: String, headers: Vec<(String, String)>, body: Vec<u8> },
    Response { request_id: String, status: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    Health { cpu_usage: f32, latency_ms: u32 },
    Pong,
}

Notes

    Use serde_json for message framing

    Messages are newline-delimited JSON objects

    Each message includes a request_id for pairing requests/responses

2. Relay Server (relay/)
Primary Components

    TunnelRegistry:
    In-memory map:

    HashMap<String, TunnelHandle> // subdomain -> client tunnel

    where TunnelHandle holds:

        WebSocket Sender<ControlMessage>

        Last health timestamp

        Optional fallback URL

    HTTP frontend:

        Uses axum or hyper::Server

        On incoming request:

            Extracts host/subdomain

            Serializes HTTP request into ControlMessage::Request

            Sends through tunnel

            Awaits ControlMessage::Response

            Writes it back to client via axum::Response

    WebSocket handler:

        /connect endpoint for clients

        Authenticates token

        Registers subdomain

        Starts message forwarding loop (read/write)

Example Relay Startup

#[tokio::main]
async fn main() {
    let registry = TunnelRegistry::default();

    let app = Router::new()
        .route("/connect", post(ws_handler))
        .fallback_service(service_fn(|req| handle_public_request(req, registry.clone())));

    axum::Server::bind(&"0.0.0.0:443".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

3. Local Client (client/)
Responsibilities

    Maintain persistent WebSocket connection to relay

    On receiving Request message:

        Convert to hyper::Request

        Forward to target (local backend)

        Await response

        Send Response message back

    Periodically send Health pings

Example Client Loop

async fn handle_messages(mut ws: WebSocketStream<MaybeTlsStream<TcpStream>>) {
    while let Some(msg) = ws.next().await {
        let data = msg?.into_text()?;
        let msg: ControlMessage = serde_json::from_str(&data)?;

        match msg {
            ControlMessage::Request { request_id, .. } => {
                let response = proxy_to_local_backend(&msg).await;
                ws.send(Message::Text(serde_json::to_string(&response)?)).await?;
            }
            _ => {}
        }
    }
}

4. Switching & Health Logic
Relay-side

    Track health updates per tunnel.

    If tunnel silent for >5s or high CPU reported:

        Route future requests to fallback backend:

        if registry.get_health(subdomain).is_unhealthy() {
            forward_to_cloud(subdomain, req).await
        }

Client-side

    Send periodic health:

    ControlMessage::Health { cpu_usage, latency_ms }

5. CLI (traffic-switch-cli)

Optional convenience tool wrapping the client binary:

traffic-switch-cli connect --target http://127.0.0.1:8000 --api-key <token>

    Handles auth token

    Launches client process

    Displays public URL

🧱 Example High-Level Flow

[HTTP User] → [Relay (public HTTPS)] → [Tunnel over WS] → [Local Client] → [127.0.0.1:8000]
                              ↑
                              └── fallback → [Cloud Server]

