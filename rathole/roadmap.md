# Technical Implementation Roadmap for `traffic-switch-proxy`

This document provides a detailed, technical breakdown for implementing the `traffic-switch-proxy` by adapting the existing `rathole` codebase.

---

## Phase 1: Core Integration and Protocol Extension

**Goal:** Integrate `axum` to handle HTTP services and extend `rathole`'s protocol. This phase focuses on establishing a control and data plane over a single WebSocket connection.

**Note:** This approach avoids multiplexing (`yamux`) for simplicity. As a result, each tunnel will only be able to process one HTTP request at a time (serially).

### Technical Tasks:

1.  **Update Dependencies (`Cargo.toml`):**
    *   Ensure `axum` with `ws` and `macros` features is present.
    *   Ensure `hyper` with the `full` feature is present.

2.  **Refactor Server Entrypoint (`main.rs`):**
    *   Modify `main()` to launch a new `axum`-based server. The existing `legacy_main` can be used as a reference for the raw TCP/UDP server logic, which can be run separately or integrated if desired.
    *   The `axum` server will be the primary entry point for this new functionality.

3.  **Implement `axum` Server and WebSocket Handler (`server.rs` or `main.rs`):**
    *   Create a new `async fn run_http_server()` that sets up an `axum::Router`.
    *   Define a route `/register/:id` that points to a WebSocket handler function `ws_handler`.
    *   `ws_handler` will use `axum::extract::ws::WebSocketUpgrade` to upgrade the HTTP connection to a WebSocket.

4.  **Implement `ControlMessage` Handling:**
    *   **Server-side (`ws_handler`):**
        *   On upgrade, the handler will manage the `WebSocket` stream.
        *   It will read incoming `axum::ws::Message::Text` messages.
        *   Each message will be deserialized from JSON into a `ControlMessage`.
        *   A `TunnelMap` will be used to store a `mpsc::Sender<Message>` for each connected client, allowing other parts of the application (like the `proxy_handler`) to send messages to the client.
    *   **Client-side (`client.rs`):**
        *   The client will connect to the WebSocket endpoint.
        *   It will listen for incoming `Message::Text` and deserialize them into `ControlMessage`s.
        *   It will use the WebSocket's `Sink` to send JSON-serialized `ControlMessage`s to the server.

---

## Phase 2: Implement Traffic Switching and Client Logic

**Goal:** Implement the dynamic traffic switching logic in the server and update the client to act as an HTTP proxy for requests received from the relay.

### Technical Tasks:

1.  **Integrate `hyper-reverse-proxy` (`server.rs`):**
    *   Add the `hyper-reverse-proxy` dependency to `Cargo.toml`.
    *   In `run_http_server`, initialize `ReverseProxy::new(hyper::Client::new())` and share it as `axum` state.

2.  **Implement `axum` Fallback Handler (`server.rs`):**
    *   Add a `.fallback(proxy_handler)` to the `axum::Router`.
    *   The `proxy_handler` function will receive the `axum::http::Request`.
    *   It will extract the `Host` header to determine the target service subdomain.

3.  **Implement Traffic Switching Logic (`proxy_handler`):**
    *   Look up the service's tunnel status in the state manager (in-memory `HashMap` for now).
    *   **If tunnel is healthy:**
        1.  Serialize the `hyper::Request` into a `ControlMessage::Request`.
        2.  Generate a unique `request_id`.
        3.  Use a `ResponseMap` (e.g., `HashMap<String, oneshot::Sender<HttpResponse>>`) to store a sender for the response.
        4.  Send the `ControlMessage::Request` to the client via the `mpsc::Sender` stored in the `TunnelMap`.
        5.  `await` the response from the `oneshot::Receiver`.
        6.  Construct and return a `hyper::Response` from the received `HttpResponse`.
    *   **If tunnel is unhealthy:**
        1.  Use the `hyper-reverse-proxy` client to forward the request to the service's configured `fallback_url`.
        2.  Return the response from the reverse proxy directly.

4.  **Implement Client-side HTTP Proxying (`client.rs`):**
    *   When the client's WebSocket loop receives a `ControlMessage::Request`:
        1.  Initialize a `hyper::Client`.
        2.  Construct a new `hyper::Request` from the fields in the message.
        3.  Send the request to the local backend address (e.g., `http://127.0.0.1:8000`).
        4.  Await the `hyper::Response` from the local server.
        5.  Read the response body and headers.
        6.  Construct a `ControlMessage::Response` with the matching `request_id` and send it back to the server over the WebSocket connection.

---

## Phase 3: State Management and Authentication

**Goal:** Replace in-memory state with Redis for scalability and implement JWT-based authentication.

### Technical Tasks:

1.  **Integrate `redis` (`server.rs`):**
    *   Add the `redis` crate with the `tokio-comp` feature to `Cargo.toml`.
    *   Create a `RedisManager` struct to encapsulate the Redis client and connection logic.
    *   Share an `Arc<RedisManager>` as `axum` state.
    *   In the `ws_handler`, upon successful authentication, store the client's tunnel state (e.g., `subdomain`, `health_status`, `last_heartbeat`) in Redis. A `HASH` is a suitable data structure, keyed by subdomain.
    *   In the `proxy_handler`, query Redis instead of the in-memory map to decide on traffic switching.
    *   When a WebSocket connection is terminated, remove the corresponding state from Redis.

2.  **Implement JWT Authentication (`server.rs` and `client.rs`):**
    *   **Define Claims:** In `protocol.rs`, define a `Claims` struct containing `sub: String` (for the service name) and `exp: usize` (expiration).
    *   **Client-side:** The `token` in `config.toml` will now be the full JWT string. The client will send this token in the `ControlMessage::Register`.
    *   **Server-side (`ws_handler`):**
        1.  When a `ControlMessage::Register` message is received, use `jsonwebtoken::decode` to validate its `token` field.
        2.  The `DecodingKey` will be created from the `jwt_secret` in the server config.
        3.  The `Validation` struct should be configured to check the expiration time.
        4.  On successful validation, use the `sub` claim from the token to authorize the registration.
        5.  If validation fails, send a message back to the client and close the connection.

---

## Phase 4: Production Readiness

**Goal:** Add configuration, robust error handling, health checks, and documentation.

### Technical Tasks:

1.  **Configuration (`config.rs`):**
    *   Add a `[redis]` section to `config.toml` for the connection URL.
    *   Add a `[jwt]` section for issuer (`iss`) and audience (`aud`) claims to be used in token validation.
    *   Ensure all magic numbers (timeouts, pool sizes) are moved to the configuration.

2.  **Error Handling:**
    *   Define custom error types using `thiserror` for different failure domains (e.g., `ProxyError`, `AuthError`).
    *   Implement `axum::response::IntoResponse` for these error types to ensure the `axum` server returns meaningful HTTP status codes (e.g., 502 for proxy errors, 401 for auth errors).

3.  **Logging and Tracing:**
    *   Generate a unique request ID for each incoming HTTP request to the `proxy_handler`.
    *   Use `tracing::Span` with the request ID to trace the entire lifecycle of a request as it's processed, sent to the client, and returned.
    *   Log all significant events, such as tunnel registration, traffic switching decisions, and errors, with structured fields.

4.  **Health Checks:**
    *   **Client-side:** In `client.rs`, create a background task that periodically sends a `ControlMessage::Health { ... }` over the WebSocket connection.
    *   **Server-side:** In the server's `ws_handler`, listen for `Health` messages and update the corresponding health status and `last_heartbeat` timestamp in Redis.
    *   Add a separate server task that periodically scans Redis for stale tunnels (i.e., where `now() - last_heartbeat > timeout`) and marks them as unhealthy.

5.  **Documentation:**
    *   Update `README.md` with instructions on how to generate and configure JWTs.
    *   Add documentation for setting up and configuring Redis.
    *   Provide clear examples for running an `Http` service.