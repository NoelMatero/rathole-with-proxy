# Evaluation of `axum-reverse-proxy`

This report evaluates the replacement of the `hyper-reverse-proxy` crate with `axum-reverse-proxy` for the fallback mechanism in `proxy.rs`.

## Problem

The current implementation in `proxy.rs` uses `hyper-reverse-proxy`. This crate is built on an older version of the `http` crate (`http` v0.2), while `axum` v0.7 (used in this project) relies on `http` v1.0.

This version mismatch forces a cumbersome and inefficient conversion process:

1.  An `axum::Request` must be manually deconstructed into its parts.
2.  A new `hyper::Request` (from the older `http` version) must be reconstructed.
3.  After the proxy call, the resulting `hyper::Response` must be deconstructed.
4.  Finally, a new `axum::Response` must be reconstructed to be returned by the handler.

This is not only verbose but also introduces unnecessary overhead and potential for errors.

## Solution: `axum-reverse-proxy`

The `axum-reverse-proxy` crate is a modern, high-level reverse proxy designed specifically for `axum`. It is built on compatible versions of `hyper` and `http`, which eliminates the need for manual request/response conversion.

### `Cargo.toml` Changes

First, we need to update the dependencies in `Cargo.toml`.

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -100,7 +100,7 @@
 reqwest = { version = "0.11", features = ["json"] }
 redis = { version = "0.25.0", features = ["tokio-comp"] }
-hyper-reverse-proxy = "0.5.1"
+axum-reverse-proxy = "1.0.3"
 
 [[bin]]
 name = "rathole-client"
```

### Code Changes in `src/proxy.rs`

The `proxy_handler` function in `src/proxy.rs` can be significantly simplified.

#### Before (`hyper-reverse-proxy`)

```rust
// Fallback logic
println!(
    "No tunnel found. Forwarding to fallback URL: {}",
    fallback_url
);
let (parts, body) = req.into_parts();
let body_bytes = to_bytes(body, usize::MAX)
    .await
    .map_err(|e| AppError::Other(e.into()))?;

let mut hyper_req = HyperRequest::new(Body::from(body_bytes));
*hyper_req.method_mut() = parts.method;
*hyper_req.uri_mut() = parts.uri;
*hyper_req.headers_mut() = parts.headers;

match hyper_reverse_proxy::call(client_ip, fallback_url, hyper_req).await {
    Ok(response) => {
        let (parts, body) = response.into_parts();
        let body_bytes = hyper::body::to_bytes(body)
            .await
            .map_err(|e| AppError::Other(e.into()))?;
        let body = Body::from(body_bytes);
        let mut axum_res = Response::new(body);
        *axum_res.status_mut() = parts.status;
        *axum_res.headers_mut() = parts.headers;
        Ok(axum_res)
    }
    Err(error) => {
        println!("Failed to forward request: {:?}", error);
        Err(AppError::Other(anyhow::anyhow!("Fallback failed")))
    }
}
```

#### After (`axum-reverse-proxy`)

With `axum-reverse-proxy`, the fallback logic becomes a single function call.

```rust
use axum_reverse_proxy::ReverseProxy;
use hyper::Client;

// ...

// Fallback logic
println!(
    "No tunnel found. Forwarding to fallback URL: {}",
    fallback_url
);

// Create a new ReverseProxy for the fallback URL
let proxy = ReverseProxy::new(fallback_url);

// The request `req` is already a `hyper::Request<axum::body::Body>`, which is compatible.
// We can use the `call` method of the `Service` trait.
use tower::Service;
match proxy.oneshot(req).await {
    Ok(response) => Ok(response.into_response()),
    Err(error) => {
        tracing::error!("Failed to forward request: {:?}", error);
        Err(AppError::Other(anyhow::anyhow!("Fallback failed")))
    }
}
```

## Flexibility and Low-Level Control

You asked a few important questions about the level of abstraction.

*   **Can we build a low-level proxy by ourselves with `axum-reverse-proxy`?**
    The crate is designed as a high-level abstraction. It does not expose its internal, low-level components for building a custom proxy piece-by-piece. It provides a complete, ready-to-use proxy `Service`.

*   **Does this matter?**
    For the primary goal of this project—creating a reliable fallback mechanism—this high level of abstraction is actually a significant advantage. Building a correct and robust reverse proxy is complex. It involves correctly handling headers (like `Forwarded`, `X-Forwarded-For`), managing connection pooling, streaming large bodies, and properly handling WebSocket upgrades. `axum-reverse-proxy` encapsulates all this complexity, providing a simple and well-tested solution. For our use case, we don't need fine-grained control over the proxy mechanics, we just need it to forward requests reliably.

*   **Can lower-level components be used instead?**
    No, the library's public API is focused on providing a complete proxy service. If you needed more control, you would have to bypass `axum-reverse-proxy` and use `hyper` directly, which would lead back to the kind of boilerplate code we are trying to eliminate.

## Conclusion

Replacing `hyper-reverse-proxy` with `axum-reverse-proxy` is highly recommended. The benefits are:

*   **Simplified Code:** The verbose and error-prone request/response conversion is eliminated.
*   **Improved Performance:** Removing the manual conversion steps reduces overhead.
*   **Better Maintainability:** The code becomes cleaner, more idiomatic, and easier to understand.
*   **Native Integration:** It provides a seamless and modern integration with the `axum` framework.

This change would align the proxy logic with modern `axum` practices and resolve the underlying dependency incompatibility.
