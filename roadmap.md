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
