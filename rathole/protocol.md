🧭 Goal

Integrate Rathole’s binary stream layer into trafficswitch so that the ControlMessage protocol rides on top of it instead of replacing it.
This means:

The transport layer (connection management, framing, multiplexing) is Rathole’s.

The control plane (registration, routing, metadata, auth, tunnel events) is yours.

               ┌────────────────────────────┐
               │        Public User         │
               │     (HTTP / TCP Client)    │
               └────────────┬───────────────┘
                            │
                     (public port)
                            │
                      ┌─────────────┐
                      │  Relay Proxy│
                      │ (trafficswitch)
                      ├─────────────┤
     Control Channel →│ ControlMessage handler │
   (Registration etc.)│       (JSON/Bincode)   │
                      ├─────────────┤
  Data Tunnel Layer → │ Rathole Framed Streams │
                      └─────────────┘
                            ▲
                            │
                     Persistent TCP tunnel
                            │
                      ┌─────────────┐
                      │   Client    │
                      │ (trafficswitch-tunnel) │
                      ├─────────────┤
                      │ Rathole Conn/StreamHandler │
                      ├─────────────┤
                      │ ControlMessage logic │
                      └─────────────┘
🔐 Control Plane Responsibilities

ControlMessage layer continues to handle:

Tunnel registration (RegisterTunnel { id, token, port })

Ping / pong heartbeats

Route updates (SwitchRoute, CloseRoute)

Stream open/close requests (with IDs referencing Rathole multiplexed streams)

Error and status responses

These messages remain semantically clear and can be logged or inspected.

⚡ Data Plane Responsibilities (Rathole layer)

The reused Rathole components handle:

Stream framing and demultiplexing

Binary encoding/decoding of chunked stream data

Connection keepalive and reconnection retries

Efficient I/O via tokio’s split and async channels

No need to reinvent multiplexing or binary framing — you just map your logical streams onto Rathole’s infrastructure.
