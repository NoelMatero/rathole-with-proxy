use clap::Parser;
use futures_util::{sink::SinkExt, stream::StreamExt};
use hyper::{body::to_bytes, header::{HeaderName, HeaderValue}, Body, Method, Request, Uri};
use shared::ControlMessage;
use std::{str::FromStr, time::Duration};
use tokio::time::interval;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "ws://127.0.0.1:3001/register")]
    relay_url: String,

    #[arg(long)]
    api_key: String,

    #[arg(long)]
    subdomain: String,

    #[arg(long, default_value = "http://127.0.0.1:8000")]
    target_url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let (ws_stream, _) = connect_async(format!("{}/{}", args.relay_url, args.subdomain)).await?;
    let (mut write, mut read) = ws_stream.split();

    let register_msg = ControlMessage::Register {
        api_key: args.api_key,
        target_subdomain: args.subdomain.clone(),
    };
    write.send(Message::Text(serde_json::to_string(&register_msg)?)).await?;

    println!("Connected to relay and registered subdomain: {}", args.subdomain);

    let client = hyper::Client::new();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ControlMessage>(100);

    tokio::spawn(async move {
        let mut health_interval = interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                Some(msg) = rx.recv() => {
                    if write.send(Message::Text(serde_json::to_string(&msg).unwrap())).await.is_err() {
                        break;
                    }
                }
                _ = health_interval.tick() => {
                    let health_msg = ControlMessage::Health { cpu_usage: 0.0, latency_ms: 0 };
                    if write.send(Message::Text(serde_json::to_string(&health_msg).unwrap())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(message) = read.next().await {
        let msg = match message {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("Error receiving message: {}", e);
                continue;
            }
        };

        if let Message::Text(text) = msg {
            let control_msg: ControlMessage = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("Error deserializing control message: {}", e);
                    continue;
                }
            };

            if let ControlMessage::Request { request_id, method, path, headers, body } = control_msg {
                println!("Received request: {}", request_id);
                let client = client.clone();
                let target_url = args.target_url.clone();
                let tx = tx.clone();

                tokio::spawn(async move {
                    let mut request_builder = Request::builder()
                        .method(Method::from_str(&method).unwrap())
                        .uri(Uri::from_str(&format!("{}{}", target_url, path)).unwrap())
                        .body(Body::from(body))
                        .unwrap();

                    for (key, value) in headers {
                        request_builder.headers_mut().insert(HeaderName::from_str(&key).unwrap(), HeaderValue::from_str(&value).unwrap());
                    }

                    match client.request(request_builder).await {
                        Ok(response) => {
                            let (parts, body) = response.into_parts();
                            let body_bytes = to_bytes(body).await.unwrap().to_vec();

                            let control_response = ControlMessage::Response {
                                request_id,
                                status: parts.status.as_u16(),
                                headers: parts.headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string())).collect(),
                                body: body_bytes,
                            };

                            if tx.send(control_response).await.is_err() {
                                eprintln!("Failed to send response to relay");
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to proxy request: {}", e);
                            let response = ControlMessage::Response {
                                request_id,
                                status: 502,
                                headers: vec![],
                                body: b"Bad Gateway".to_vec(),
                            };
                            if tx.send(response).await.is_err() {
                                eprintln!("Failed to send error response to relay");
                            }
                        }
                    }
                });
            }
        }
    }

    Ok(())
}
