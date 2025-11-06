use futures_util::{sink::SinkExt, stream::StreamExt};
use hyper::{
    body::to_bytes,
    client::conn::{self},
    Body, Method, Request, Uri, Version,
};
use shared::ControlMessage;
use std::{str::FromStr, time::Duration};
use tokio::{net::TcpStream, sync::mpsc, time::interval};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

#[tokio::main]
async fn main() {
    let relay_url = "ws://127.0.0.1:3000/connect"; // TODO: Make configurable
    let api_key = "some_api_key"; // TODO: Make configurable
    let target_subdomain = "test"; // TODO: Make configurable
    let local_target_url = "http://127.0.0.1:8000"; // TODO: Make configurable

    let (ws_stream, _) = connect_async(relay_url)
        .await
        .expect("Failed to connect to relay");

    let (mut write, mut read) = ws_stream.split();

    // Send registration message
    let register_msg = ControlMessage::Register {
        api_key: api_key.to_string(),
        target_subdomain: target_subdomain.to_string(),
    };
    write
        .send(Message::Text(serde_json::to_string(&register_msg).unwrap()))
        .await
        .expect("Failed to send registration message");

    println!(
        "Connected to relay and registered subdomain: {}",
        target_subdomain
    );

    let (tx_req, mut rx_req) = mpsc::channel::<ControlMessage>(100);

    // Task to send messages to relay
    tokio::spawn(async move {
        let mut health_interval = interval(Duration::from_secs(5));
        health_interval.tick().await; // initial tick

        loop {
            tokio::select! {
                Some(msg) = rx_req.recv() => {
                    write
                        .send(Message::Text(serde_json::to_string(&msg).unwrap()))
                        .await
                        .unwrap();
                }
                _ = health_interval.tick() => {
                    // TODO: Get actual CPU usage and latency
                    let health_msg = ControlMessage::Health { cpu_usage: 0.0, latency_ms: 0 };
                    write
                        .send(Message::Text(serde_json::to_string(&health_msg).unwrap()))
                        .await
                        .unwrap();
                }
            }
        }
    });

    // Task to read messages from relay
    while let Some(message) = read.next().await {
        let msg = message.unwrap();
        if let Message::Text(text) = msg {
            let control_msg: ControlMessage = serde_json::from_str(&text).unwrap();
            match control_msg {
                ControlMessage::Request {
                    request_id,
                    method,
                    path,
                    headers,
                    body,
                } => {
                    println!("Received request: {}", request_id);

                    let local_target_url_parsed = Url::parse(local_target_url).unwrap();
                    let target_uri = format!(
                        "{}{}",
                        local_target_url_parsed.join(&path).unwrap(),
                        local_target_url_parsed
                            .query()
                            .map_or("".to_string(), |q| format!("?{}", q))
                    );

                    let mut request_builder = Request::builder()
                        .method(Method::from_str(&method).unwrap())
                        .uri(Uri::from_str(&target_uri).unwrap())
                        .version(Version::HTTP_11);

                    for (key, value) in headers {
                        request_builder = request_builder.header(&key, value);
                    }

                    let client_req = request_builder.body(Body::from(body)).unwrap();

                    let client = hyper::Client::new();
                    let response = client.request(client_req).await.unwrap();

                    //let client_req = request_builder.body(Body::from(body)).unwrap();

                    let addr = format!(
                        "{}:{}",
                        local_target_url_parsed.host_str().unwrap(),
                        local_target_url_parsed.port_or_known_default().unwrap()
                    );
                    let stream = TcpStream::connect(&addr).await.unwrap();

                    /*let stream = TcpStream::connect(local_target_url_parsed.host_str().unwrap())
                    .await
                    .unwrap();*/
                    let (mut sender, conn) = conn::handshake(stream).await.unwrap();
                    tokio::spawn(async move {
                        conn.await.unwrap();
                    });

                    //let response = sender.send_request(client_req).await.unwrap();

                    let (parts, body) = response.into_parts();
                    let body_bytes = to_bytes(body).await.unwrap().to_vec();

                    let control_response = ControlMessage::Response {
                        request_id,
                        status: parts.status.as_u16(),
                        headers: parts
                            .headers
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
                            .collect(),
                        body: body_bytes,
                    };

                    tx_req.send(control_response).await.unwrap();
                }
                _ => {}
            }
        }
    }
}
