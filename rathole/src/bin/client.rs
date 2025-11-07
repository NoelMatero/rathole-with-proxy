
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use hyper::{
    header::{HeaderName, HeaderValue},
    Client, Request, Body,
};
use rathole::protocol::{ControlMessage, HttpResponse};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream};

#[tokio::main]
async fn main() -> Result<()> {
    let (ws_stream, _) = connect_async("ws://127.0.0.1:3000/register/test").await?;
    println!("Connected to server");

    let (mut write, mut read) = ws_stream.split();

    let register_msg = ControlMessage::Register {
        api_key: "test_key".to_string(),
        target_subdomain: "test".to_string(),
    };
    let register_msg_str = serde_json::to_string(&register_msg)?;
    write.send(Message::Text(register_msg_str)).await?;

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

                    let client = Client::new();
                    let path_and_query = http.path;
                    let uri = format!("http://127.0.0.1:8000{}", path_and_query);
                    let mut req = Request::builder()
                        .method(http.method.as_str())
                        .uri(uri)
                        .body(Body::from(http.body))?;

                    for (key, value) in http.headers {
                        req.headers_mut().insert(
                            HeaderName::from_bytes(key.as_bytes())?,
                            HeaderValue::from_str(&value)?,
                        );
                    }

                    let res = client.request(req).await?;
                    let status = res.status().as_u16();
                    let headers = res
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
                        .collect();
                    let body = hyper::body::to_bytes(res.into_body()).await?.to_vec();

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
                    write.send(Message::Text(res_msg_str)).await?;
                }
                _ => {
                    // Ignore other messages for now
                }
            }
        }
    }

    Ok(())
}
