use lightai_agent::gpu::{custom, nvidia};

#[test]
fn parses_nvidia_smi_csv_output() {
    let output = "0, NVIDIA A10, GPU-abc, 550.1, 24000, 12000, 88, 62, 110\n";

    let gpus = nvidia::parse_nvidia_smi_csv(output).unwrap();

    assert_eq!(gpus.len(), 1);
    assert_eq!(gpus[0].gpu_key, "nvidia:GPU-abc");
    assert_eq!(gpus[0].vendor, "nvidia");
    assert_eq!(gpus[0].name, "NVIDIA A10");
    assert_eq!(gpus[0].memory_total_bytes, Some(24_000 * 1024 * 1024));
    assert_eq!(gpus[0].utilization_percent, Some(88.0));
}

#[test]
fn parses_custom_collector_json_output() {
    let output = r#"{
      "gpus": [{
        "index": 0,
        "vendor": "custom",
        "name": "Vendor GPU",
        "uuid": "custom-0",
        "memory_total_bytes": 1000,
        "memory_used_bytes": 400,
        "utilization_percent": 55,
        "temperature_celsius": 70,
        "power_watts": 120
      }]
    }"#;

    let gpus = custom::parse_custom_output(output).unwrap();

    assert_eq!(gpus.len(), 1);
    assert_eq!(gpus[0].gpu_key, "custom:custom-0");
    assert_eq!(gpus[0].memory_used_bytes, Some(400));
}
