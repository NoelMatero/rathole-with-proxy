use crate::protocol::HardwareData;
use crate::proxy::HealthStatus;

struct HardwareLimitConfig {
    cpu_limit: f32,
    memory_limit: f32,
    swap_limit: f32,
    free_memory: u64,
    free_swap: u64,
}

pub async fn handle_hardware_data(hardware_data: HardwareData, cpu_limit: f32) -> HealthStatus {
    if hardware_data.cpu_usage > cpu_limit
        || hardware_data.used_memory as f32 / hardware_data.total_memory as f32 > 90.0
        || hardware_data.used_swap as f32 / hardware_data.total_swap as f32 > 90.0
        || hardware_data.total_swap - hardware_data.used_swap <= 1
        || hardware_data.total_memory - hardware_data.used_memory <= 1
    {
        return HealthStatus::Critical;
    }
    HealthStatus::Normal
}
