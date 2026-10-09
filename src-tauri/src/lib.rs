mod activity;
mod automation;
mod autostart;
mod connectivity;
mod control_process;
mod db;
mod desktop;
mod docker_control;
mod gpu_control;
mod health;
mod hypr;
mod logbook;
mod memory_map;
mod profiles;
mod search;
mod projects;
mod namespace_control;
mod network_control;
mod packages;
mod storage_explorer;
mod sys_util;
mod system_control;
mod systemd_control;
mod tuning;
mod tweak_catalog;
mod tweak_presets;
mod tweaks;
mod update_check;
mod usb_control;
mod vpn;
mod window_control;

use serde::Serialize;
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Disks, System};
use tauri::Manager;

#[derive(Serialize, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    memory_bytes: u64,
}

#[derive(Serialize, Clone)]
struct DiskInfo {
    name: String,
    mount_point: String,
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(Serialize, Clone)]
struct GpuInfo {
    name: String,
    vendor: String,
    usage_percent: f32,
    vram_used_bytes: u64,
    vram_total_bytes: u64,
    temperature_c: Option<f32>,
    power_draw_watts: Option<f32>,
}

#[derive(Serialize, Clone)]
struct TempSensor {
    label: String,
    celsius: f32,
}

#[derive(Serialize, Clone)]
struct NetworkInfo {
    interface: String,
    rx_bytes_per_sec: u64,
    tx_bytes_per_sec: u64,
}

#[derive(Serialize, Clone)]
struct BatteryInfo {
    percent: u8,
    status: String,
}

#[derive(Serialize, Clone)]
struct Snapshot {
    cpu_usage_percent: f32,
    per_core_usage: Vec<f32>,
    per_core_freq_mhz: Vec<u64>,
    total_memory_bytes: u64,
    used_memory_bytes: u64,
    available_memory_bytes: u64,
    total_swap_bytes: u64,
    used_swap_bytes: u64,
    load_average: (f64, f64, f64),
    disks: Vec<DiskInfo>,
    top_processes: Vec<ProcessInfo>,
    uptime_secs: u64,
    gpus: Vec<GpuInfo>,
    temperatures: Vec<TempSensor>,
    networks: Vec<NetworkInfo>,
    battery: Option<BatteryInfo>,
    pressure: health::Pressure,
}

struct NetSample {
    rx_bytes: u64,
    tx_bytes: u64,
    at: Instant,
}

struct AppState {
    sys: Mutex<System>,
    last_net: Mutex<std::collections::HashMap<String, NetSample>>,
    db: db::Db,
    last_snapshot: Mutex<Option<Snapshot>>,
    clipboard_history: Mutex<std::collections::VecDeque<String>>,
    activity: Mutex<activity::ActivityState>,
    automation: Mutex<automation::State>,
    slow_health: Mutex<SlowHealth>,
    battery_prev: Mutex<logbook::BatteryPrev>,
    /// The tray icon and the menu line that shows live numbers, once built.
    tray: Mutex<Option<TrayHandles>>,
    net_totals: Mutex<std::collections::HashMap<String, (u64, u64)>>,
}

/// Health facts that are too expensive to recompute on every 10s tick
/// (they spawn `systemctl` and walk `/proc`), refreshed every few minutes.
#[derive(Default, Clone)]
struct SlowHealth {
    failed_units: usize,
    reboot_needed: bool,
}

const CLIPBOARD_HISTORY_LIMIT: usize = 50;

fn poll_clipboard(app_state: &AppState) {
    let Ok(out) = Command::new("wl-paste").args(["--type", "text", "--no-newline"]).output() else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 20_000 {
        return;
    }
    let mut history = app_state.clipboard_history.lock().unwrap();
    if history.front().map(|s| s.as_str()) == Some(trimmed) {
        return;
    }
    history.push_front(trimmed.to_string());
    history.truncate(CLIPBOARD_HISTORY_LIMIT);
}

/// Excludes mount points that aren't real storage a user would care about:
/// AppImage FUSE mounts (`/tmp/.mount_*`), snap loopback mounts, and other
/// transient bind/overlay mounts that show up as "disks" but are actually
/// just an app's own squashfs image mounted for its lifetime.
fn is_ephemeral_mount(mount_point: &str) -> bool {
    mount_point.contains("/.mount_")
        || mount_point.starts_with("/snap/")
        || mount_point.starts_with("/var/lib/docker/")
        || mount_point.starts_with("/run/")
}

fn read_amd_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return gpus;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device = entry.path().join("device");
        let vendor = fs::read_to_string(device.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if vendor != "0x1002" {
            continue;
        }
        let busy: f32 = fs::read_to_string(device.join("gpu_busy_percent"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let vram_used: u64 = fs::read_to_string(device.join("mem_info_vram_used"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let vram_total: u64 = fs::read_to_string(device.join("mem_info_vram_total"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        gpus.push(GpuInfo {
            name: "AMD GPU".to_string(),
            vendor: "AMD".to_string(),
            usage_percent: busy,
            vram_used_bytes: vram_used,
            vram_total_bytes: vram_total,
            temperature_c: None,
            power_draw_watts: None,
        });
    }
    gpus
}

fn read_nvidia_gpus() -> Vec<GpuInfo> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() < 6 {
                return None;
            }
            Some(GpuInfo {
                name: parts[0].to_string(),
                vendor: "NVIDIA".to_string(),
                usage_percent: parts[1].parse().unwrap_or(0.0),
                vram_used_bytes: parts[2].parse::<u64>().unwrap_or(0) * 1024 * 1024,
                vram_total_bytes: parts[3].parse::<u64>().unwrap_or(0) * 1024 * 1024,
                temperature_c: parts[4].parse().ok(),
                power_draw_watts: parts[5].parse().ok(),
            })
        })
        .collect()
}

fn read_temperatures() -> Vec<TempSensor> {
    let mut temps = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
        return temps;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let chip = fs::read_to_string(dir.join("name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let Ok(files) = fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.starts_with("temp") || !fname.ends_with("_input") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(f.path()) else {
                continue;
            };
            let Ok(millideg) = raw.trim().parse::<i64>() else {
                continue;
            };
            let label_file = fname.replace("_input", "_label");
            let label = fs::read_to_string(dir.join(&label_file))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| chip.clone());
            temps.push(TempSensor {
                label: format!("{} {}", chip, label),
                celsius: millideg as f32 / 1000.0,
            });
        }
    }
    temps
}

fn read_battery() -> Option<BatteryInfo> {
    let base = "/sys/class/power_supply/BAT0";
    let percent: u8 = fs::read_to_string(format!("{base}/capacity"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let status = fs::read_to_string(format!("{base}/status"))
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(BatteryInfo { percent, status })
}

fn read_networks(
    last: &mut std::collections::HashMap<String, NetSample>,
) -> Vec<NetworkInfo> {
    let mut result = Vec::new();
    let Ok(text) = fs::read_to_string("/proc/net/dev") else {
        return result;
    };
    let now = Instant::now();
    for line in text.lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim().to_string();
        if iface == "lo" {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 9 {
            continue;
        }
        let rx_bytes: u64 = fields[0].parse().unwrap_or(0);
        let tx_bytes: u64 = fields[8].parse().unwrap_or(0);

        let (rx_rate, tx_rate) = if let Some(prev) = last.get(&iface) {
            let dt = now.duration_since(prev.at).as_secs_f64().max(0.001);
            (
                ((rx_bytes.saturating_sub(prev.rx_bytes)) as f64 / dt) as u64,
                ((tx_bytes.saturating_sub(prev.tx_bytes)) as f64 / dt) as u64,
            )
        } else {
            (0, 0)
        };

        last.insert(
            iface.clone(),
            NetSample {
                rx_bytes,
                tx_bytes,
                at: now,
            },
        );

        if rx_bytes > 0 || tx_bytes > 0 {
            result.push(NetworkInfo {
                interface: iface,
                rx_bytes_per_sec: rx_rate,
                tx_bytes_per_sec: tx_rate,
            });
        }
    }
    result
}

fn take_snapshot(state: &AppState) -> Snapshot {
    let mut sys = state.sys.lock().unwrap();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let disks = Disks::new_with_refreshed_list();
    let disk_infos = disks
        .iter()
        .filter(|d| !is_ephemeral_mount(&d.mount_point().to_string_lossy()))
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().to_string(),
            mount_point: d.mount_point().to_string_lossy().to_string(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
        })
        .collect();

    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        // sysinfo lists threads as well; they'd repeat their process's memory.
        .filter(|p| p.thread_kind().is_none())
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_usage: p.cpu_usage(),
            memory_bytes: p.memory(),
        })
        .collect();
    processes.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    processes.truncate(15);

    let load = System::load_average();

    let mut gpus = read_nvidia_gpus();
    gpus.extend(read_amd_gpus());

    let mut last_net = state.last_net.lock().unwrap();
    let networks = read_networks(&mut last_net);

    Snapshot {
        cpu_usage_percent: sys.global_cpu_usage(),
        per_core_usage: sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
        per_core_freq_mhz: sys.cpus().iter().map(|c| c.frequency()).collect(),
        total_memory_bytes: sys.total_memory(),
        used_memory_bytes: sys.used_memory(),
        available_memory_bytes: sys.available_memory(),
        total_swap_bytes: sys.total_swap(),
        used_swap_bytes: sys.used_swap(),
        load_average: (load.one, load.five, load.fifteen),
        disks: disk_infos,
        top_processes: processes,
        uptime_secs: System::uptime(),
        gpus,
        temperatures: read_temperatures(),
        networks,
        battery: read_battery(),
        pressure: health::read_pressure(),
    }
}

#[tauri::command(async)]
fn get_snapshot(state: tauri::State<AppState>) -> Snapshot {
    take_snapshot(&state)
}

#[tauri::command(async)]
fn get_resource_history(state: tauri::State<AppState>, since_secs_ago: i64) -> Vec<db::ResourcePoint> {
    let since_ts = chrono_now() - since_secs_ago;
    state.db.resource_history(since_ts)
}

#[tauri::command(async)]
fn get_disk_history(state: tauri::State<AppState>, since_secs_ago: i64) -> Vec<db::DiskPoint> {
    let since_ts = chrono_now() - since_secs_ago;
    state.db.disk_history(since_ts)
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn record_snapshot(app_state: &AppState, snap: &Snapshot) {
    let ts = chrono_now();
    let gpu = snap.gpus.first();
    app_state.db.insert_resource(
        ts,
        snap.cpu_usage_percent,
        snap.used_memory_bytes,
        snap.total_memory_bytes,
        snap.used_swap_bytes,
        gpu.map(|g| g.usage_percent),
        gpu.map(|g| g.vram_used_bytes),
    );
    app_state.db.insert_pressure(
        ts,
        snap.pressure.cpu_some,
        snap.pressure.io_some,
        snap.pressure.mem_some,
    );
    logbook::record_sample(
        &app_state.db,
        ts,
        &logbook::Sample {
            cpu_percent: snap.cpu_usage_percent,
            ram_percent: snap.used_memory_bytes as f32 / snap.total_memory_bytes.max(1) as f32 * 100.0,
            swap_bytes: snap.used_swap_bytes,
            cpu_stall: snap.pressure.cpu_some,
            mem_stall: snap.pressure.mem_some,
            io_stall: snap.pressure.io_some,
            temp_c: snap.temperatures.iter().map(|t| t.celsius).fold(0.0, f32::max),
        },
    );
    for d in &snap.disks {
        app_state.db.insert_disk(
            ts,
            &d.mount_point,
            d.total_bytes - d.available_bytes,
            d.total_bytes,
        );
    }

    let mut last = app_state.last_snapshot.lock().unwrap();
    if let Some(prev) = last.as_ref() {
        detect_events(app_state, prev, snap, ts);
    }
    *last = Some(snap.clone());
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

fn detect_events(app_state: &AppState, prev: &Snapshot, curr: &Snapshot, ts: i64) {
    let db = &app_state.db;

    let ram_delta_gb =
        (curr.used_memory_bytes as f64 - prev.used_memory_bytes as f64) / GB;
    if ram_delta_gb > 1.0 {
        db.insert_event(
            ts,
            "ram_increase",
            "warn",
            &format!("RAM usage increased {:.1} GB", ram_delta_gb),
        );
    }

    if prev.used_swap_bytes == 0 && curr.used_swap_bytes > 0 {
        db.insert_event(
            ts,
            "swap_activated",
            "warn",
            "Swap usage began (system started paging memory to disk)",
        );
    }

    if curr.cpu_usage_percent > 90.0 && prev.cpu_usage_percent <= 90.0 {
        db.insert_event(
            ts,
            "cpu_high",
            "warn",
            &format!("CPU usage spiked to {:.0}%", curr.cpu_usage_percent),
        );
    }

    let core_count = curr.per_core_usage.len().max(1) as f64;
    if curr.load_average.0 > core_count && prev.load_average.0 <= core_count {
        db.insert_event(
            ts,
            "load_high",
            "warn",
            &format!(
                "Load average ({:.2}) exceeded core count ({})",
                curr.load_average.0, curr.per_core_usage.len()
            ),
        );
    }

    if curr.pressure.mem_some > 10.0 && prev.pressure.mem_some <= 10.0 {
        db.insert_event(
            ts,
            "memory_stall",
            "warn",
            &format!(
                "Tasks stalled waiting for memory {:.0}% of the time",
                curr.pressure.mem_some
            ),
        );
    }

    if curr.pressure.io_some > 25.0 && prev.pressure.io_some <= 25.0 {
        db.insert_event(
            ts,
            "io_stall",
            "warn",
            &format!(
                "Tasks stalled waiting for disk {:.0}% of the time",
                curr.pressure.io_some
            ),
        );
    }

    for d in &curr.disks {
        if let Some(pd) = prev.disks.iter().find(|p| p.mount_point == d.mount_point) {
            let delta_gb = (pd.available_bytes as f64 - d.available_bytes as f64) / GB;
            if delta_gb > 0.5 {
                db.insert_event(
                    ts,
                    "disk_growth",
                    "info",
                    &format!("{} used {:.1} GB more disk space", d.mount_point, delta_gb),
                );
            }
        }
    }

    for g in &curr.gpus {
        if let Some(pg) = prev.gpus.iter().find(|p| p.name == g.name) {
            let vram_delta_gb =
                (g.vram_used_bytes as f64 - pg.vram_used_bytes as f64) / GB;
            if vram_delta_gb > 0.5 {
                db.insert_event(
                    ts,
                    "vram_increase",
                    "info",
                    &format!("{} VRAM usage increased {:.1} GB", g.name, vram_delta_gb),
                );
            }
        }
    }
}

#[derive(Serialize)]
struct Diagnosis {
    cause: String,
    confidence: String,
    evidence: Vec<String>,
    events: Vec<db::EventRow>,
}

#[tauri::command(async)]
fn diagnose_slowness(state: tauri::State<AppState>) -> Diagnosis {
    let since = chrono_now() - 3600;
    let events = state.db.events_since(since);

    let has = |t: &str| events.iter().any(|e| e.event_type == t);
    let count = |t: &str| events.iter().filter(|e| e.event_type == t).count();

    let (cause, confidence, evidence) = if has("memory_stall") {
        (
            "Memory pressure — work is stalling while the kernel frees RAM".to_string(),
            "HIGH".to_string(),
            vec![
                format!("Memory stalls were measured {} time(s) in the last hour", count("memory_stall")),
                "This is direct evidence of slowdown, not just high usage".to_string(),
                "The Memory page shows which app to close".to_string(),
            ],
        )
    } else if has("io_stall") {
        (
            "Disk is the bottleneck — work is stalling on reads and writes".to_string(),
            "HIGH".to_string(),
            vec![
                format!("Disk stalls were measured {} time(s) in the last hour", count("io_stall")),
                "The Activity page shows which app is reading or writing".to_string(),
            ],
        )
    } else if has("swap_activated") && has("ram_increase") {
        (
            "Memory pressure → swapping".to_string(),
            "HIGH".to_string(),
            vec![
                format!("RAM increased in {} interval(s)", count("ram_increase")),
                "Swap usage activated after RAM growth".to_string(),
                "Disk I/O and responsiveness typically drop once swapping starts"
                    .to_string(),
            ],
        )
    } else if has("swap_activated") {
        (
            "Swap activated — memory is under pressure".to_string(),
            "MEDIUM".to_string(),
            vec!["System began paging memory to disk".to_string()],
        )
    } else if count("cpu_high") >= 2 {
        (
            "Sustained high CPU usage".to_string(),
            "MEDIUM".to_string(),
            vec![format!("CPU crossed 90% {} times in the last hour", count("cpu_high"))],
        )
    } else if has("load_high") {
        (
            "Load average exceeded available cores".to_string(),
            "MEDIUM".to_string(),
            vec!["More runnable processes than CPU cores can service at once".to_string()],
        )
    } else if has("ram_increase") {
        (
            "Memory usage rising but no swap yet".to_string(),
            "LOW".to_string(),
            vec![format!("RAM grew in {} interval(s), no swap triggered", count("ram_increase"))],
        )
    } else {
        (
            "No clear performance incident detected in the last hour".to_string(),
            "LOW".to_string(),
            vec!["Resource usage has stayed within normal bounds".to_string()],
        )
    };

    Diagnosis {
        cause,
        confidence,
        evidence,
        events,
    }
}

#[derive(Serialize, Clone)]
struct PortOwner {
    pid: u32,
    process_name: String,
    local_address: String,
}

fn parse_hex_port(hex_addr: &str) -> Option<(String, u16)> {
    let (ip_hex, port_hex) = hex_addr.split_once(':')?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    Some((ip_hex.to_string(), port))
}

fn find_inodes_for_port(port: u16) -> Vec<String> {
    let mut inodes = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            if let Some((_, p)) = parse_hex_port(fields[1]) {
                if p == port {
                    inodes.push(fields[9].to_string());
                }
            }
        }
    }
    inodes
}

#[tauri::command(async)]
fn scan_projects(root: String) -> Vec<projects::ProjectInfo> {
    projects::scan(&root)
}

fn log_action(state: &AppState, action_desc: &str, ok: bool, message: &str) {
    state.db.insert_action(chrono_now(), action_desc, ok, message);
}

fn as_result(r: control_process::ActionResult) -> Result<String, String> {
    if r.ok { Ok(r.message) } else { Err(r.message) }
}

async fn execute_action(state: &AppState, action: &profiles::Action) -> Result<String, String> {
    use profiles::Action::*;
    match action {
        FreezeProcess { pid } => {
            let r = control_process::freeze(*pid);
            if r.ok { Ok(r.message) } else { Err(r.message) }
        }
        ResumeProcess { pid } => {
            let r = control_process::resume(*pid);
            if r.ok { Ok(r.message) } else { Err(r.message) }
        }
        KillProcess { pid, force } => {
            let r = control_process::kill(*pid, *force);
            if r.ok { Ok(r.message) } else { Err(r.message) }
        }
        SetCpuLimit { pid, percent } => {
            let r = control_process::set_cpu_limit(*pid, *percent);
            if r.ok { Ok(r.message) } else { Err(r.message) }
        }
        SetMemoryLimitMb { pid, mb } => {
            let r = control_process::set_memory_limit(*pid, *mb);
            if r.ok { Ok(r.message) } else { Err(r.message) }
        }
        StartService { name } => systemd_control::start_unit(name).await,
        StopService { name } => systemd_control::stop_unit(name).await,
        RestartService { name } => systemd_control::restart_unit(name).await,
        StartContainer { id } => docker_control::start_container(id).await,
        StopContainer { id } => docker_control::stop_container(id).await,
        FreezeApp { app } => as_result(memory_map::app_action(app, "freeze")),
        ResumeApp { app } => as_result(memory_map::app_action(app, "resume")),
        QuitApp { app } => as_result(memory_map::app_action(app, "kill")),
        SetPowerProfile { profile } => as_result(tuning::set_power_profile(profile)),
        ApplyPreset { preset } => {
            let changes = tweak_presets::changes_for(preset, &tweaks::list(&state.db))
                .ok_or_else(|| format!("No settings preset called {preset}"))?;
            if changes.is_empty() {
                return Ok("Everything in the preset was already set".to_string());
            }
            let results = tweaks::apply(&state.db, &changes, &format!("preset:{preset}"), None);
            let failed: Vec<String> = results.iter().filter(|r| !r.ok).map(|r| format!("{} ({})", r.title, r.message)).collect();
            if failed.is_empty() {
                Ok(format!("Applied {} setting(s)", results.len()))
            } else {
                Err(format!("{} of {} failed: {}", failed.len(), results.len(), failed.join("; ")))
            }
        }
    }
}

#[tauri::command(async)]
fn freeze_process(state: tauri::State<AppState>, pid: u32) -> control_process::ActionResult {
    let r = control_process::freeze(pid);
    log_action(&state, "freeze_process", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn resume_process(state: tauri::State<AppState>, pid: u32) -> control_process::ActionResult {
    let r = control_process::resume(pid);
    log_action(&state, "resume_process", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn kill_process(state: tauri::State<AppState>, pid: u32, force: bool) -> control_process::ActionResult {
    let r = control_process::kill(pid, force);
    log_action(&state, "kill_process", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_process_priority(state: tauri::State<AppState>, pid: u32, nice: i32) -> control_process::ActionResult {
    let r = control_process::set_priority(pid, nice);
    log_action(&state, "set_process_priority", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_process_affinity(state: tauri::State<AppState>, pid: u32, cores: Vec<usize>) -> control_process::ActionResult {
    let r = control_process::set_affinity(pid, cores);
    log_action(&state, "set_process_affinity", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_process_cpu_limit(state: tauri::State<AppState>, pid: u32, percent: u32) -> control_process::ActionResult {
    let r = control_process::set_cpu_limit(pid, percent);
    log_action(&state, "set_process_cpu_limit", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_process_memory_limit(state: tauri::State<AppState>, pid: u32, mb: u64) -> control_process::ActionResult {
    let r = control_process::set_memory_limit(pid, mb);
    log_action(&state, "set_process_memory_limit", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_process_oom_score(state: tauri::State<AppState>, pid: u32, score: i32) -> control_process::ActionResult {
    let r = control_process::set_oom_score_adj(pid, score);
    log_action(&state, "set_process_oom_score", r.ok, &r.message);
    r
}

#[tauri::command]
async fn list_services(running_or_failed_only: bool) -> Result<Vec<systemd_control::UnitInfo>, String> {
    systemd_control::list_units(running_or_failed_only).await
}

#[tauri::command]
async fn start_service(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    let r = systemd_control::start_unit(&name).await;
    log_action(&state, "start_service", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command]
async fn stop_service(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    let r = systemd_control::stop_unit(&name).await;
    log_action(&state, "stop_service", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command]
async fn restart_service(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    let r = systemd_control::restart_unit(&name).await;
    log_action(&state, "restart_service", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command]
async fn list_containers() -> Result<Vec<docker_control::ContainerInfo>, String> {
    docker_control::list_containers().await
}

#[tauri::command]
async fn start_container(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    let r = docker_control::start_container(&id).await;
    log_action(&state, "start_container", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command]
async fn stop_container(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    let r = docker_control::stop_container(&id).await;
    log_action(&state, "stop_container", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command]
async fn restart_container(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    let r = docker_control::restart_container(&id).await;
    log_action(&state, "restart_container", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command(async)]
fn set_gpu_persistence(state: tauri::State<AppState>, enabled: bool) -> Result<String, String> {
    let r = gpu_control::set_persistence_mode(enabled);
    log_action(&state, "set_gpu_persistence", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command(async)]
fn set_gpu_power_limit(state: tauri::State<AppState>, watts: u32) -> Result<String, String> {
    let r = gpu_control::set_power_limit(watts);
    log_action(&state, "set_gpu_power_limit", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command(async)]
fn list_profiles() -> Vec<profiles::Profile> {
    profiles::list_profiles()
}

#[tauri::command(async)]
fn save_profile(profile: profiles::Profile) -> Result<(), String> {
    profiles::save_profile(&profile)
}

#[derive(Serialize)]
struct ActionOutcome {
    description: String,
    ok: bool,
    message: String,
}

#[tauri::command]
async fn apply_profile(
    state: tauri::State<'_, AppState>,
    profile: profiles::Profile,
    dry_run: bool,
) -> Result<Vec<ActionOutcome>, String> {
    Ok(run_profile(&state, &profile, dry_run).await)
}

async fn run_profile(state: &AppState, profile: &profiles::Profile, dry_run: bool) -> Vec<ActionOutcome> {
    let mut outcomes = Vec::new();
    for action in &profile.actions {
        let description = action.describe();
        if dry_run {
            outcomes.push(ActionOutcome {
                description,
                ok: true,
                message: "(dry run — not executed)".to_string(),
            });
            continue;
        }
        let result = execute_action(state, action).await;
        let (ok, message) = match &result {
            Ok(m) => (true, m.clone()),
            Err(e) => (false, e.clone()),
        };
        log_action(state, &format!("profile:{}", profile.name), ok, &message);
        outcomes.push(ActionOutcome { description, ok, message });
    }
    outcomes
}

#[tauri::command]
async fn undo_profile(
    state: tauri::State<'_, AppState>,
    profile: profiles::Profile,
) -> Result<Vec<ActionOutcome>, String> {
    let mut outcomes = Vec::new();
    for action in profile.actions.iter().rev() {
        let Some(inverse) = action.inverse() else {
            outcomes.push(ActionOutcome {
                description: action.describe(),
                ok: false,
                message: "No safe inverse for this action — skipped".to_string(),
            });
            continue;
        };
        let description = inverse.describe();
        let result = execute_action(&state, &inverse).await;
        let (ok, message) = match &result {
            Ok(m) => (true, m.clone()),
            Err(e) => (false, e.clone()),
        };
        log_action(&state, &format!("undo:{}", profile.name), ok, &message);
        outcomes.push(ActionOutcome { description, ok, message });
    }
    Ok(outcomes)
}

#[tauri::command]
async fn check_for_update() -> Result<Option<update_check::UpdateInfo>, String> {
    update_check::check_for_update("ItzAditya43/Trace").await
}

#[tauri::command(async)]
fn get_action_log(state: tauri::State<AppState>, since_secs_ago: i64) -> Vec<db::ActionRow> {
    state.db.actions_since(chrono_now() - since_secs_ago)
}

#[tauri::command(async)]
fn who_is_using_port(state: tauri::State<AppState>, port: u16) -> Vec<PortOwner> {
    let inodes = find_inodes_for_port(port);
    if inodes.is_empty() {
        return Vec::new();
    }
    let mut owners = Vec::new();
    let Ok(proc_entries) = fs::read_dir("/proc") else {
        return owners;
    };
    let sys = state.sys.lock().unwrap();
    for entry in proc_entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(pid_num) = name.parse::<u32>() else {
            continue;
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else {
                continue;
            };
            let link_str = link.to_string_lossy();
            if let Some(inode) = link_str
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
            {
                if inodes.iter().any(|i| i == inode) {
                    let process_name = sys
                        .process(sysinfo::Pid::from_u32(pid_num))
                        .map(|p| p.name().to_string_lossy().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    owners.push(PortOwner {
                        pid: pid_num,
                        process_name,
                        local_address: format!("port {}", port),
                    });
                    break;
                }
            }
        }
    }
    owners
}

#[tauri::command(async)]
fn kill_process_on_port(
    state: tauri::State<AppState>,
    port: u16,
    force: bool,
) -> control_process::ActionResult {
    let owners = who_is_using_port(tauri::State::clone(&state), port);
    if owners.is_empty() {
        return control_process::ActionResult {
            ok: false,
            message: format!("Nothing is listening on port {port}"),
        };
    }
    let mut messages = Vec::new();
    let mut all_ok = true;
    for o in &owners {
        let r = control_process::kill(o.pid, force);
        all_ok &= r.ok;
        messages.push(format!("{} (PID {}): {}", o.process_name, o.pid, r.message));
    }
    let r = control_process::ActionResult {
        ok: all_ok,
        message: messages.join("; "),
    };
    log_action(&state, "kill_process_on_port", r.ok, &r.message);
    r
}

#[tauri::command(async)]
fn set_ionice(state: tauri::State<AppState>, pid: u32, class: u32, level: u32) -> control_process::ActionResult {
    let r = system_control::set_ionice(pid, class, level);
    log_action(&state, "set_ionice", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn get_brightness() -> Option<system_control::BrightnessInfo> {
    system_control::get_brightness()
}

#[tauri::command(async)]
fn set_brightness(state: tauri::State<AppState>, percent: u32) -> control_process::ActionResult {
    let r = system_control::set_brightness(percent);
    log_action(&state, "set_brightness", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn get_volume() -> Option<system_control::VolumeInfo> {
    system_control::get_volume()
}

#[tauri::command(async)]
fn set_volume(state: tauri::State<AppState>, percent: u32) -> control_process::ActionResult {
    let r = system_control::set_volume(percent);
    log_action(&state, "set_volume", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn toggle_mute(state: tauri::State<AppState>) -> control_process::ActionResult {
    let r = system_control::toggle_mute();
    log_action(&state, "toggle_mute", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn startup_impact() -> Vec<system_control::StartupEntry> {
    system_control::startup_impact()
}

#[tauri::command(async)]
fn list_autostart() -> Vec<system_control::AutostartEntry> {
    system_control::list_autostart()
}

#[tauri::command(async)]
fn set_autostart_enabled(state: tauri::State<AppState>, filename: String, enabled: bool) -> control_process::ActionResult {
    let r = system_control::set_autostart_enabled(&filename, enabled);
    log_action(&state, "set_autostart_enabled", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn get_clipboard_history(state: tauri::State<AppState>) -> Vec<String> {
    state.clipboard_history.lock().unwrap().iter().cloned().collect()
}

#[tauri::command(async)]
fn list_connections(state: tauri::State<AppState>, pid: Option<u32>) -> Vec<network_control::ConnectionInfo> {
    let sys = state.sys.lock().unwrap();
    network_control::list_connections(&sys, pid)
}

#[tauri::command(async)]
fn list_network_interfaces() -> Vec<String> {
    network_control::list_interfaces()
}

#[tauri::command(async)]
fn limit_interface_bandwidth(state: tauri::State<AppState>, iface: String, rate_kbit: u32) -> control_process::ActionResult {
    let r = network_control::limit_interface(&iface, rate_kbit);
    log_action(&state, "limit_interface_bandwidth", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn clear_interface_bandwidth_limit(state: tauri::State<AppState>, iface: String) -> control_process::ActionResult {
    let r = network_control::clear_interface_limit(&iface);
    log_action(&state, "clear_interface_bandwidth_limit", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn block_process_network(state: tauri::State<AppState>, pid: u32) -> control_process::ActionResult {
    let r = network_control::block_process_network(pid);
    log_action(&state, "block_process_network", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn unblock_all_network(state: tauri::State<AppState>) -> control_process::ActionResult {
    let r = network_control::unblock_all_network();
    log_action(&state, "unblock_all_network", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn list_usb_devices() -> Vec<usb_control::UsbDevice> {
    usb_control::list_devices()
}

#[tauri::command(async)]
fn set_usb_authorized(state: tauri::State<AppState>, device_id: String, authorized: bool) -> control_process::ActionResult {
    let r = usb_control::set_authorized(&device_id, authorized);
    log_action(&state, "set_usb_authorized", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn list_windows() -> Result<Vec<window_control::WindowInfo>, String> {
    window_control::list_windows()
}

#[tauri::command(async)]
fn move_window_to_workspace(state: tauri::State<AppState>, address: String, workspace: i32) -> control_process::ActionResult {
    let r = window_control::move_window_to_workspace(&address, workspace);
    log_action(&state, "move_window_to_workspace", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn focus_workspace(state: tauri::State<AppState>, workspace: i32) -> control_process::ActionResult {
    let r = window_control::focus_workspace(workspace);
    log_action(&state, "focus_workspace", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn close_window(state: tauri::State<AppState>, address: String) -> control_process::ActionResult {
    let r = window_control::close_window(&address);
    log_action(&state, "close_window", r.ok, &r.message);
    control_process::ActionResult { ok: r.ok, message: r.message }
}

#[tauri::command(async)]
fn set_gpu_clock_lock(state: tauri::State<AppState>, min_mhz: u32, max_mhz: u32) -> Result<String, String> {
    let r = gpu_control::set_gpu_clock_lock(min_mhz, max_mhz);
    log_action(&state, "set_gpu_clock_lock", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command(async)]
fn reset_gpu_clock_lock(state: tauri::State<AppState>) -> Result<String, String> {
    let r = gpu_control::reset_gpu_clock_lock();
    log_action(&state, "reset_gpu_clock_lock", r.is_ok(), r.as_deref().unwrap_or_else(|e| e));
    r
}

#[tauri::command(async)]
fn list_namespaces(state: tauri::State<AppState>) -> Vec<namespace_control::NamespaceGroup> {
    let sys = state.sys.lock().unwrap();
    namespace_control::list_namespaces(&sys)
}

#[tauri::command]
async fn scan_folder_sizes(path: String) -> Result<Vec<storage_explorer::FolderEntry>, String> {
    storage_explorer::scan_folder(&path).await
}

#[tauri::command]
async fn get_memory_breakdown() -> Result<memory_map::MemoryBreakdown, String> {
    tauri::async_runtime::spawn_blocking(memory_map::breakdown)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn get_history(state: tauri::State<AppState>, since_secs_ago: i64) -> db::History {
    state.db.history(chrono_now() - since_secs_ago, since_secs_ago / 360)
}

#[tauri::command(async)]
fn get_log_report(state: tauri::State<AppState>, kind: String, offset: i32) -> Result<logbook::LogReport, String> {
    logbook::report(&state.db, &kind, offset)
}

#[tauri::command(async)]
fn export_log_report(state: tauri::State<AppState>, kind: String, offset: i32) -> Result<String, String> {
    logbook::export(&state.db, &kind, offset)
}

#[tauri::command(async)]
fn search_processes(state: tauri::State<AppState>, query: String) -> Vec<ProcessInfo> {
    let needle = query.to_lowercase();
    let sys = state.sys.lock().unwrap();
    let mut found: Vec<ProcessInfo> = sys
        .processes()
        .values()
        // sysinfo lists threads too; keep only real processes.
        .filter(|p| p.thread_kind().is_none())
        .filter(|p| {
            needle.is_empty()
                || p.name().to_string_lossy().to_lowercase().contains(&needle)
                || p.pid().to_string() == needle
        })
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_usage: p.cpu_usage(),
            memory_bytes: p.memory(),
        })
        .collect();
    found.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    found.truncate(60);
    found
}

#[tauri::command(async)]
fn open_path(path: String, with: String) -> control_process::ActionResult {
    desktop::open_path(&path, &with)
}

#[tauri::command(async)]
fn session_action(state: tauri::State<AppState>, action: String) -> control_process::ActionResult {
    logged(&state, &format!("session_{action}"), desktop::session_action(&action))
}

#[tauri::command(async)]
fn get_audio_devices() -> desktop::AudioDevices {
    desktop::audio_devices()
}

#[tauri::command(async)]
fn set_default_audio(state: tauri::State<AppState>, kind: String, name: String) -> control_process::ActionResult {
    logged(&state, "set_default_audio", desktop::set_default_audio(&kind, &name))
}

#[tauri::command(async)]
fn get_cleanup_status() -> desktop::CleanupStatus {
    desktop::cleanup_status()
}

#[tauri::command(async)]
fn clean_up(state: tauri::State<AppState>, target: String) -> control_process::ActionResult {
    logged(&state, &format!("clean_{target}"), desktop::clean(&target))
}

#[derive(Serialize)]
struct CategoryView {
    name: &'static str,
    blurb: &'static str,
}

#[derive(Serialize)]
struct TweaksView {
    categories: Vec<CategoryView>,
    tweaks: Vec<tweaks::TweakState>,
    presets: Vec<tweak_presets::PresetView>,
    recommendations: Vec<tweak_presets::Recommendation>,
    changes: Vec<tweaks::ChangeRow>,
    /// "none" | "installed" | "stale" | "missing"
    boot_service: &'static str,
}

#[tauri::command(async)]
fn get_tweaks(state: tauri::State<AppState>) -> TweaksView {
    let tweaks = tweaks::list(&state.db);
    TweaksView {
        categories: tweak_catalog::CATEGORIES
            .iter()
            .map(|(name, blurb)| CategoryView { name, blurb })
            .collect(),
        presets: tweak_presets::list(&tweaks),
        recommendations: tweak_presets::recommendations(&tweaks, &tweak_presets::Facts::gather()),
        changes: tweaks::recent_changes(&state.db, 40),
        boot_service: tweaks::boot_service_state(&state.db),
        tweaks,
    }
}

fn log_tweak_results(state: &AppState, source: &str, results: &[tweaks::ChangeResult]) {
    for r in results {
        log_action(
            state,
            &format!("tweak:{} ({source})", r.id),
            r.ok,
            &format!("{} → {}: {}", r.old_value, r.new_value, r.message),
        );
    }
}

#[tauri::command(async)]
fn apply_tweaks(
    state: tauri::State<AppState>,
    changes: Vec<tweaks::Change>,
    source: String,
    keep: Option<bool>,
) -> Vec<tweaks::ChangeResult> {
    let results = tweaks::apply(&state.db, &changes, &source, keep);
    log_tweak_results(&state, &source, &results);
    results
}

#[tauri::command(async)]
fn undo_tweak_batch(state: tauri::State<AppState>, batch: i64) -> Vec<tweaks::ChangeResult> {
    let changes = tweaks::undo_changes(&state.db, batch);
    let results = tweaks::apply(&state.db, &changes, "undo", None);
    log_tweak_results(&state, "undo", &results);
    results
}

#[tauri::command(async)]
fn set_tweak_kept(state: tauri::State<AppState>, id: String, keep: bool) -> control_process::ActionResult {
    let r = match tweaks::set_kept(&state.db, &id, keep) {
        Ok(message) => sys_util::ok(message),
        Err(e) => sys_util::err(e),
    };
    logged(&state, &format!("tweak_keep:{id}"), r)
}

#[tauri::command(async)]
fn save_tweak_preset(state: tauri::State<AppState>, name: String) -> control_process::ActionResult {
    match tweak_presets::save_current(&name, &tweaks::list(&state.db)) {
        Ok(count) => sys_util::ok(format!("Saved “{}” with {count} setting(s)", name.trim())),
        Err(e) => sys_util::err(e),
    }
}

#[tauri::command(async)]
fn delete_tweak_preset(name: String) -> control_process::ActionResult {
    match tweak_presets::delete_saved(&name) {
        Ok(()) => sys_util::ok("Preset deleted"),
        Err(e) => sys_util::err(e),
    }
}

#[tauri::command(async)]
fn list_vpns() -> Vec<vpn::VpnConnection> {
    vpn::list()
}

#[tauri::command(async)]
fn set_vpn_active(state: tauri::State<AppState>, uuid: String, up: bool) -> control_process::ActionResult {
    logged(&state, if up { "vpn_connect" } else { "vpn_disconnect" }, vpn::set_active(&uuid, up))
}

#[tauri::command(async)]
fn set_vpn_autoconnect(state: tauri::State<AppState>, uuid: String, enabled: bool) -> control_process::ActionResult {
    logged(&state, "vpn_autoconnect", vpn::set_autoconnect(&uuid, enabled))
}

#[tauri::command(async)]
fn rename_vpn(state: tauri::State<AppState>, uuid: String, name: String) -> control_process::ActionResult {
    logged(&state, "vpn_rename", vpn::rename(&uuid, &name))
}

#[tauri::command(async)]
fn delete_vpn(state: tauri::State<AppState>, uuid: String) -> control_process::ActionResult {
    logged(&state, "vpn_delete", vpn::delete(&uuid))
}

#[tauri::command(async)]
fn import_vpn_file(state: tauri::State<AppState>, path: String) -> control_process::ActionResult {
    let r = match vpn::import_file(&path, None) {
        Ok(_) => sys_util::ok("Added to your VPNs"),
        Err(e) => sys_util::err(e),
    };
    logged(&state, "vpn_import", r)
}

#[tauri::command(async)]
fn find_vpn_configs() -> Vec<String> {
    vpn::find_configs()
}

#[tauri::command(async)]
fn fetch_vpn_gate_servers() -> Result<Vec<vpn::GateServer>, String> {
    vpn::fetch_gate_servers()
}

#[tauri::command(async)]
fn add_vpn_gate_server(state: tauri::State<AppState>, host: String, connect: bool) -> control_process::ActionResult {
    logged(&state, "vpn_gate_add", vpn::add_gate_server(&host, connect))
}

#[tauri::command(async)]
fn connect_best_vpn_gate(state: tauri::State<AppState>, country_code: String) -> control_process::ActionResult {
    logged(&state, "vpn_gate_auto", vpn::connect_best_gate(&country_code))
}

#[tauri::command(async)]
fn get_public_address() -> Result<vpn::PublicAddress, String> {
    vpn::public_address()
}

#[tauri::command(async)]
fn get_vpn_kill_switch() -> vpn::KillSwitchStatus {
    vpn::kill_switch_status()
}

#[tauri::command(async)]
fn set_vpn_kill_switch(state: tauri::State<AppState>, enabled: bool, allow_lan: bool) -> control_process::ActionResult {
    logged(&state, "vpn_kill_switch", vpn::set_kill_switch(enabled, allow_lan))
}

#[tauri::command(async)]
fn get_vpn_split() -> vpn::SplitStatus {
    vpn::split_status()
}

#[tauri::command(async)]
fn start_vpn_split(state: tauri::State<AppState>, uuid: String) -> control_process::ActionResult {
    logged(&state, "vpn_split_start", vpn::split_start(&uuid))
}

#[tauri::command(async)]
fn stop_vpn_split(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "vpn_split_stop", vpn::split_stop())
}

#[tauri::command(async)]
fn launch_in_vpn(state: tauri::State<AppState>, command: String) -> control_process::ActionResult {
    logged(&state, "vpn_split_launch", vpn::launch_in_vpn(&command))
}

#[tauri::command(async)]
fn get_container_logs(id: String) -> Result<String, String> {
    docker_control::container_logs(&id)
}

#[tauri::command(async)]
fn get_container_stats(id: String) -> Result<docker_control::ContainerStats, String> {
    docker_control::container_stats(&id)
}

#[tauri::command(async)]
fn get_service_logs(name: String, user: bool) -> Result<Vec<String>, String> {
    systemd_control::unit_logs(&name, user)
}

#[tauri::command(async)]
fn import_profile_file(path: String) -> control_process::ActionResult {
    match profiles::import_file(&path) {
        Ok(name) => sys_util::ok(format!("Imported profile “{name}”")),
        Err(e) => sys_util::err(e),
    }
}

#[tauri::command(async)]
fn get_autostart() -> autostart::AutostartStatus {
    autostart::status()
}

#[tauri::command(async)]
fn set_autostart(state: tauri::State<AppState>, enabled: bool) -> control_process::ActionResult {
    logged(&state, "start_at_login", autostart::set(enabled))
}

#[tauri::command(async)]
fn global_search(state: tauri::State<AppState>, query: String) -> Vec<search::Hit> {
    // Snapshot the process list first so the lock isn't held during the search.
    let sys = state.sys.lock().unwrap();
    search::search(&state.db, &sys, &query)
}

#[tauri::command(async)]
fn get_notifications(state: tauri::State<AppState>) -> Vec<logbook::NotificationRow> {
    logbook::flush_notifications(&state.db);
    logbook::recent_notifications(&state.db, 30)
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    /// Whether a tray icon could be created on this desktop.
    tray: bool,
}

#[tauri::command(async)]
fn get_app_info(state: tauri::State<AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        tray: state.tray.lock().unwrap().is_some(),
    }
}

// ---- tray ----

struct TrayHandles {
    tray: tauri::tray::TrayIcon,
    stats: tauri::menu::MenuItem<tauri::Wry>,
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.show().ok();
        window.unminimize().ok();
        window.set_focus().ok();
    }
}

/// Builds the tray icon. Fails harmlessly on desktops without tray support,
/// in which case closing the window simply quits as before.
fn build_tray(app: &tauri::App) -> tauri::Result<TrayHandles> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let stats = MenuItemBuilder::with_id("stats", "Trace is starting…").enabled(false).build(app)?;
    let mut power = SubmenuBuilder::new(app, "Power profile");
    for profile in tuning::cpu_power().available_power_profiles {
        power = power.text(format!("power:{profile}"), profile.replace('-', " "));
    }
    let menu = MenuBuilder::new(app)
        .item(&stats)
        .separator()
        .text("open", "Open Trace")
        .item(&power.build()?)
        .text("awake_on", "Keep awake: turn on")
        .text("awake_off", "Keep awake: turn off")
        .separator()
        .text("quit", "Quit Trace")
        .build()?;
    let mut builder = tauri::tray::TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("Trace")
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            let state = app.state::<AppState>();
            let tweak = |tweak_id: &str, value: &str| {
                let change = tweaks::Change { id: tweak_id.to_string(), value: value.to_string() };
                let results = tweaks::apply(&state.db, &[change], "tray", None);
                log_tweak_results(&state, "tray", &results);
            };
            match id.as_str() {
                "open" => show_main_window(app),
                "quit" => app.exit(0),
                "awake_on" => tweak("power.keep_awake", "on"),
                "awake_off" => tweak("power.keep_awake", "off"),
                other => {
                    if let Some(profile) = other.strip_prefix("power:") {
                        tweak("power.profile", profile);
                    }
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    Ok(TrayHandles { tray: builder.build(app)?, stats })
}

fn update_tray(state: &AppState, snap: &Snapshot) {
    let guard = state.tray.lock().unwrap();
    let Some(handles) = guard.as_ref() else {
        return;
    };
    let ram = snap.used_memory_bytes as f32 / snap.total_memory_bytes.max(1) as f32 * 100.0;
    let hottest = snap.temperatures.iter().map(|t| t.celsius).fold(0.0, f32::max);
    let mut line = format!("CPU {:.0}%  ·  RAM {:.0}%", snap.cpu_usage_percent, ram);
    if hottest > 0.0 {
        line.push_str(&format!("  ·  {hottest:.0}°C"));
    }
    handles.stats.set_text(&line).ok();
    handles
        .tray
        .set_title(Some(format!("{:.0}% {:.0}%", snap.cpu_usage_percent, ram)))
        .ok();
}

/// Runs an action, records it in the audit log, and hands the result back.
fn logged(state: &AppState, name: &str, r: control_process::ActionResult) -> control_process::ActionResult {
    log_action(state, name, r.ok, &r.message);
    r
}

// Every command is `command(async)` (or an `async fn`): plain commands run on
// the UI thread, where anything that shells out, walks /proc or waits on a
// password prompt would freeze the window.

#[tauri::command(async)]
fn get_app_memory_history(state: tauri::State<AppState>, since_secs_ago: i64) -> Vec<db::AppMemoryPoint> {
    state
        .db
        .app_memory_history(chrono_now() - since_secs_ago, since_secs_ago / 120)
}

#[tauri::command(async)]
fn app_action(state: tauri::State<AppState>, name: String, action: String) -> control_process::ActionResult {
    let r = memory_map::app_action(&name, &action);
    logged(&state, &format!("app_{action}:{name}"), r)
}

#[tauri::command(async)]
fn get_app_limits(name: String) -> memory_map::AppLimits {
    memory_map::app_limits(&name)
}

#[tauri::command(async)]
fn set_app_limits(
    state: tauri::State<AppState>,
    name: String,
    cpu_percent: Option<u32>,
    memory_mb: Option<u64>,
) -> control_process::ActionResult {
    let r = memory_map::set_app_limits(&name, cpu_percent, memory_mb);
    logged(&state, &format!("limit_app:{name}"), r)
}

#[tauri::command(async)]
fn get_pressure_history(state: tauri::State<AppState>, since_secs_ago: i64) -> Vec<db::PressurePoint> {
    state.db.pressure_history(chrono_now() - since_secs_ago)
}

#[tauri::command(async)]
fn get_activity(state: tauri::State<AppState>) -> activity::Activity {
    activity::sample(&mut state.activity.lock().unwrap())
}

#[derive(Serialize)]
struct HealthOverview {
    report: health::HealthReport,
    reboot: health::RebootStatus,
    failed_units: Vec<health::FailedUnit>,
    batteries: Vec<health::BatteryHealth>,
    boots: Vec<db::BootPoint>,
    drives: Vec<health::Drive>,
}

fn health_inputs(snap: &Snapshot, slow: &SlowHealth, battery_wear_percent: f32) -> health::HealthInputs {
    health::HealthInputs {
        mem_available_percent: snap.available_memory_bytes as f32 / snap.total_memory_bytes.max(1) as f32 * 100.0,
        swap_used_percent: if snap.total_swap_bytes > 0 {
            snap.used_swap_bytes as f32 / snap.total_swap_bytes as f32 * 100.0
        } else {
            0.0
        },
        disks: snap
            .disks
            .iter()
            .filter(|d| d.total_bytes > 0)
            .map(|d| {
                let used = (d.total_bytes - d.available_bytes) as f32 / d.total_bytes as f32 * 100.0;
                (d.mount_point.clone(), used)
            })
            .collect(),
        max_temp_c: snap.temperatures.iter().map(|t| t.celsius).fold(0.0, f32::max),
        pressure: snap.pressure.clone(),
        failed_units: slow.failed_units,
        reboot_needed: slow.reboot_needed,
        battery_wear_percent,
    }
}

#[tauri::command(async)]
fn get_health(state: tauri::State<AppState>) -> HealthOverview {
    let snap = take_snapshot(&state);
    let reboot = health::reboot_status();
    let failed_units = health::failed_units();
    let batteries = health::battery_health();
    let slow = SlowHealth {
        failed_units: failed_units.len(),
        reboot_needed: reboot.reboot_needed,
    };
    *state.slow_health.lock().unwrap() = slow.clone();
    let wear = batteries.iter().map(|b| b.wear_percent).fold(0.0, f32::max);
    HealthOverview {
        report: health::health_report(&health_inputs(&snap, &slow, wear)),
        reboot,
        failed_units,
        batteries,
        boots: state.db.boot_history(),
        drives: health::list_drives(),
    }
}

#[tauri::command(async)]
fn get_oom_kills() -> Vec<health::LogLine> {
    health::oom_kills()
}

#[tauri::command(async)]
fn get_recent_errors() -> Vec<health::LogLine> {
    health::recent_errors()
}

#[tauri::command(async)]
fn check_drive_health() -> Result<Vec<health::DriveHealth>, String> {
    health::drive_health()
}

#[tauri::command(async)]
fn get_memory_tuning() -> tuning::MemoryTuning {
    tuning::memory_tuning()
}

#[tauri::command(async)]
fn set_swappiness(state: tauri::State<AppState>, value: u32) -> control_process::ActionResult {
    logged(&state, "set_swappiness", tuning::set_swappiness(value))
}

#[tauri::command(async)]
fn drop_caches(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "drop_caches", tuning::drop_caches())
}

#[tauri::command(async)]
fn compact_memory(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "compact_memory", tuning::compact_memory())
}

#[tauri::command(async)]
fn flush_swap(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "flush_swap", tuning::flush_swap())
}

#[tauri::command(async)]
fn get_cpu_power() -> tuning::CpuPower {
    tuning::cpu_power()
}

#[tauri::command(async)]
fn set_power_profile(state: tauri::State<AppState>, profile: String) -> control_process::ActionResult {
    logged(&state, "set_power_profile", tuning::set_power_profile(&profile))
}

#[tauri::command(async)]
fn set_cpu_governor(state: tauri::State<AppState>, governor: String) -> control_process::ActionResult {
    logged(&state, "set_cpu_governor", tuning::set_governor(&governor))
}

#[tauri::command(async)]
fn set_energy_preference(state: tauri::State<AppState>, preference: String) -> control_process::ActionResult {
    logged(&state, "set_energy_preference", tuning::set_energy_preference(&preference))
}

#[tauri::command(async)]
fn set_platform_profile(state: tauri::State<AppState>, profile: String) -> control_process::ActionResult {
    logged(&state, "set_platform_profile", tuning::set_platform_profile(&profile))
}

#[tauri::command(async)]
fn set_core_online(state: tauri::State<AppState>, index: u32, online: bool) -> control_process::ActionResult {
    logged(&state, "set_core_online", tuning::set_core_online(index, online))
}

#[tauri::command(async)]
fn list_fans() -> Vec<tuning::Fan> {
    tuning::list_fans()
}

#[tauri::command(async)]
fn set_fan_mode(state: tauri::State<AppState>, pwm_path: String, mode: String, percent: u32) -> control_process::ActionResult {
    logged(&state, "set_fan_mode", tuning::set_fan_mode(&pwm_path, &mode, percent))
}

#[tauri::command(async)]
fn get_automation() -> automation::Config {
    automation::load()
}

#[tauri::command(async)]
fn save_automation(config: automation::Config) -> Result<(), String> {
    automation::save(&config)
}

#[tauri::command(async)]
fn get_package_status() -> packages::PackageStatus {
    packages::status()
}

#[tauri::command(async)]
fn clean_package_cache(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "clean_package_cache", packages::clean_cache())
}

#[tauri::command(async)]
fn remove_orphan_packages(state: tauri::State<AppState>) -> control_process::ActionResult {
    logged(&state, "remove_orphan_packages", packages::remove_orphans())
}

#[tauri::command(async)]
fn get_firewall_status() -> connectivity::FirewallStatus {
    connectivity::firewall_status()
}

#[tauri::command(async)]
fn set_firewall_port(
    state: tauri::State<AppState>,
    port: u16,
    protocol: String,
    open: bool,
    permanent: bool,
) -> control_process::ActionResult {
    logged(
        &state,
        "set_firewall_port",
        connectivity::set_firewall_port(port, &protocol, open, permanent),
    )
}

#[tauri::command(async)]
fn list_radios() -> Vec<connectivity::Radio> {
    connectivity::list_radios()
}

#[tauri::command(async)]
fn set_radio(state: tauri::State<AppState>, kind: String, enabled: bool) -> control_process::ActionResult {
    logged(&state, "set_radio", connectivity::set_radio(&kind, enabled))
}

#[tauri::command(async)]
fn list_wifi(rescan: bool) -> Vec<connectivity::WifiNetwork> {
    connectivity::list_wifi(rescan)
}

#[tauri::command(async)]
fn connect_wifi(state: tauri::State<AppState>, ssid: String, password: Option<String>) -> control_process::ActionResult {
    logged(&state, "connect_wifi", connectivity::connect_wifi(&ssid, password.as_deref()))
}

#[tauri::command(async)]
fn list_bluetooth() -> Vec<connectivity::BluetoothDevice> {
    connectivity::list_bluetooth()
}

#[tauri::command(async)]
fn set_bluetooth_connected(state: tauri::State<AppState>, address: String, connect: bool) -> control_process::ActionResult {
    logged(
        &state,
        "set_bluetooth_connected",
        connectivity::set_bluetooth_connected(&address, connect),
    )
}

#[derive(Serialize)]
struct DisplayStatus {
    monitors: Vec<connectivity::Monitor>,
    night_light_kelvin: Option<u32>,
}

#[tauri::command(async)]
fn get_display_status() -> DisplayStatus {
    DisplayStatus {
        monitors: connectivity::list_monitors(),
        night_light_kelvin: connectivity::night_light(),
    }
}

#[tauri::command(async)]
fn set_monitor_mode(state: tauri::State<AppState>, name: String, mode: String, scale: f64) -> control_process::ActionResult {
    logged(&state, "set_monitor_mode", connectivity::set_monitor_mode(&name, &mode, scale))
}

#[tauri::command(async)]
fn set_night_light(state: tauri::State<AppState>, kelvin: Option<u32>) -> control_process::ActionResult {
    logged(&state, "set_night_light", connectivity::set_night_light(kelvin))
}

const TICK_SECS: u64 = 10;

/// One pass of everything the background loop does besides sampling:
/// per-app memory history, rules, scheduled profiles and alerts.
fn background_tick(state: &AppState, snap: &Snapshot, tick: u64) {
    let now = chrono_now();
    let config = automation::load();

    if tick % 30 == 0 {
        *state.slow_health.lock().unwrap() = SlowHealth {
            failed_units: health::failed_units().len(),
            reboot_needed: health::reboot_status().reboot_needed,
        };
        logbook::ingest_crashes(&state.db, tick == 0);
        logbook::ingest_power_events(&state.db, tick == 0);
        logbook::ingest_packages(&state.db);
    }
    logbook::record_battery(&state.db, now, TICK_SECS as i64, &mut state.battery_prev.lock().unwrap());
    logbook::record_network(&state.db, now, &mut state.net_totals.lock().unwrap());
    logbook::flush_notifications(&state.db);

    if let Some(app) = logbook::focused_app() {
        logbook::record_focus(&state.db, now, TICK_SECS as i64, &app);
    }

    if tick % 6 == 0 {
        let apps = memory_map::breakdown().apps;
        let top: Vec<(String, u64, u64)> = apps
            .iter()
            .take(20)
            .map(|a| (a.name.clone(), a.ram_bytes, a.swap_bytes))
            .collect();
        state.db.insert_app_memory(now, &top);
        // Anything under ~20 MB is a helper, not something you'd call an app.
        let running: Vec<(String, u64)> = apps
            .iter()
            .filter(|a| a.ram_bytes + a.swap_bytes > 20 * 1024 * 1024)
            .map(|a| (a.name.clone(), a.ram_bytes))
            .collect();
        logbook::record_running(&state.db, now, 6 * TICK_SECS as i64, &running);
        let outcomes = automation::evaluate_rules(&config, &apps, &mut state.automation.lock().unwrap(), now);
        for o in outcomes {
            log_action(state, &o.description, o.ok, &o.message);
        }
    }

    let due = automation::due_schedules(&config, &mut state.automation.lock().unwrap());
    if !due.is_empty() {
        let profiles = profiles::list_profiles();
        for name in due {
            match profiles.iter().find(|p| p.name == name) {
                Some(profile) => {
                    let outcomes = tauri::async_runtime::block_on(run_profile(state, profile, false));
                    let failed = outcomes.iter().filter(|o| !o.ok).count();
                    sys_util::notify(
                        &format!("Trace ran profile “{name}”"),
                        &format!("{} action(s), {failed} failed", outcomes.len()),
                        false,
                    );
                }
                None => log_action(state, &format!("schedule:{name}"), false, "Profile no longer exists"),
            }
        }
    }

    if config.alerts_enabled {
        let slow = state.slow_health.lock().unwrap().clone();
        let report = health::health_report(&health_inputs(snap, &slow, 0.0));
        let mut automation = state.automation.lock().unwrap();
        for issue in report.issues.iter().filter(|i| i.severity == "critical") {
            automation.alert(&issue.title, &issue.detail, now);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let start_hidden = std::env::args().any(|a| a == "--hidden");
    tauri::Builder::default()
        // Must be registered first: a second launch hands over to the
        // running copy (and brings its window back) instead of starting again.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == "--hidden") {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let has_tray = state.tray.lock().unwrap().is_some();
                // Keep recording in the background; the tray icon (or
                // launching Trace again) brings the window back.
                if has_tray && automation::load().close_to_tray {
                    api.prevent_close();
                    window.hide().ok();
                }
            }
        })
        .manage(AppState {
            sys: Mutex::new(System::new_all()),
            last_net: Mutex::new(std::collections::HashMap::new()),
            db: db::Db::open(),
            last_snapshot: Mutex::new(None),
            clipboard_history: Mutex::new(std::collections::VecDeque::new()),
            activity: Mutex::new(activity::ActivityState::default()),
            automation: Mutex::new(automation::State::default()),
            slow_health: Mutex::new(SlowHealth::default()),
            battery_prev: Mutex::new(logbook::BatteryPrev::default()),
            tray: Mutex::new(None),
            net_totals: Mutex::new(std::collections::HashMap::new()),
        })
        .setup(move |app| {
            profiles::ensure_default_profiles();
            match build_tray(app) {
                Ok(handles) => *app.state::<AppState>().tray.lock().unwrap() = Some(handles),
                Err(e) => eprintln!("trace: no tray icon on this desktop ({e})"),
            }
            if !start_hidden {
                show_main_window(app.handle());
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                {
                    let state = handle.state::<AppState>();
                    logbook::init(&state.db);
                    tweaks::init(&state.db);
                    tweaks::reapply_session(&state.db);
                    state.db.prune_app_memory(chrono_now() - 14 * 86_400);
                    logbook::prune_raw(&state.db, chrono_now() - 90 * 86_400);
                    if let Some(boot) = health::boot_time() {
                        let booted_at = chrono_now() - System::uptime() as i64;
                        state.db.record_boot(&boot.boot_id, booted_at, boot.total_ms, boot.userspace_ms);
                    }
                }
                let mut tick: u64 = 0;
                loop {
                    {
                        let state = handle.state::<AppState>();
                        let snap = take_snapshot(&state);
                        record_snapshot(&state, &snap);
                        poll_clipboard(&state);
                        background_tick(&state, &snap, tick);
                        update_tray(&state, &snap);
                    }
                    tick += 1;
                    std::thread::sleep(std::time::Duration::from_secs(TICK_SECS));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_resource_history,
            get_disk_history,
            diagnose_slowness,
            who_is_using_port,
            scan_projects,
            freeze_process,
            resume_process,
            kill_process,
            set_process_priority,
            set_process_affinity,
            set_process_cpu_limit,
            set_process_memory_limit,
            set_process_oom_score,
            list_services,
            start_service,
            stop_service,
            restart_service,
            list_containers,
            start_container,
            stop_container,
            restart_container,
            set_gpu_persistence,
            set_gpu_power_limit,
            list_profiles,
            save_profile,
            apply_profile,
            undo_profile,
            get_action_log,
            check_for_update,
            kill_process_on_port,
            set_ionice,
            get_brightness,
            set_brightness,
            get_volume,
            set_volume,
            toggle_mute,
            startup_impact,
            list_autostart,
            set_autostart_enabled,
            get_clipboard_history,
            list_connections,
            list_network_interfaces,
            limit_interface_bandwidth,
            clear_interface_bandwidth_limit,
            block_process_network,
            unblock_all_network,
            list_usb_devices,
            set_usb_authorized,
            list_windows,
            move_window_to_workspace,
            focus_workspace,
            close_window,
            set_gpu_clock_lock,
            reset_gpu_clock_lock,
            list_namespaces,
            scan_folder_sizes,
            get_memory_breakdown,
            connect_best_vpn_gate,
            get_public_address,
            get_vpn_kill_switch,
            set_vpn_kill_switch,
            get_vpn_split,
            start_vpn_split,
            stop_vpn_split,
            launch_in_vpn,
            get_container_logs,
            get_container_stats,
            get_service_logs,
            import_profile_file,
            get_autostart,
            set_autostart,
            global_search,
            get_notifications,
            get_app_info,
            get_tweaks,
            apply_tweaks,
            undo_tweak_batch,
            set_tweak_kept,
            save_tweak_preset,
            delete_tweak_preset,
            list_vpns,
            set_vpn_active,
            set_vpn_autoconnect,
            rename_vpn,
            delete_vpn,
            import_vpn_file,
            find_vpn_configs,
            fetch_vpn_gate_servers,
            add_vpn_gate_server,
            get_history,
            get_log_report,
            export_log_report,
            search_processes,
            open_path,
            session_action,
            get_audio_devices,
            set_default_audio,
            get_cleanup_status,
            clean_up,
            get_app_memory_history,
            app_action,
            get_app_limits,
            set_app_limits,
            get_pressure_history,
            get_activity,
            get_health,
            get_oom_kills,
            get_recent_errors,
            check_drive_health,
            get_memory_tuning,
            set_swappiness,
            drop_caches,
            compact_memory,
            flush_swap,
            get_cpu_power,
            set_power_profile,
            set_cpu_governor,
            set_energy_preference,
            set_platform_profile,
            set_core_online,
            list_fans,
            set_fan_mode,
            get_automation,
            save_automation,
            get_package_status,
            clean_package_cache,
            remove_orphan_packages,
            get_firewall_status,
            set_firewall_port,
            list_radios,
            set_radio,
            list_wifi,
            connect_wifi,
            list_bluetooth,
            set_bluetooth_connected,
            get_display_status,
            set_monitor_mode,
            set_night_light
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                tweaks::release_keep_awake();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises every read-only collector against the real machine. Set
    /// `TRACE_DUMP_DIR` to also write each result as JSON (handy as UI fixtures).
    #[test]
    fn live_readers_do_not_panic() {
        let state = AppState {
            sys: Mutex::new(System::new_all()),
            last_net: Mutex::new(std::collections::HashMap::new()),
            db: db::Db::open(),
            last_snapshot: Mutex::new(None),
            clipboard_history: Mutex::new(std::collections::VecDeque::new()),
            activity: Mutex::new(activity::ActivityState::default()),
            automation: Mutex::new(automation::State::default()),
            slow_health: Mutex::new(SlowHealth::default()),
            battery_prev: Mutex::new(logbook::BatteryPrev::default()),
            tray: Mutex::new(None),
            net_totals: Mutex::new(std::collections::HashMap::new()),
        };
        let snap = take_snapshot(&state);
        assert!(snap.total_memory_bytes > 0);

        activity::sample(&mut state.activity.lock().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let act = activity::sample(&mut state.activity.lock().unwrap());
        assert!(act.ready);

        let slow = SlowHealth {
            failed_units: health::failed_units().len(),
            reboot_needed: health::reboot_status().reboot_needed,
        };
        let report = health::health_report(&health_inputs(&snap, &slow, 0.0));
        assert!(report.score <= 100);

        logbook::init(&state.db);
        tweaks::init(&state.db);
        logbook::ingest_crashes(&state.db, false);
        logbook::ingest_power_events(&state.db, false);
        logbook::ingest_packages(&state.db);
        logbook::record_battery(&state.db, chrono_now(), 10, &mut state.battery_prev.lock().unwrap());
        logbook::record_network(&state.db, chrono_now(), &mut state.net_totals.lock().unwrap());
        for kind in ["day", "week", "month", "year"] {
            let report = logbook::report(&state.db, kind, 0).unwrap();
            assert!(report.end_ts > report.start_ts);
        }

        let Ok(dir) = std::env::var("TRACE_DUMP_DIR") else {
            return;
        };
        let dump = |name: &str, value: serde_json::Value| {
            fs::write(format!("{dir}/{name}.json"), value.to_string()).unwrap();
        };
        let json = |v: &dyn erased::Dump| v.dump();
        dump("get_snapshot", json(&snap));
        dump("get_activity", json(&act));
        dump("get_memory_breakdown", json(&memory_map::breakdown()));
        dump(
            "get_health",
            serde_json::json!({
                "report": report,
                "reboot": health::reboot_status(),
                "failed_units": health::failed_units(),
                "batteries": health::battery_health(),
                "boots": health::boot_time().map(|b| vec![serde_json::json!({
                    "ts": chrono_now(), "total_ms": b.total_ms, "userspace_ms": b.userspace_ms
                })]).unwrap_or_default(),
                "drives": health::list_drives(),
            }),
        );
        dump("get_recent_errors", json(&health::recent_errors()));
        dump("get_memory_tuning", json(&tuning::memory_tuning()));
        dump("get_cpu_power", json(&tuning::cpu_power()));
        dump("list_fans", json(&tuning::list_fans()));
        dump("get_firewall_status", json(&connectivity::firewall_status()));
        dump("list_radios", json(&connectivity::list_radios()));
        dump("list_wifi", json(&connectivity::list_wifi(false)));
        dump("list_bluetooth", json(&connectivity::list_bluetooth()));
        dump(
            "get_display_status",
            serde_json::json!({
                "monitors": connectivity::list_monitors(),
                "night_light_kelvin": connectivity::night_light(),
            }),
        );
        dump("get_automation", json(&automation::Config::default()));
        dump("list_profiles", json(&profiles::list_profiles()));
        dump("list_vpns", json(&vpn::list()));
        let all = tweaks::list(&state.db);
        dump(
            "get_tweaks",
            serde_json::json!({
                "categories": tweak_catalog::CATEGORIES.iter().map(|(name, blurb)| serde_json::json!({"name": name, "blurb": blurb})).collect::<Vec<_>>(),
                "presets": tweak_presets::list(&all),
                "recommendations": tweak_presets::recommendations(&all, &tweak_presets::Facts::gather()),
                "changes": tweaks::recent_changes(&state.db, 40),
                "boot_service": tweaks::boot_service_state(&state.db),
                "tweaks": all,
            }),
        );
        dump("find_vpn_configs", json(&vpn::find_configs()));
        if let Ok(servers) = vpn::fetch_gate_servers() {
            dump("fetch_vpn_gate_servers", json(&servers));
        }
        dump("get_log_report", json(&logbook::report(&state.db, "week", 0).unwrap()));
        dump("get_history", json(&state.db.history(chrono_now() - 6 * 3600, 60)));
        dump("get_audio_devices", json(&desktop::audio_devices()));
        dump("get_cleanup_status", json(&desktop::cleanup_status()));
        dump("search_processes", json(&snap.top_processes));
        dump("startup_impact", json(&system_control::startup_impact()));
        dump("list_autostart", json(&system_control::list_autostart()));
        dump("list_usb_devices", json(&usb_control::list_devices()));
        dump("list_windows", json(&window_control::list_windows().unwrap_or_default()));
        dump("list_namespaces", json(&namespace_control::list_namespaces(&state.sys.lock().unwrap())));
        dump(
            "list_connections",
            json(&network_control::list_connections(&state.sys.lock().unwrap(), None)),
        );
        dump("list_network_interfaces", json(&network_control::list_interfaces()));
        let services = tauri::async_runtime::block_on(systemd_control::list_units(true));
        dump("list_services", json(&services.unwrap_or_default()));
        let containers = tauri::async_runtime::block_on(docker_control::list_containers());
        dump("list_containers", json(&containers.unwrap_or_default()));
    }

    mod erased {
        pub trait Dump {
            fn dump(&self) -> serde_json::Value;
        }
        impl<T: serde::Serialize> Dump for T {
            fn dump(&self) -> serde_json::Value {
                serde_json::to_value(self).unwrap()
            }
        }
    }
}
