# How to Test the Relay Server

This guide provides step-by-step instructions on how to test the `traffic-switch-proxy` relay server.

Since the local tunnel client is not yet implemented, we will use standard command-line tools like `websocat` and `curl` to simulate a client and test the server's functionality.

## Prerequisites

You will need the following tools installed:

* `cargo`: The Rust package manager.
* `websocat`: A command-line WebSocket client. You can install it with `cargo install websocat`.
* `curl`: A command-line tool for transferring data with URLs.

## Step 1: Run the Relay Server

First, start the relay server in a terminal window:

```bash
cargo run -p relay
```

You should see the following output, indicating that the server is running:

```
INFO listening on 127.0.0.1:3001
```

Keep this terminal window open to monitor the server logs.

## Step 2: Simulate a Client Connection

Next, we will simulate a local client connecting to the relay and registering a tunnel.

Open a **new terminal window** and use `websocat` to connect to the relay's `/register/:id` endpoint. We will use `test-tunnel` as the tunnel ID:

```bash
websocat ws://127.0.0.1:3001/register/test-tunnel
```

This command will establish a persistent WebSocket connection to the relay.

In the relay server's terminal window, you should see a log message indicating that the tunnel has been registered:

```
INFO Tunnel test-tunnel registered
```

## Step 3: Test the Tunnel Forwarding

Now that we have a registered tunnel, we can send an HTTP request to the relay and see if it gets forwarded to the tunnel.

Open a **third terminal window** and use `curl` to send a request. The key is to set the `Host` header to `test-tunnel.localhost`, which tells the relay to route the request to our registered tunnel:

```bash
curl -H "Host: test-tunnel.localhost" http://127.0.0.1:3001/some/path
```

## Step 4: Observe the Relay and Client Logs

After running the `curl` command, you should see the following activity:

**In the relay server's terminal:**

You will see logs indicating that a tunnel was found and the request is being forwarded via WebSocket:

```
INFO Tunnel found for subdomain: test-tunnel. Forwarding via WebSocket.
```

**In the `websocat` (client) terminal:**

You will see the `ControlMessage::Request` that the relay sent to the client. It will be a JSON object containing the details of the HTTP request you sent with `curl`.

This confirms that the relay server is correctly forwarding requests to the registered tunnel.

## Step 5: Test the Fallback Mechanism

Finally, let's test the fallback mechanism. We will send a request to a subdomain that does **not** have a registered tunnel.

In your third terminal window, run the following `curl` command with a different `Host` header:

```bash
curl -H "Host: another-tunnel.localhost" http://127.0.0.1:3001/
```

Since there is no tunnel registered for `another-tunnel`, the relay server should forward the request to the fallback backend URL (`http://httpbin.org`).

**In the relay server's terminal:**

You will see a log message indicating that no tunnel was found and the request is being forwarded to the fallback URL:

```
INFO No tunnel found for host. Forwarding to fallback URL: http://httpbin.org
```

You should then see the HTML response from `httpbin.org` in your terminal.

This confirms that the fallback mechanism is working correctly.
