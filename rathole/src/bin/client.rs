use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
//use hyper::body::{Body, Incoming as HyperBody};
use hyper::header::{HeaderName, HeaderValue};
use hyper::Request;
use reqwest::Method;
use std::usize;
//use hyper::{Body, Client, Request};
use hyper::body::Incoming as HyperBody;
//use hyper::Body;

use hyper::body::Incoming;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use rathole::protocol::{ControlMessage, HttpResponse};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, WebSocketStream};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};

#[tokio::main]
async fn main() -> Result<()> {
    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .unwrap() // TODO:
        .https_or_http()
        .enable_http1()
        .build();

    let client = reqwest::Client::new();
    let res = client
        .post("http://127.0.0.1:3000/login")
        .json(&json!({
            "username": "test",
            "password": "test"
        }))
        .send()
        .await?;
    let token = res.text().await?;

    let (ws_stream, _) = connect_async("ws://127.0.0.1:3000/register/test").await?;
    println!("Connected to server");

    let (mut write, mut read) = ws_stream.split();

    let register_msg = ControlMessage::Register {
        token,
        target_subdomain: "test".to_string(),
    };
    let register_msg_str = serde_json::to_string(&register_msg)?;
    write.send(Message::Text(register_msg_str.into())).await?;

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if let Message::Text(text) = msg {
            let msg: ControlMessage = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(err) => {
                    println!("Failed to parse message: {}", err);
                    continue;
                }
            };

            match msg {
                ControlMessage::Request { request_id, http } => {
                    println!("Received request: {:?}", http);

                    let client: Client<_, Full<Bytes>> =
                        Client::builder(TokioExecutor::new()).build(https.clone());
                    println!("client: {:?}", client);
                    let path_and_query = http.path;
                    let uri = format!("http://127.0.0.1:8000/{}", path_and_query);
                    println!("uri: {}", uri);
                    let body = Full::new(Bytes::from(http.body));
                    let mut req = Request::builder()
                        .method(http.method.as_str())
                        .uri(uri)
                        .body(body)?;

                    for (key, value) in http.headers {
                        req.headers_mut().insert(
                            HeaderName::from_bytes(key.as_bytes())?,
                            HeaderValue::from_str(&value)?,
                        );
                    }

                    let res = client.request(req).await?;
                    println!("res {:?}", res);
                    let status = res.status().as_u16();
                    let headers = res
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
                        .collect();

                    let body_bytes = res.into_body().collect().await?.to_bytes();
                    let body = body_bytes.to_vec();

                    let http_res = HttpResponse {
                        status,
                        headers,
                        body,
                    };

                    let res_msg = ControlMessage::Response {
                        request_id,
                        http: http_res,
                    };
                    let res_msg_str = serde_json::to_string(&res_msg)?;
                    write.send(Message::Text(res_msg_str.into())).await?;
                }
                _ => {
                    // Ignore other messages for now
                }
            }
        }
    }

    Ok(())
}
