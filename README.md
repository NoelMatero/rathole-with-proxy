Testing md from a project. basically a light demo showcasing basic intelligent load balancing

# Testing Guide for Rathole Proxy with Traffic Switching

This guide provides step-by-step instructions to test the `rathole` proxy's basic tunneling, health updates, traffic switching, and sticky session features.

**Prerequisites:**
*   `python3` and `curl` installed on your system.
*   Ensure you are in the root directory of the `rathole` project.

---

## 1. Build the Project

First, compile the `rathole` server and `rathole-client` binaries.

```bash
cargo build
```

---

## 2. Prepare Configuration Files

Ensure your `config.toml` and `client.toml` files are correctly configured as follows. These files should already exist and have been updated in previous steps.

**`config.toml` (for the `rathole` server):**
```toml
[server]
bind_addr = "127.0.0.1:3000"
jwt_secret = "your-super-secret-and-long-jwt-secret"
# IMPORTANT: Ensure this URL includes the "http://" or "https://" scheme.
default_cloud_backend = "http://127.0.0.1:4000"
```

**`client.toml` (for the `rathole-client`):**
```toml
[client]
remote_addr = "127.0.0.1:3000" # Rathole server address

[client.services.test] # Note: The service name is 'test' as per your client.toml
token = "some_auth_token" # This token will be used for JWT authentication
local_addr = "127.0.0.1:8000" # Our mock local backend
```

---

## 3. Set up Mock Backend Servers

Create two simple `index.html` files in your current directory:

*   **`index.html` for local backend:**
    ```html
    <h1>Hello from Local!</h1>
    ```
*   **`index.html` for cloud backend:**
    ```html
    <h1>Hello from Cloud!</h1>
    ```

Now, open **two separate terminal windows** and start these simple Python HTTP servers:

### Terminal 1: Local Backend (port 8000)
```bash
python3 -m http.server 8000
```

### Terminal 2: Cloud Backend (port 4000)
```bash
python3 -m http.server 4000
```

---

## 4. Start the `rathole` Server

Open a **third terminal window** and start the `rathole` server:

### Terminal 3: Rathole Server
```bash
cargo run --bin rathole -- --config config.toml
```
*Observe the server logs for messages indicating tunnel registration and health updates.*

---

## 5. Start the `rathole-client`

Open a **fourth terminal window** and start the `rathole-client`:

### Terminal 4: Rathole Client
```bash
cargo run --bin rathole-client -- --config client.toml
```
*Observe the client logs for connection status and "Client: Sent health update." messages. Also, check the server logs for "Received HealthUpdate" messages.*

---

## 6. Run Test Commands

Open a **fifth terminal window** for running `curl` commands.

### 6.1. Test Basic Tunneling

This verifies that requests are routed through the tunnel to your local backend when everything is healthy.

```bash
curl -H "Host: test.127.0.0.1" http://127.0.0.1:3000
```
**Expected Output:**
```
<h1>Hello from Local!</h1>
```

### 6.2. Test Health Updates

*   Observe **Terminal 4 (Rathole Client)**. You should see `Client: Sent health update.` messages appearing every 10 seconds.
*   Observe **Terminal 3 (Rathole Server)**. You should see messages like `Received HealthUpdate from client...` (or similar debug output) indicating the server is receiving these updates.

### 6.3. Test Traffic Switching (Failover)

This verifies that traffic automatically switches to the cloud backend when the local client becomes unhealthy.

1.  **Simulate Client Unhealthiness:**
    Go to **Terminal 4 (Rathole Client)** and press `Ctrl+C` to stop it.
    *Wait about 30-40 seconds* for the `rathole` server to detect the client's unhealthiness (due to no recent health updates).

2.  **Send a Request:**
    ```bash
    curl -H "Host: test.127.0.0.1" http://127.0.0.1:3000
    ```
    **Expected Output:**
    ```
    <h1>Hello from Cloud!</h1>
    ```
    (This confirms traffic has switched to the `default_cloud_backend` because the local client is unhealthy.)

### 6.4. Test Sticky Sessions - Local Sticky (Failover)

This verifies that if a session is "stuck" to the local backend, it will still failover to the cloud if the local client becomes unhealthy.

1.  **Restart Client:**
    Go to **Terminal 4 (Rathole Client)** and restart it:
    ```bash
    cargo run --bin rathole-client -- --config client.toml
    ```
    Wait a few seconds for it to reconnect and start sending health updates.

2.  **Establish Local Session:**
    Send a request to establish a local session and capture the cookie. The `-v` flag shows response headers.
    ```bash
    curl -v -H "Host: test.127.0.0.1" http://127.0.0.1:3000
    ```
    **Expected Output:**
    *   `<h1>Hello from Local!</h1>`
    *   You should see a `Set-Cookie: backend=local; Path=/` header in the verbose output.

3.  **Simulate Client Unhealthiness (again):**
    Go to **Terminal 4 (Rathole Client)** and stop it again (`Ctrl+C`).

4.  **Send Request with Local Cookie:**
    Send a request *with the captured `backend=local` cookie*:
    ```bash
    curl -H "Host: test.127.0.0.1" -H "Cookie: backend=local" http://127.0.0.1:3000
    ```
    **Expected Output:**
    ```
    <h1>Hello from Cloud!</h1>
    ```
    (Even with the `backend=local` cookie, the server detects the local client is unhealthy and fails over to the cloud. This demonstrates the failover aspect of the sticky session.)

### 6.5. Test Sticky Sessions - Cloud Sticky

This verifies that if a session is "stuck" to the cloud backend, it will remain there even if the local client becomes healthy.

1.  **Ensure Client is Stopped:**
    Make sure the `rathole-client` in **Terminal 4** is *stopped*.

2.  **Establish Cloud Session:**
    Send a request to establish a cloud session and capture the cookie:
    ```bash
    curl -v -H "Host: test.127.0.0.1" http://127.0.0.1:3000
    ```
    **Expected Output:**
    *   `<h1>Hello from Cloud!</h1>`
    *   You should see a `Set-Cookie: backend=cloud; Path=/` header in the verbose output.

3.  **Restart Client:**
    Go to **Terminal 4 (Rathole Client)** and restart it:
    ```bash
    cargo run --bin rathole-client -- --config client.toml
    ```
    Wait a few seconds for it to reconnect and become healthy.

4.  **Send Request with Cloud Cookie:**
    Send a request *with the captured `backend=cloud` cookie*:
    ```bash
    curl -H "Host: test.127.0.0.1" -H "Cookie: backend=cloud" http://127.0.0.1:3000
    ```
    **Expected Output:**
    ```
    <h1>Hello from Cloud!</h1>
    ```
    (Even though the local client is now healthy, the `backend=cloud` cookie forces the request to the cloud backend, demonstrating the sticky session.)

---
This concludes the testing guide.
