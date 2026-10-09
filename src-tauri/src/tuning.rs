use crate::control_process::ActionResult;
use crate::sys_util::{
    err, has_tool, is_safe_token, ok, privileged_action, read_num, read_trimmed, run, run_lossy,
};
use serde::Serialize;
use std::fs;
use std::process::Command;

// ---- memory & swap tuning ----

#[derive(Serialize)]
pub struct MemoryTuning {
    pub swappiness: u32,
    pub swap_used_bytes: u64,
    pub available_bytes: u64,
    /// Whether everything in swap would fit back into RAM right now.
    pub can_flush_swap: bool,
}

fn meminfo_bytes(key: &str) -> u64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::to_string))
        })
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024
}

pub fn memory_tuning() -> MemoryTuning {
    let swap_used = meminfo_bytes("SwapTotal").saturating_sub(meminfo_bytes("SwapFree"));
    let available = meminfo_bytes("MemAvailable");
    MemoryTuning {
        swappiness: read_num("/proc/sys/vm/swappiness").unwrap_or(60),
        swap_used_bytes: swap_used,
        available_bytes: available,
        // Compressed swap expands when pulled back in, so leave real headroom.
        can_flush_swap: swap_used > 0 && available > swap_used + swap_used / 4,
    }
}

pub fn set_swappiness(value: u32) -> ActionResult {
    let value = value.min(200);
    privileged_action(
        &format!("sysctl -q vm.swappiness={value}"),
        format!("Swappiness set to {value} (until next reboot)"),
    )
}

pub fn drop_caches() -> ActionResult {
    privileged_action(
        "sync; echo 3 > /proc/sys/vm/drop_caches",
        "Dropped the file cache — it will refill as files are read again",
    )
}

pub fn compact_memory() -> ActionResult {
    privileged_action(
        "echo 1 > /proc/sys/vm/compact_memory",
        "Compacted memory to reduce fragmentation",
    )
}

/// Pulls everything back out of swap by cycling it off and on. Refuses when
/// the swapped data wouldn't fit in RAM, because `swapoff` would then grind
/// the machine to a halt or trigger the OOM killer.
pub fn flush_swap() -> ActionResult {
    let t = memory_tuning();
    if t.swap_used_bytes == 0 {
        return ok("Swap is already empty");
    }
    if !t.can_flush_swap {
        return err(format!(
            "Not enough free RAM to pull {:.1} GB back out of swap safely — close something first",
            t.swap_used_bytes as f64 / 1e9
        ));
    }
    privileged_action(
        "swapoff -a && swapon -a",
        "Swap emptied — everything is back in RAM",
    )
}

// ---- CPU governor, power profile, cores ----

#[derive(Serialize)]
pub struct CpuCore {
    pub index: u32,
    pub online: bool,
    /// cpu0 usually can't be taken offline.
    pub can_toggle: bool,
}

#[derive(Serialize)]
pub struct CpuPower {
    pub governor: Option<String>,
    pub available_governors: Vec<String>,
    pub energy_preference: Option<String>,
    pub available_energy_preferences: Vec<String>,
    pub power_profile: Option<String>,
    pub available_power_profiles: Vec<String>,
    pub platform_profile: Option<String>,
    pub available_platform_profiles: Vec<String>,
    pub cores: Vec<CpuCore>,
}

const CPUFREQ: &str = "/sys/devices/system/cpu/cpu0/cpufreq";
const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";

fn words(path: &str) -> Vec<String> {
    read_trimmed(path)
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

fn list_power_profiles() -> Vec<String> {
    // Lines look like "* balanced:" / "  performance:".
    run_lossy(Command::new("powerprofilesctl").arg("list"))
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(['*', ' ']);
            let name = l.strip_suffix(':')?;
            (!name.contains(' ')).then(|| name.to_string())
        })
        .collect()
}

pub fn cpu_power() -> CpuPower {
    let mut cores: Vec<CpuCore> = fs::read_dir("/sys/devices/system/cpu")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let index: u32 = name.strip_prefix("cpu")?.parse().ok()?;
                    let online_path = format!("/sys/devices/system/cpu/{name}/online");
                    let online = read_num::<u8>(&online_path);
                    Some(CpuCore {
                        index,
                        online: online != Some(0),
                        can_toggle: online.is_some(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    cores.sort_by_key(|c| c.index);

    let has_ppd = has_tool("powerprofilesctl");
    CpuPower {
        governor: read_trimmed(&format!("{CPUFREQ}/scaling_governor")),
        available_governors: words(&format!("{CPUFREQ}/scaling_available_governors")),
        energy_preference: read_trimmed(&format!("{CPUFREQ}/energy_performance_preference")),
        available_energy_preferences: words(&format!(
            "{CPUFREQ}/energy_performance_available_preferences"
        )),
        power_profile: has_ppd
            .then(|| run(Command::new("powerprofilesctl").arg("get")).ok())
            .flatten(),
        available_power_profiles: if has_ppd { list_power_profiles() } else { Vec::new() },
        platform_profile: read_trimmed(PLATFORM_PROFILE),
        available_platform_profiles: words(&format!("{PLATFORM_PROFILE}_choices")),
        cores,
    }
}

pub fn set_power_profile(profile: &str) -> ActionResult {
    if !list_power_profiles().iter().any(|p| p == profile) {
        return err(format!("Unknown power profile {profile}"));
    }
    match run(Command::new("powerprofilesctl").args(["set", profile])) {
        Ok(_) => ok(format!("Power profile set to {profile}")),
        Err(e) => err(format!("Failed to set power profile: {e}")),
    }
}

fn write_all_cores(file: &str, value: &str, allowed: &[String], what: &str) -> ActionResult {
    if !allowed.iter().any(|a| a == value) || !is_safe_token(value) {
        return err(format!("{value} is not a supported {what} on this CPU"));
    }
    privileged_action(
        &format!(
            "for f in /sys/devices/system/cpu/cpu*/cpufreq/{file}; do echo {value} > \"$f\"; done"
        ),
        format!("CPU {what} set to {value} on all cores"),
    )
}

pub fn set_governor(governor: &str) -> ActionResult {
    let allowed = words(&format!("{CPUFREQ}/scaling_available_governors"));
    write_all_cores("scaling_governor", governor, &allowed, "governor")
}

pub fn set_energy_preference(preference: &str) -> ActionResult {
    let allowed = words(&format!("{CPUFREQ}/energy_performance_available_preferences"));
    write_all_cores("energy_performance_preference", preference, &allowed, "energy preference")
}

pub fn set_platform_profile(profile: &str) -> ActionResult {
    let allowed = words(&format!("{PLATFORM_PROFILE}_choices"));
    if !allowed.iter().any(|a| a == profile) || !is_safe_token(profile) {
        return err(format!("{profile} is not a supported platform profile"));
    }
    privileged_action(
        &format!("echo {profile} > {PLATFORM_PROFILE}"),
        format!("Platform (fan/thermal) profile set to {profile}"),
    )
}

pub fn set_core_online(index: u32, online: bool) -> ActionResult {
    let path = format!("/sys/devices/system/cpu/cpu{index}/online");
    if !std::path::Path::new(&path).exists() {
        return err(format!("Core {index} can't be switched on or off"));
    }
    privileged_action(
        &format!("echo {} > {path}", online as u8),
        format!("Core {index} switched {}", if online { "on" } else { "off" }),
    )
}

// ---- fans ----

#[derive(Serialize)]
pub struct Fan {
    pub chip: String,
    pub label: String,
    pub rpm: u32,
    /// hwmon path of the matching PWM control, when the driver exposes one.
    pub pwm_path: Option<String>,
    /// 0-100, only when a raw PWM value is readable.
    pub pwm_percent: Option<u32>,
    /// True when the firmware/driver is controlling the fan itself.
    pub automatic: Option<bool>,
}

pub fn list_fans() -> Vec<Fan> {
    let mut fans = Vec::new();
    let Ok(chips) = fs::read_dir("/sys/class/hwmon") else {
        return fans;
    };
    for chip in chips.flatten() {
        let dir = chip.path().to_string_lossy().to_string();
        let chip_name = read_trimmed(&format!("{dir}/name")).unwrap_or_default();
        for n in 1..=8 {
            let Some(rpm) = read_num::<u32>(&format!("{dir}/fan{n}_input")) else {
                continue;
            };
            let pwm = format!("{dir}/pwm{n}");
            let enable = read_num::<u32>(&format!("{pwm}_enable"));
            fans.push(Fan {
                chip: chip_name.clone(),
                label: read_trimmed(&format!("{dir}/fan{n}_label"))
                    .unwrap_or_else(|| format!("fan{n}")),
                rpm,
                pwm_percent: read_num::<u32>(&pwm).map(|v| v * 100 / 255),
                pwm_path: enable.map(|_| pwm.clone()),
                // hwmon convention: 0 = full speed, 1 = manual, 2+ = automatic.
                automatic: enable.map(|v| v >= 2),
            });
        }
    }
    fans
}

fn is_pwm_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/sys/class/hwmon/hwmon") else {
        return false;
    };
    let Some((chip, file)) = rest.split_once('/') else {
        return false;
    };
    chip.chars().all(|c| c.is_ascii_digit())
        && file.strip_prefix("pwm").is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit()))
}

/// `mode`: "auto" hands the fan back to firmware, "full" pins it at maximum,
/// "manual" sets a fixed duty cycle (only on drivers that expose raw PWM).
pub fn set_fan_mode(pwm_path: &str, mode: &str, percent: u32) -> ActionResult {
    if !is_pwm_path(pwm_path) {
        return err("Not a fan control path");
    }
    let script = match mode {
        "auto" => format!("echo 2 > {pwm_path}_enable"),
        "full" => format!("echo 0 > {pwm_path}_enable"),
        "manual" => {
            if !std::path::Path::new(pwm_path).exists() {
                return err("This fan only supports automatic or full speed, not a custom speed");
            }
            // Never let a manual setting stop a fan outright.
            let duty = percent.clamp(25, 100) * 255 / 100;
            format!("echo 1 > {pwm_path}_enable && echo {duty} > {pwm_path}")
        }
        _ => return err(format!("Unknown fan mode {mode}")),
    };
    privileged_action(&script, format!("Fan set to {mode}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_real_pwm_paths() {
        assert!(is_pwm_path("/sys/class/hwmon/hwmon6/pwm1"));
        assert!(!is_pwm_path("/sys/class/hwmon/hwmon6/pwm1; rm -rf /"));
        assert!(!is_pwm_path("/sys/class/hwmon/hwmon6/../../x/pwm1"));
        assert!(!is_pwm_path("/etc/passwd"));
    }
}
