# 🧩 Project Description — `traffic-switch-proxy`

## 📘 Summary

`traffic-switch-proxy` is a reverse proxy and tunneling system that allows public HTTP traffic to be routed to a developer’s local machine by maintaining a persistent outbound tunnel between a local client and a remote relay server.

It supports traffic switching — dynamically redirecting incoming requests between:

* a local backend (developer’s local server), and
* a cloud backend (for failover or burst scaling).

This enables a hybrid local-cloud hosting setup where apps run locally by default but failover seamlessly to a remote instance.

## ⚙️ Architecture Overview

The system has two main parts:

1. **Proxy Relay Server (runs on your public VPS)**
    * Publicly accessible over HTTPS (e.g., `relay.yourdomain.run`)
    * Handles:
        * Assigning public URLs/subdomains to connected clients
        * Receiving incoming HTTP requests
        * Forwarding them through tunnels to the correct client
        * Optionally rerouting traffic to a cloud backend if the client is unavailable or under load

2. **Local Tunnel Client (runs on the user’s machine)**
    * Establishes an outbound persistent connection (WebSocket or TCP) to the relay
    * Forwards requests from the relay to a local server (like `http://127.0.0.1:8000`)
    * Sends the local server’s responses back to the relay through the same tunnel
    * Periodically sends health/status metrics (CPU, latency, etc.) for load monitoring

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
