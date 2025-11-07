# `traffic-switch-proxy` Tunnel: Technical Plan & Roadmap

## 1. Introduction

This document provides a technical plan and roadmap for adapting the `rathole` project to create `traffic-switch-proxy`. The goal is to leverage `rathole`'s robust and performant foundation while implementing the specific features required by `traffic-switch-proxy`, such as dynamic traffic switching and an HTTP-centric tunneling protocol.

This plan is based on a critical analysis of the `traffic-switch-proxy` project description (`general.md` and `project.md`) and the `rathole` source code.

## 2. Technical Description

The core of the `traffic-switch-proxy` will be a modified version of `rathole`. We will introduce a new `ServiceType` called `Http`, which will use a new protocol based on the `ControlMessage` enum described in `project.md`. This will allow us to keep `rathole`'s existing capabilities for forwarding raw TCP and UDP traffic, making the final product more versatile.

The communication between the client and server for `Http` services will be done over a WebSocket connection. The server will expose a RESTful-like API for tunnel registration, and it will use `hyper-reverse-proxy` for the cloud fallback mechanism.

## 3. Critical Analysis & Proposed Improvements

While the `traffic-switch-proxy` design is sound, there are several areas where we can make improvements to enhance performance, scalability, and security.

### 3.1. Protocol: Extension over Replacement

Instead of completely replacing `rathole`'s protocol, we will extend it. We will add a new `ServiceType::Http` and a corresponding protocol based on the `ControlMessage` enum. This has several advantages:

* **Preserves Existing Functionality:** We retain `rathole`'s ability to forward raw TCP and UDP traffic.
* **Reduces Development Time:** We only need to implement the new protocol, not re-implement the entire communication stack.
* **Increased Versatility:** The final product will be a more general-purpose tunneling tool.

### 3.2. Multiplexing for Performance

The `project.md` mentions multiplexing, which is critical for handling multiple concurrent HTTP requests over a single tunnel. `rathole`'s current control channel is not multiplexed. We propose integrating the `yamux` crate to provide multiplexing over the WebSocket connection. `yamux` is a battle-tested multiplexing library that is used in projects like `libp2p`.

### 3.3. Scalable State Management

The `project.md` suggests an in-memory `HashMap` for storing tunnel state. This is a single point of failure and does not scale to multiple relay server instances. We propose using **Redis** for state management. This will allow us to run multiple instances of the relay server for high availability and load balancing.

### 3.4. Robust Authentication with JWT

The `project.md` mentions authentication as a future enhancement. We propose implementing authentication from the start using **JSON Web Tokens (JWTs)**. The client will send a JWT in the `Authorization` header when establishing the WebSocket connection. The server will validate the JWT and use the claims to identify the client and their permissions.

## 4. Implementation Roadmap

This roadmap is divided into four phases, each building upon the previous one.

### Phase 1: Core Integration and Protocol Extension

**Goal:** Integrate `axum` and `yamux`, and extend `rathole`'s protocol with the new `Http` service type.

**Tasks:**

1. **Integrate `axum`:** Add `axum` as a dependency and set up a basic web server in `server.rs`.
2. **Implement `/register/:id` API:** Create a WebSocket handler that will be responsible for authentication and tunnel registration.
3. **Integrate `yamux`:** Integrate `yamux` into the WebSocket transport to provide multiplexing.
4. **Extend Protocol:** Add `ServiceType::Http` and define the `ControlMessage` enum for HTTP traffic.
5. **Implement `ControlMessage` Handling:** Implement the serialization/deserialization of `ControlMessage` over the multiplexed WebSocket connection.

### Phase 2: Implement Traffic Switching and Client Logic

**Goal:** Implement the dynamic traffic switching logic in the server and update the client to act as an HTTP proxy.

**Tasks:**

1. **Integrate `hyper-reverse-proxy`:** Add `hyper-reverse-proxy` to the server for the cloud fallback.
2. **Implement Traffic Switching Logic:** In the server's HTTP handler, implement the logic to check the tunnel's health and decide whether to forward the request to the local client or the cloud fallback.
3. **Update Client:** Modify the client to connect to the `/register/:id` endpoint, handle `ControlMessage::Request` messages, proxy them to the local server using `hyper`, and send back `ControlMessage::Response` messages.

### Phase 3: State Management and Authentication

**Goal:** Implement scalable state management with Redis and secure the system with JWT-based authentication.

**Tasks:**

1. **Integrate Redis:** Add `redis` as a dependency and replace the in-memory `HashMap` with Redis for storing tunnel state.
2. **Implement JWT Authentication:** Implement JWT-based authentication for the `/register/:id` endpoint.
3. **Secure Client:** Update the client to obtain and use a JWT for authentication.

### Phase 4: Production Readiness

**Goal:** Make the system robust, configurable, and easy to deploy.

**Tasks:**

1. **Configuration:** Make all aspects of the system configurable via a `config.toml` file.
2. **Error Handling and Logging:** Implement comprehensive error handling and structured logging.
3. **Health Checks:** Implement a robust heartbeat and health check mechanism.
4. **Deployment:** Create Dockerfiles and deployment scripts for easy deployment.
5. **Documentation:** Write comprehensive documentation for users and developers.

By following this roadmap, we can efficiently adapt the `rathole` project to create a powerful and scalable `traffic-switch-proxy` that meets all the requirements of the project description while also incorporating several key improvements.
