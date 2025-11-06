use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ControlMessage {
    Register { api_key: String, target_subdomain: String },
    Request { request_id: String, method: String, path: String, headers: Vec<(String, String)>, body: Vec<u8> },
    Response { request_id: String, status: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    Health { cpu_usage: f32, latency_ms: u32 },
    Pong,
}
