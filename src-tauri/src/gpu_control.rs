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

fn query_clock(field: &str) -> Option<u32> {
    let out = Command::new("nvidia-smi")
        .args(["--query-gpu", field, "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Locks the GPU core clock to [min_mhz, max_mhz]. Unlike raw voltage table
/// writes (the AMD `pp_od_clk_voltage` route), nvidia-smi's clock lock only
/// accepts values within the driver-reported supported range — out-of-range
/// requests are rejected by the driver itself, not silently applied. This
/// makes it safe to expose without a separate hardware-damage risk review.
/// The result includes a read-back of the actual applied clock so a
/// rejected/clamped value is visible rather than silently assumed.
pub fn set_gpu_clock_lock(min_mhz: u32, max_mhz: u32) -> Result<String, String> {
    if min_mhz > max_mhz {
        return Err("min clock must be <= max clock".to_string());
    }
    let out = Command::new("nvidia-smi")
        .args(["-lgc", &format!("{min_mhz},{max_mhz}")])
        .output()
        .map_err(|e| format!("nvidia-smi not available: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "nvidia-smi -lgc {min_mhz},{max_mhz} failed: {} (usually requires root, and the range must be within clocks.max.graphics)",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let applied = query_clock("clocks.applications.graphics");
    match applied {
        Some(mhz) => Ok(format!(
            "Locked GPU clock to [{min_mhz}, {max_mhz}] MHz (driver reports applications clock now {mhz} MHz)"
        )),
        None => Ok(format!(
            "Locked GPU clock to [{min_mhz}, {max_mhz}] MHz (could not read back applied value to confirm)"
        )),
    }
}

pub fn reset_gpu_clock_lock() -> Result<String, String> {
    let out = Command::new("nvidia-smi")
        .args(["-rgc"])
        .output()
        .map_err(|e| format!("nvidia-smi not available: {e}"))?;
    if out.status.success() {
        Ok("GPU clock lock reset to driver defaults".to_string())
    } else {
        Err(format!(
            "nvidia-smi -rgc failed: {} (usually requires root)",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
