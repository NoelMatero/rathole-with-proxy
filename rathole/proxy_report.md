# Proxy Logic Integration Report

## 1. Analysis of `src/proxy.rs`

I began by analyzing the provided file, `/home/noelmatero/trafficswitchproxy/rathole/src/proxy.rs`. My analysis concluded the following:

*   The file represents a complete, standalone `axum` server, not a library module.
*   It contains a fully-featured implementation of a reverse proxy with traffic switching capabilities.
*   It uses an in-memory `HashMap` protected by an `Arc<Mutex<...>>` to manage the state of active tunnels.
*   It uses `hyper-reverse-proxy` to forward traffic to a fallback URL if a tunnel is not available for a given subdomain.
*   The communication protocol is based on `ControlMessage`s serialized as JSON over a WebSocket, which is consistent with the project's direction.

## 2. Integration Strategy

Rather than replacing the existing `src/main.rs` (which contains more robust features like Redis integration and a reliable cleanup mechanism), I chose to **integrate the core traffic-switching logic from `proxy.rs` into `main.rs`**.

This approach preserves the progress made in Phase 3 (State Management & Authentication) while incorporating the key feature of Phase 2 (Traffic Switching).

## 3. Implementation Details

The following changes were made to `src/main.rs`:

1.  **Added Dependency:** The `hyper-reverse-proxy` crate was added to `Cargo.toml`.

2.  **Updated Router:** The `axum` router was modified to use the new `tunnel` function as a `.fallback()` handler. This means any request that doesn't match `/login` or `/register/:id` will be sent to this handler for processing.

3.  **Implemented Traffic Switching in `tunnel` handler:** The `tunnel` function now contains the core traffic switching logic:
    *   It inspects the `Host` header of the incoming HTTP request to extract the subdomain.
    *   It performs a lookup in the in-memory `TunnelMap` using the subdomain as the key.
    *   **If a tunnel is found:** The request is serialized into a `ControlMessage::Request` and sent to the connected client through the WebSocket tunnel, as it was before.
    *   **If no tunnel is found:** The new fallback logic is executed. It uses `hyper_reverse_proxy::call` to forward the original HTTP request to a hardcoded fallback URL (`http://httpbin.org/anything`).

## 4. New Request Flow

The server now supports two distinct request paths:

*   **Tunneled Path:**
    1.  A request arrives at `http://{subdomain}.your-app.com`.
    2.  The `tunnel` handler extracts `{subdomain}`.
    3.  A matching tunnel is found in the `TunnelMap`.
    4.  The request is sent to the connected client via WebSocket.
    5.  The client proxies the request to the local service.
    6.  The response flows back through the tunnel to the server and then to the original user.

*   **Fallback Path:**
    1.  A request arrives at `http://{subdomain}.your-app.com`.
    2.  The `tunnel` handler extracts `{subdomain}`.
    3.  No matching tunnel is found.
    4.  The request is forwarded to the fallback URL using `hyper-reverse-proxy`.
    5.  The response from the fallback service is streamed back to the original user.

This completes the implementation of the core requirements for Phase 2.
