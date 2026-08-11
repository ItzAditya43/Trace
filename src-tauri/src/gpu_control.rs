use std::process::Command;

pub fn set_persistence_mode(enabled: bool) -> Result<String, String> {
    let flag = if enabled { "1" } else { "0" };
    let out = Command::new("nvidia-smi")
        .args(["-pm", flag])
        .output()
        .map_err(|e| format!("nvidia-smi not available: {e}"))?;
    if out.status.success() {
        Ok(format!(
            "Persistence mode {}",
            if enabled { "enabled" } else { "disabled" }
        ))
    } else {
        Err(format!(
            "nvidia-smi -pm {flag} failed: {} (usually requires root)",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

pub fn set_power_limit(watts: u32) -> Result<String, String> {
    let out = Command::new("nvidia-smi")
        .args(["-pl", &watts.to_string()])
        .output()
        .map_err(|e| format!("nvidia-smi not available: {e}"))?;
    if out.status.success() {
        Ok(format!("GPU power limit set to {watts}W"))
    } else {
        Err(format!(
            "nvidia-smi -pl {watts} failed: {} (usually requires root, and must be within the card's supported range)",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
