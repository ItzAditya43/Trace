use crate::sys_util::{read_num, read_trimmed, run_lossy, run_privileged};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::process::Command;

// ---- pressure (PSI) ----

/// Share of the last 10 seconds in which work was stalled waiting for a
/// resource. `some` = at least one task waiting, `full` = everything waiting.
#[derive(Serialize, Clone, Default)]
pub struct Pressure {
    pub cpu_some: f32,
    pub io_some: f32,
    pub io_full: f32,
    pub mem_some: f32,
    pub mem_full: f32,
}

fn psi_avg10(resource: &str, kind: &str) -> f32 {
    let Ok(text) = fs::read_to_string(format!("/proc/pressure/{resource}")) else {
        return 0.0;
    };
    text.lines()
        .find(|l| l.starts_with(kind))
        .and_then(|l| l.split_whitespace().find_map(|f| f.strip_prefix("avg10=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

pub fn read_pressure() -> Pressure {
    Pressure {
        cpu_some: psi_avg10("cpu", "some"),
        io_some: psi_avg10("io", "some"),
        io_full: psi_avg10("io", "full"),
        mem_some: psi_avg10("memory", "some"),
        mem_full: psi_avg10("memory", "full"),
    }
}

// ---- reboot / restart needed ----

#[derive(Serialize, Clone)]
pub struct StaleApp {
    pub name: String,
    pub process_count: usize,
}

#[derive(Serialize, Clone)]
pub struct RebootStatus {
    pub reboot_needed: bool,
    pub reasons: Vec<String>,
    pub running_kernel: String,
    /// Apps still running a binary that an update has since replaced —
    /// restarting just the app is enough for these.
    pub stale_apps: Vec<StaleApp>,
}

fn installed_nvidia_userspace_version() -> Option<String> {
    for dir in ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu"] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(version) = name.strip_prefix("libnvidia-ml.so.") {
                // Skip the bare soname symlink (libnvidia-ml.so.1).
                if version.contains('.') {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}

pub fn reboot_status() -> RebootStatus {
    let running_kernel = read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_default();
    let mut reasons = Vec::new();

    let modules_present = ["/usr/lib/modules", "/lib/modules"]
        .iter()
        .any(|base| std::path::Path::new(&format!("{base}/{running_kernel}")).exists());
    if !running_kernel.is_empty() && !modules_present {
        reasons.push(format!(
            "Kernel {running_kernel} is still running but an update removed its modules — new devices and drivers can't load until you reboot"
        ));
    }
    if std::path::Path::new("/var/run/reboot-required").exists() {
        reasons.push("The package manager flagged that a reboot is required".to_string());
    }
    if let (Some(loaded), Some(installed)) = (
        read_trimmed("/sys/module/nvidia/version"),
        installed_nvidia_userspace_version(),
    ) {
        if loaded != installed {
            reasons.push(format!(
                "NVIDIA driver mismatch: kernel module {loaded} is loaded but libraries are {installed} — GPU apps will fail until you reboot"
            ));
        }
    }

    let mut stale: HashMap<String, usize> = HashMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(link) = fs::read_link(entry.path().join("exe")) else {
                continue;
            };
            let link = link.to_string_lossy().to_string();
            if let Some(path) = link.strip_suffix(" (deleted)") {
                // memfd-backed and temp binaries are "deleted" by design.
                if path.starts_with("/usr/") || path.starts_with("/opt/") {
                    let name = path.rsplit('/').next().unwrap_or(path).to_string();
                    *stale.entry(name).or_default() += 1;
                }
            }
        }
    }
    let mut stale_apps: Vec<StaleApp> = stale
        .into_iter()
        .map(|(name, process_count)| StaleApp { name, process_count })
        .collect();
    stale_apps.sort_by(|a, b| b.process_count.cmp(&a.process_count).then(a.name.cmp(&b.name)));

    RebootStatus {
        reboot_needed: !reasons.is_empty(),
        reasons,
        running_kernel,
        stale_apps,
    }
}

// ---- journal: OOM kills, errors, failed units ----

#[derive(Serialize, Clone)]
pub struct LogLine {
    pub time: String,
    pub source: String,
    pub message: String,
}

/// Parses `journalctl -o short-iso` lines: "<iso time> <host> <source>: <message>".
fn parse_journal(text: &str) -> Vec<LogLine> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let time = parts.next()?;
            let _host = parts.next()?;
            let rest = parts.next()?;
            let (source, message) = rest.split_once(": ").unwrap_or(("", rest));
            // Not a timestamped entry (e.g. "-- No entries --").
            if !time.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            Some(LogLine {
                time: time.to_string(),
                source: source.split('[').next().unwrap_or(source).to_string(),
                message: message.to_string(),
            })
        })
        .collect()
}

pub fn oom_kills() -> Vec<LogLine> {
    let out = run_lossy(Command::new("journalctl").args([
        "--since=-7d",
        "-o",
        "short-iso",
        "--no-pager",
        "-q",
        "-g",
        "Out of memory: Killed process|Killed .* due to memory (pressure|used)",
    ]));
    let mut lines = parse_journal(&out);
    lines.reverse();
    lines.truncate(50);
    lines
}

pub fn recent_errors() -> Vec<LogLine> {
    let out = run_lossy(Command::new("journalctl").args([
        "-p", "3", "-b", "-n", "60", "-o", "short-iso", "--no-pager", "-q",
    ]));
    let mut lines = parse_journal(&out);
    lines.reverse();
    lines
}

#[derive(Serialize, Clone)]
pub struct FailedUnit {
    pub name: String,
    pub description: String,
    pub user: bool,
}

pub fn failed_units() -> Vec<FailedUnit> {
    let mut units = Vec::new();
    for user in [false, true] {
        let mut cmd = Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        let out = run_lossy(cmd.args(["--failed", "--no-legend", "--plain", "--no-pager"]));
        for line in out.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 4 {
                continue;
            }
            units.push(FailedUnit {
                name: fields[0].to_string(),
                description: fields[4..].join(" "),
                user,
            });
        }
    }
    units
}

// ---- battery & power ----

#[derive(Serialize, Clone)]
pub struct BatteryHealth {
    pub name: String,
    pub status: String,
    pub percent: f32,
    /// Positive while discharging; the rate the battery is charging at otherwise.
    pub power_watts: f32,
    pub hours_remaining: Option<f32>,
    pub full_wh: f32,
    pub design_wh: f32,
    pub wear_percent: f32,
    pub cycle_count: Option<u32>,
    pub charge_limit_percent: Option<u32>,
}

pub fn battery_health() -> Vec<BatteryHealth> {
    let Ok(entries) = fs::read_dir("/sys/class/power_supply") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let base = entry.path().to_string_lossy().to_string();
        if read_trimmed(&format!("{base}/type")).as_deref() != Some("Battery") {
            continue;
        }
        let num = |f: &str| read_num::<f64>(&format!("{base}/{f}"));
        let volts = num("voltage_now").unwrap_or(0.0) / 1e6;
        // Drivers report either energy (µWh) or charge (µAh); normalise to Wh.
        let wh = |energy: &str, charge: &str| {
            num(energy)
                .map(|v| v / 1e6)
                .or_else(|| num(charge).map(|v| v / 1e6 * volts))
                .unwrap_or(0.0)
        };
        let now_wh = wh("energy_now", "charge_now");
        let full_wh = wh("energy_full", "charge_full");
        let design_wh = wh("energy_full_design", "charge_full_design");
        let power = num("power_now")
            .map(|v| v / 1e6)
            .or_else(|| num("current_now").map(|v| v / 1e6 * volts))
            .unwrap_or(0.0);
        let status = read_trimmed(&format!("{base}/status")).unwrap_or_default();
        let hours_remaining = if status == "Discharging" && power > 0.1 {
            Some((now_wh / power) as f32)
        } else {
            None
        };
        result.push(BatteryHealth {
            name,
            status,
            percent: if full_wh > 0.0 { (now_wh / full_wh * 100.0) as f32 } else { 0.0 },
            power_watts: power as f32,
            hours_remaining,
            full_wh: full_wh as f32,
            design_wh: design_wh as f32,
            wear_percent: if design_wh > 0.0 {
                ((1.0 - full_wh / design_wh) * 100.0).max(0.0) as f32
            } else {
                0.0
            },
            cycle_count: num("cycle_count").map(|v| v as u32).filter(|c| *c > 0),
            charge_limit_percent: num("charge_control_end_threshold").map(|v| v as u32),
        });
    }
    result
}

// ---- boot time ----

#[derive(Serialize, Clone)]
pub struct BootTime {
    pub boot_id: String,
    pub total_ms: u64,
    pub userspace_ms: u64,
}

/// "1min 2.345s" / "3.790s" / "234ms" -> milliseconds.
fn parse_duration_ms(s: &str) -> u64 {
    s.split_whitespace()
        .map(|token| {
            let split = token.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(token.len());
            let value: f64 = token[..split].parse().unwrap_or(0.0);
            match &token[split..] {
                "min" => value * 60_000.0,
                "s" => value * 1_000.0,
                "ms" => value,
                _ => 0.0,
            }
        })
        .sum::<f64>() as u64
}

pub fn boot_time() -> Option<BootTime> {
    let out = run_lossy(Command::new("systemd-analyze").arg("time"));
    let line = out.lines().next()?;
    let (segments, total) = line.strip_prefix("Startup finished in ")?.rsplit_once(" = ")?;
    let userspace = segments
        .split(" + ")
        .find_map(|seg| seg.strip_suffix(" (userspace)"))
        .map(parse_duration_ms)
        .unwrap_or(0);
    Some(BootTime {
        boot_id: read_trimmed("/proc/sys/kernel/random/boot_id")?,
        total_ms: parse_duration_ms(total),
        userspace_ms: userspace,
    })
}

// ---- drives ----

#[derive(Serialize, Clone)]
pub struct Drive {
    pub device: String,
    pub model: String,
    pub size_bytes: u64,
    pub rotational: bool,
}

pub fn list_drives() -> Vec<Drive> {
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return Vec::new();
    };
    let mut drives: Vec<Drive> = entries
        .flatten()
        .filter_map(|entry| {
            let device = entry.file_name().to_string_lossy().to_string();
            let base = format!("/sys/block/{device}");
            // Only real hardware has a backing device node (skips loop, zram, dm-*).
            if !std::path::Path::new(&format!("{base}/device")).exists() {
                return None;
            }
            Some(Drive {
                model: read_trimmed(&format!("{base}/device/model")).unwrap_or_default(),
                size_bytes: read_num::<u64>(&format!("{base}/size")).unwrap_or(0) * 512,
                rotational: read_num::<u8>(&format!("{base}/queue/rotational")) == Some(1),
                device,
            })
        })
        .filter(|d| d.size_bytes > 0)
        .collect();
    drives.sort_by(|a, b| a.device.cmp(&b.device));
    drives
}

#[derive(Serialize, Clone)]
pub struct DriveHealth {
    pub device: String,
    pub passed: Option<bool>,
    pub temperature_c: Option<i64>,
    pub power_on_hours: Option<i64>,
    /// NVMe "percentage used" of rated endurance.
    pub wear_percent: Option<i64>,
    pub available_spare_percent: Option<i64>,
    pub media_errors: Option<i64>,
}

/// SMART data is root-only, so this goes through one polkit prompt for all drives.
pub fn drive_health() -> Result<Vec<DriveHealth>, String> {
    let drives = list_drives();
    if drives.is_empty() {
        return Ok(Vec::new());
    }
    let script = drives
        .iter()
        .filter(|d| crate::sys_util::is_safe_token(&d.device))
        .map(|d| format!("smartctl -H -A -j /dev/{} || true; echo '---TRACE---'", d.device))
        .collect::<Vec<_>>()
        .join("; ");
    let out = run_privileged(&script)?;
    Ok(out
        .split("---TRACE---")
        .zip(drives.iter())
        .filter_map(|(chunk, drive)| {
            let v: serde_json::Value = serde_json::from_str(chunk.trim()).ok()?;
            let nvme = &v["nvme_smart_health_information_log"];
            Some(DriveHealth {
                device: drive.device.clone(),
                passed: v["smart_status"]["passed"].as_bool(),
                temperature_c: v["temperature"]["current"].as_i64(),
                power_on_hours: v["power_on_time"]["hours"].as_i64(),
                wear_percent: nvme["percentage_used"].as_i64(),
                available_spare_percent: nvme["available_spare"].as_i64(),
                media_errors: nvme["media_errors"].as_i64(),
            })
        })
        .collect())
}

// ---- health score ----

#[derive(Serialize, Clone)]
pub struct Issue {
    /// "critical" | "warn" | "info"
    pub severity: String,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct HealthReport {
    pub score: u32,
    pub grade: String,
    pub issues: Vec<Issue>,
}

pub struct HealthInputs {
    pub mem_available_percent: f32,
    pub swap_used_percent: f32,
    /// (mount point, used percent)
    pub disks: Vec<(String, f32)>,
    pub max_temp_c: f32,
    pub pressure: Pressure,
    pub failed_units: usize,
    pub reboot_needed: bool,
    pub battery_wear_percent: f32,
}

pub fn health_report(i: &HealthInputs) -> HealthReport {
    let mut score: i32 = 100;
    let mut issues = Vec::new();
    let mut add = |penalty: i32, severity: &str, title: String, detail: &str| {
        score -= penalty;
        issues.push(Issue {
            severity: severity.to_string(),
            title,
            detail: detail.to_string(),
        });
    };

    if i.mem_available_percent < 8.0 {
        add(20, "critical", format!("Only {:.0}% of RAM is available", i.mem_available_percent),
            "Apps will stall or be killed soon. Close something large from the Memory page.");
    } else if i.mem_available_percent < 18.0 {
        add(8, "warn", format!("RAM is getting tight ({:.0}% available)", i.mem_available_percent),
            "Still fine, but new apps will start pushing others into swap.");
    }
    if i.swap_used_percent > 80.0 {
        add(12, "critical", format!("Swap is {:.0}% full", i.swap_used_percent),
            "Once swap fills the kernel starts killing processes.");
    } else if i.swap_used_percent > 50.0 {
        add(4, "info", format!("Swap is {:.0}% used", i.swap_used_percent),
            "Old app data has been pushed out of RAM; switching back to those apps may be slow.");
    }
    for (mount, used) in &i.disks {
        if *used > 92.0 {
            add(15, "critical", format!("{mount} is {used:.0}% full"),
                "Free space with the Disk Usage Explorer on the History page.");
        } else if *used > 82.0 {
            add(5, "warn", format!("{mount} is {used:.0}% full"), "Worth cleaning up before it becomes urgent.");
        }
    }
    if i.max_temp_c > 92.0 {
        add(15, "critical", format!("Running hot: {:.0}°C", i.max_temp_c),
            "The CPU/GPU will throttle itself. Check fans and what is using the CPU.");
    } else if i.max_temp_c > 82.0 {
        add(5, "warn", format!("Warm: {:.0}°C", i.max_temp_c), "Expected under heavy load; watch it if idle.");
    }
    if i.pressure.mem_some > 15.0 {
        add(15, "critical", format!("Work is stalling on memory {:.0}% of the time", i.pressure.mem_some),
            "This is what makes the desktop freeze. Free RAM now.");
    } else if i.pressure.mem_some > 3.0 {
        add(5, "warn", format!("Occasional stalls waiting for memory ({:.0}%)", i.pressure.mem_some),
            "The kernel is reclaiming memory to keep up.");
    }
    if i.pressure.io_some > 30.0 {
        add(8, "warn", format!("Work is stalling on disk {:.0}% of the time", i.pressure.io_some),
            "See the Activity page for who is reading and writing.");
    }
    if i.pressure.cpu_some > 60.0 {
        add(5, "warn", format!("CPU is oversubscribed ({:.0}% of the time tasks wait for a core)", i.pressure.cpu_some),
            "More runnable work than cores.");
    }
    if i.failed_units > 0 {
        add((i.failed_units as i32 * 4).min(12), "warn",
            format!("{} system service(s) have failed", i.failed_units), "Listed below under Failed services.");
    }
    if i.reboot_needed {
        add(10, "warn", "A reboot is needed".to_string(), "An update replaced the running kernel or GPU driver.");
    }
    if i.battery_wear_percent > 30.0 {
        add(4, "info", format!("Battery has lost {:.0}% of its original capacity", i.battery_wear_percent),
            "Normal ageing; runtime on battery is shorter than when new.");
    }

    let score = score.clamp(0, 100) as u32;
    let grade = match score {
        90..=100 => "Healthy",
        70..=89 => "Fair",
        45..=69 => "Strained",
        _ => "Critical",
    };
    HealthReport {
        score,
        grade: grade.to_string(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_systemd_durations() {
        assert_eq!(parse_duration_ms("3.790s"), 3790);
        assert_eq!(parse_duration_ms("1min 2.5s"), 62_500);
        assert_eq!(parse_duration_ms("234ms"), 234);
    }

    #[test]
    fn parses_journal_lines() {
        let lines = parse_journal(
            "2026-10-09T10:00:00+05:30 host kernel: Out of memory: Killed process 12 (brave)\n-- No entries --",
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].source, "kernel");
        assert!(lines[0].message.starts_with("Out of memory"));
    }

    #[test]
    fn healthy_machine_scores_100() {
        let report = health_report(&HealthInputs {
            mem_available_percent: 60.0,
            swap_used_percent: 0.0,
            disks: vec![("/".to_string(), 40.0)],
            max_temp_c: 50.0,
            pressure: Pressure::default(),
            failed_units: 0,
            reboot_needed: false,
            battery_wear_percent: 5.0,
        });
        assert_eq!(report.score, 100);
        assert!(report.issues.is_empty());
    }
}
