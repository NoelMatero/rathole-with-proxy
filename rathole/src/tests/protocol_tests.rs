use rathole::protocol::HardwareData;

#[test]
fn test_hardware_data_debug() {
    let hardware_data = HardwareData {
        operating_system: "TestOS".to_string(),
        total_memory: 1024,
        used_memory: 512,
        total_swap: 2048,
        used_swap: 1024,
        cpu_usage: 0.75,
        avg_temp: 55.5,
        max_temp: 70.0,
    };

    let debug_output = format!("{:?}", hardware_data);

    assert!(debug_output.contains("operating_system: \"TestOS\""));
    assert!(debug_output.contains("total_memory: 1024"));
    assert!(debug_output.contains("used_memory: 512"));
    assert!(debug_output.contains("total_swap: 2048"));
    assert!(debug_output.contains("used_swap: 1024"));
    assert!(debug_output.contains("cpu_usage: 0.75"));
    assert!(debug_output.contains("avg_temp: 55.5"));
    assert!(debug_output.contains("max_temp: 70.0"));
}

