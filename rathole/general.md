# `traffic-switch-proxy` Technical Roadmap

## 1. Introduction

This document outlines the technical roadmap for implementing the `traffic-switch-proxy` project. The goal is to create a reverse proxy and tunneling system in Rust, allowing public HTTP traffic to be routed to a local developer machine.

The core technologies used will be:

- **Rust** as the programming language.
- **Tokio** as the asynchronous runtime.
- **Axum** as the web framework for handling HTTP and WebSocket connections.
- **`hyper-reverse-proxy`** as a key component for forwarding traffic, especially for failover scenarios.

This roadmap is divided into several phases, each representing a Minimum Viable Product (MVP) that builds upon the previous one.

## 2. Phase 1: Core Relay Server (MVP 1)

**Goal:** Implement the basic relay server with WebSocket tunnel registration and initial HTTP request forwarding.

**Key Features:**

- A WebSocket endpoint `/register/:id` where local clients can connect and register a tunnel.
- An in-memory `HashMap` to store and manage the state of registered tunnels.
- A fallback HTTP endpoint `/tunnel/:id/*path` that receives all incoming public traffic.
- Initial use of `hyper-reverse-proxy` to forward all incoming requests to a single, hardcoded backend URL. This simulates the "cloud fallback" functionality.

**Implementation Details:**

- Set up a basic Axum server with two routes.
- Implement the WebSocket handler (`/register/:id`) to accept connections and store a handle to the connection in the in-memory map.
- Implement the HTTP handler (`/tunnel/:id/*path`) that will use `hyper-reverse-proxy` to forward every request.
- Add basic logging using the `tracing` crate to observe server events.

**File Structure (Initial):**

- All the initial code will reside in `relay/src/main.rs`.

## 3. Phase 2: Dynamic Traffic Switching (MVP 2)

**Goal:** Introduce dynamic traffic switching. The relay will decide whether to forward a request to the cloud fallback (via `hyper-reverse-proxy`) or attempt to send it through the WebSocket tunnel.

**Key Features:**

- The HTTP handler will now look up the `tunnel_id` from the request path in the tunnel registry.
- **If a tunnel is registered and healthy:** The server will serialize the HTTP request into a `ControlMessage::Request` and send it over the corresponding WebSocket connection. (This phase focuses on the relay-side logic; the client is not yet built to handle this).
- **If a tunnel is not registered or is unhealthy:** The server will use `hyper-reverse-proxy` to forward the request to the configured cloud fallback URL, ensuring high availability.

**Implementation Details:**

- Define the `ControlMessage` enum in `shared/src/lib.rs` to standardize communication between the client and relay.
- The HTTP handler will be updated with the traffic switching logic.
- The concept of "health" will be introduced, initially just checking for the presence of an active WebSocket connection.

## 4. Phase 3: Local Tunnel Client (MVP 3)

**Goal:** Develop the local client that connects to the relay server and proxies requests to a local backend service.

**Key Features:**

- The client will establish a persistent WebSocket connection to the relay's `/register/:id` endpoint.
- It will listen for `ControlMessage::Request` messages from the relay.
- Upon receiving a request, it will deserialize it and use `hyper` to forward it to the user's local server (e.g., `http://127.0.0.1:8000`).
- It will capture the response from the local server, serialize it into a `ControlMessage::Response`, and send it back to the relay through the tunnel.

**Implementation Details:**

- The client will be a new binary crate in the `client/` directory.
- It will use `tokio-tungstenite` to manage the WebSocket connection.
- It will use `hyper` to act as an HTTP client to the local backend.

## 5. Phase 4: Enhancements and Production Readiness

**Goal:** Add critical features to make the system robust, secure, and configurable.

**Key Features:**

- **Authentication:** Secure the `/register` endpoint by requiring an API key or token.
- **Heartbeats & Health Checks:** The client will periodically send health metrics (e.g., CPU usage, latency) to the relay. The relay will use these metrics and a heartbeat mechanism to detect and gracefully handle dead or unhealthy tunnels.
- **Improved Error Handling:** Implement comprehensive error handling and logging throughout the client and relay.
- **Configuration:** Move hardcoded values (ports, fallback URLs, etc.) to a configuration file (e.g., `config.toml`) or environment variables.
- **Graceful Shutdown:** Ensure both the client and relay can shut down gracefully, closing connections and finishing in-flight requests.
- **TLS Security:** Add TLS support using `rustls` to secure all communication between the client, relay, and end-users.

## 6. Code Refactoring and Final Structure

As the project grows, the code will be refactored from a single `main.rs` file into a more modular structure as suggested in the project description:

- **`relay/src/main.rs`:** The main entry point, responsible for setting up the Axum router, initializing shared state, and tying all the components together.
- **`relay/src/ws.rs`:** Contains all the logic for handling WebSocket connections, including registration, authentication, and message handling.
- **`relay/src/proxy.rs`:** Contains the core reverse proxy and traffic switching logic, including the integration with `hyper-reverse-proxy` for failover.
- **`shared/src/lib.rs`:** Will contain all the shared data structures, primarily the `ControlMessage` enum, used for communication between the relay and client.

# 🧩 Project Description — `traffic-switch-proxy`

## 📘 Summary

`traffic-switch-proxy` is a reverse proxy and tunneling system that allows public HTTP traffic to be routed to a developer’s local machine by maintaining a persistent outbound tunnel between a local client and a remote relay server.

It supports traffic switching — dynamically redirecting incoming requests between:

- a local backend (developer’s local server), and
- a cloud backend (for failover or burst scaling).

This enables a hybrid local-cloud hosting setup where apps run locally by default but failover seamlessly to a remote instance.

## ⚙️ Architecture Overview

The system has two main parts:

1. **Proxy Relay Server (runs on your public VPS)**
    - Publicly accessible over HTTPS (e.g., `relay.yourdomain.run`)
    - Handles:
        - Assigning public URLs/subdomains to connected clients
        - Receiving incoming HTTP requests
        - Forwarding them through tunnels to the correct client
        - Optionally rerouting traffic to a cloud backend if the client is unavailable or under load

2. **Local Tunnel Client (runs on the user’s machine)**
    - Establishes an outbound persistent connection (WebSocket or TCP) to the relay
    - Forwards requests from the relay to a local server (like `http://127.0.0.1:8000`)
    - Sends the local server’s responses back to the relay through the same tunnel
    - Periodically sends health/status metrics (CPU, latency, etc.) for load monitoring

You are coding an Axum-based Rust server that acts as a small reverse-proxy with WebSocket tunnel registration.
The goal is to have two routes:

/register/:id — A WebSocket endpoint where clients connect and register a tunnel ID.

/tunnel/:id/*path — A reverse proxy route that forwards HTTP requests to the registered tunnel’s backend.

Use async Rust and hyper-reverse-proxy crate/project that is in github for forwarding, axum for routing and WebSockets, and tokio as the async runtime.

here is their exmaple of using the crate: "use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use hyper_reverse_proxy::ReverseProxy;
use hyper_trust_dns::{RustlsHttpsConnector, TrustDnsResolver};
use std::net::IpAddr;
use std::{convert::Infallible, net::SocketAddr};

lazy_static::lazy_static! {
    static ref  PROXY_CLIENT: ReverseProxy<RustlsHttpsConnector> = {
        ReverseProxy::new(
            hyper::Client::builder().build::<_, hyper::Body>(TrustDnsResolver::default().into_rustls_webpki_https_connector()),
        )
    };
}

fn debug_request(req: &Request<Body>) -> Result<Response<Body>, Infallible> {
    let body_str = format!("{:?}", req);
    Ok(Response::new(Body::from(body_str)))
}

async fn handle(client_ip: IpAddr, req: Request<Body>) -> Result<Response<Body>, Infallible> {
    if req.uri().path().starts_with("/target/first") {
        match PROXY_CLIENT.call(client_ip, "<http://127.0.0.1:13901>", req)
            .await
        {
            Ok(response) => {
                Ok(response)
            },
            Err(_error) => {
                Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap())},
        }
    } else if req.uri().path().starts_with("/target/second") {
        match PROXY_CLIENT.call(client_ip, "<http://127.0.0.1:13902>", req)
            .await
        {
            Ok(response) => Ok(response),
            Err(_error) => Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()),
        }
    } else {
        debug_request(&req)
    }
}

# [tokio::main]

async fn main() {
    let bind_addr = "127.0.0.1:8000";
    let addr: SocketAddr = bind_addr.parse().expect("Could not parse ip:port.");

    let make_svc = make_service_fn(|conn: &AddrStream| {
        let remote_addr = conn.remote_addr().ip();
        async move { Ok::<_, Infallible>(service_fn(move |req| handle(remote_addr, req))) }
    });

    let server = Server::bind(&addr).serve(make_svc);

    println!("Running server on {:?}", addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}"

Dependencies to include in Cargo.toml:

[dependencies]
axum = { version = "0.7", features = ["ws", "macros"] }
tokio = { version = "1", features = ["full"] }
hyper-reverse-proxy = "0.5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
hyper-trust-dns = { version = "?", features = [
  "rustls-http2",
  "dnssec-ring",
  "dns-over-https-rustls",
  "rustls-webpki",
  "https-only"
] }
futures = "0.3"

You can assume a simple Cargo.toml with these crates.
Use tokio::main as the entry point.

you can assume this structure for future scaling:

code/
├── main.rs         # Entry point, route setup, shared state
├── ws.rs           # WebSocket tunnel registration logic
└── proxy.rs        # Reverse proxy logic

Here’s the high-level pseudocode to implement:

state = HashMap<String, TunnelConnection>

on_websocket_connection(tunnel_id, socket):
    store tunnel_id -> socket_sender
    keep listening for control messages
    on disconnect, remove tunnel_id

on_http_request(tunnel_id, path, request):
    lookup tunnel_id in state
    if missing: 404
    forward request using hyper_reverse_proxy to backend (e.g. localhost:8000)
    return response

main:
    start axum app
    routes:
        /register/:id -> websocket_handler
        /tunnel/:id/*path -> proxy_handler

The WebSocket doesn’t need to send real data — it just simulates registration.

After generating the code, list improvements needed for an MVP-level version, such as:

authentication for /register
heartbeats to remove dead tunnels
error handling and logging
config support (PORT, base backend URL)
graceful shutdown
