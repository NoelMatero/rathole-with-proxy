
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use rathole::protocol::ControlMessage;
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
        println!("Received: {:?}", msg);
    }

    Ok(())
}
