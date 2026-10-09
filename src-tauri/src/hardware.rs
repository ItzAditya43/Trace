//! Hardware: every sensor the kernel exposes, graphics cards and what is
//! using them, peripherals' batteries, and display hardware (backlight,
//! external-monitor brightness, arrangement, saved layouts, night light).

use crate::control_process::ActionResult;
use crate::memory_map::app_name;
use crate::sys_util::{data_dir, err, ok, read_num, read_trimmed, run, run_lossy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::time::Instant;

// ---- sensors ----

#[derive(Serialize, Clone)]
pub struct Sensor {
    pub chip: String,
    pub label: String,
    /// "temp" | "fan" | "voltage" | "power" | "current" | "frequency"
    pub kind: String,
    pub value: f64,
    pub unit: String,
}

/// Friendlier names for the chips most machines have.
fn chip_title(name: &str) -> String {
    match name {
        "k10temp" | "coretemp" | "zenpower" => "Processor".to_string(),
        "amdgpu" => "AMD graphics".to_string(),
        "nouveau" | "nvidia" => "NVIDIA graphics".to_string(),
        "nvme" => "SSD (NVMe)".to_string(),
        "acpitz" => "Mainboard".to_string(),
        n if n.starts_with("iwlwifi") || n.starts_with("ath") || n.starts_with("mt7") => "Wi-Fi card".to_string(),
        n if n.starts_with("BAT") => "Battery".to_string(),
        "asus" | "thinkpad" | "dell_smm" | "hp" => "Fans".to_string(),
        other => other.to_string(),
    }
}

/// Reads every `*_input` value under /sys/class/hwmon, scaled to natural units.
pub fn sensors() -> Vec<Sensor> {
    let mut out = Vec::new();
    let Ok(chips) = fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for chip in chips.flatten() {
        let dir = chip.path().to_string_lossy().to_string();
        let name = read_trimmed(&format!("{dir}/name")).unwrap_or_default();
        let Ok(files) = fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<Sensor> = files
            .flatten()
            .filter_map(|f| {
                let file = f.file_name().to_string_lossy().to_string();
                let stem = file.strip_suffix("_input")?;
                let prefix: String = stem.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                // (kind, divisor, unit) — hwmon reports milli-units, power in micro.
                let (kind, divisor, unit) = match prefix.as_str() {
                    "temp" => ("temp", 1000.0, "°C"),
                    "fan" => ("fan", 1.0, "RPM"),
                    "in" => ("voltage", 1000.0, "V"),
                    "power" => ("power", 1_000_000.0, "W"),
                    "curr" => ("current", 1000.0, "A"),
                    "freq" => ("frequency", 1_000_000.0, "MHz"),
                    _ => return None,
                };
                let raw: f64 = read_num(&format!("{dir}/{file}"))?;
                let label = read_trimmed(&format!("{dir}/{stem}_label"))
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| stem.to_string());
                Some(Sensor {
                    chip: chip_title(&name),
                    label,
                    kind: kind.to_string(),
                    value: raw / divisor,
                    unit: unit.to_string(),
                })
            })
            .collect();
        found.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.label.cmp(&b.label)));
        out.extend(found);
    }
    out.sort_by(|a, b| a.chip.cmp(&b.chip));
    out
}

// ---- graphics cards and who is using them ----

#[derive(Serialize, Clone)]
pub struct GpuApp {
    pub app: String,
    pub processes: usize,
    pub memory_bytes: u64,
    /// Share of the last interval this app kept the card's engines busy.
    pub busy_percent: f32,
}

#[derive(Serialize, Clone)]
pub struct GpuDevice {
    pub address: String,
    pub vendor: String,
    pub driver: String,
    /// The card driving the built-in screen.
    pub primary: bool,
    /// "active" or "suspended" (powered down while unused).
    pub power_state: String,
    pub apps: Vec<GpuApp>,
}

/// Engine time per (card, client) at the previous sample, to turn the
/// kernel's running totals into "busy right now".
#[derive(Default)]
pub struct GpuState {
    at: Option<Instant>,
    engine_ns: HashMap<(String, String), u64>,
}

/// "2 MiB" / "12 KiB" / "5" -> bytes
fn parse_size(text: &str) -> u64 {
    let mut parts = text.split_whitespace();
    let value: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match parts.next() {
        Some("KiB") => value * 1024,
        Some("MiB") => value * 1024 * 1024,
        Some("GiB") => value * 1024 * 1024 * 1024,
        _ => value,
    }
}

struct Client {
    pdev: String,
    id: String,
    memory: u64,
    engine_ns: u64,
}

/// One DRM client as described by a /proc/<pid>/fdinfo entry.
fn parse_fdinfo(text: &str) -> Option<Client> {
    let mut client = Client { pdev: String::new(), id: String::new(), memory: 0, engine_ns: 0 };
    let mut is_drm = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "drm-driver" => is_drm = true,
            "drm-pdev" => client.pdev = value.to_string(),
            "drm-client-id" => client.id = value.to_string(),
            k if k.starts_with("drm-resident-") && k != "drm-resident-cpu" => client.memory += parse_size(value),
            k if k.starts_with("drm-engine-") && !k.starts_with("drm-engine-capacity") => {
                client.engine_ns += value.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
            }
            _ => {}
        }
    }
    (is_drm && !client.pdev.is_empty()).then_some(client)
}

pub fn gpus(state: &mut GpuState) -> Vec<GpuDevice> {
    let now = Instant::now();
    let dt = state.at.map(|t| now.duration_since(t).as_secs_f64().max(0.2));
    // (card, app) -> (pids, memory, busy ns this interval)
    let mut usage: HashMap<(String, String), (std::collections::HashSet<u32>, u64, u64)> = HashMap::new();
    let mut seen_clients = std::collections::HashSet::new();
    let mut engine_now = HashMap::new();

    if let Ok(procs) = fs::read_dir("/proc") {
        for entry in procs.flatten() {
            let pid_str = entry.file_name().to_string_lossy().to_string();
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };
            let Ok(fds) = fs::read_dir(entry.path().join("fdinfo")) else {
                continue;
            };
            let mut name: Option<String> = None;
            for fd in fds.flatten() {
                let Ok(text) = fs::read_to_string(fd.path()) else {
                    continue;
                };
                if !text.contains("drm-driver") {
                    continue;
                }
                let Some(client) = parse_fdinfo(&text) else {
                    continue;
                };
                let key = (client.pdev.clone(), client.id.clone());
                // The same client shows up once per duplicated descriptor.
                if !seen_clients.insert(key.clone()) {
                    continue;
                }
                let delta = state
                    .engine_ns
                    .get(&key)
                    .map(|before| client.engine_ns.saturating_sub(*before))
                    .unwrap_or(0);
                engine_now.insert(key, client.engine_ns);
                let app = name.get_or_insert_with(|| app_name(&pid_str).unwrap_or_else(|| pid_str.clone())).clone();
                let slot = usage.entry((client.pdev, app)).or_default();
                slot.0.insert(pid);
                slot.1 += client.memory;
                slot.2 += delta;
            }
        }
    }
    state.at = Some(now);
    state.engine_ns = engine_now;

    let mut devices = Vec::new();
    let Ok(pci) = fs::read_dir("/sys/bus/pci/devices") else {
        return devices;
    };
    for dev in pci.flatten() {
        let dir = dev.path().to_string_lossy().to_string();
        // PCI class 0x03xxxx is a display controller.
        if !read_trimmed(&format!("{dir}/class")).is_some_and(|c| c.starts_with("0x03")) {
            continue;
        }
        let address = dev.file_name().to_string_lossy().to_string();
        let mut apps: Vec<GpuApp> = usage
            .iter()
            .filter(|((pdev, _), _)| *pdev == address)
            .map(|((_, app), (pids, memory, busy_ns))| GpuApp {
                app: app.clone(),
                processes: pids.len(),
                memory_bytes: *memory,
                busy_percent: dt.map(|dt| (*busy_ns as f64 / 1e9 / dt * 100.0).min(100.0) as f32).unwrap_or(0.0),
            })
            .collect();
        apps.sort_by(|a, b| b.busy_percent.total_cmp(&a.busy_percent).then(b.memory_bytes.cmp(&a.memory_bytes)));
        devices.push(GpuDevice {
            vendor: match read_trimmed(&format!("{dir}/vendor")).as_deref() {
                Some("0x10de") => "NVIDIA",
                Some("0x1002") => "AMD",
                Some("0x8086") => "Intel",
                _ => "Other",
            }
            .to_string(),
            driver: fs::read_link(format!("{dir}/driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "none".to_string()),
            primary: read_num::<u8>(&format!("{dir}/boot_vga")) == Some(1),
            power_state: read_trimmed(&format!("{dir}/power/runtime_status")).unwrap_or_default(),
            address,
            apps,
        });
    }
    devices.sort_by(|a, b| b.primary.cmp(&a.primary));
    devices
}

// ---- peripherals ----

#[derive(Serialize)]
pub struct Peripheral {
    pub name: String,
    /// "mouse", "keyboard", "headset", …
    pub kind: String,
    pub percent: u32,
    pub state: String,
}

/// Battery levels of wireless mice, keyboards, headsets and the like.
pub fn peripherals() -> Vec<Peripheral> {
    let dump = run_lossy(Command::new("upower").arg("-d"));
    dump.split("\n\n")
        .filter_map(|block| {
            let path = block.lines().find_map(|l| l.strip_prefix("Device: "))?;
            // The laptop's own battery and its summary entry are shown elsewhere.
            if path.contains("battery_BAT") || path.ends_with("DisplayDevice") || path.contains("line_power") {
                return None;
            }
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(':'))
                    .map(|v| v.trim().to_string())
            };
            let percent = field("percentage")?.trim_end_matches('%').parse::<f32>().ok()? as u32;
            // The device type is the one line that is a bare word.
            let kind = block
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.contains(':') && !l.contains(' '))
                .unwrap_or("device")
                .to_string();
            Some(Peripheral {
                name: field("model").filter(|m| !m.is_empty()).unwrap_or_else(|| kind.clone()),
                kind,
                percent,
                state: field("state").unwrap_or_default(),
            })
        })
        .collect()
}

// ---- screen brightness ----

#[derive(Serialize)]
pub struct ExternalDisplay {
    /// ddcutil's display number.
    pub number: u32,
    pub connector: String,
    pub model: String,
    pub brightness: Option<u32>,
}

/// External monitors that accept brightness commands over the cable (DDC/CI).
/// Talking to a monitor takes a second or two, so this is on demand.
pub fn external_displays() -> Vec<ExternalDisplay> {
    let out = run_lossy(Command::new("timeout").args(["15", "ddcutil", "detect", "--brief"]));
    out.split("\n\n")
        .filter_map(|block| {
            let number: u32 = block.lines().find_map(|l| l.strip_prefix("Display "))?.trim().parse().ok()?;
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|l| l.trim().strip_prefix(key))
                    .map(|v| v.trim().to_string())
                    .unwrap_or_default()
            };
            let connector = field("DRM connector:");
            // "VCP 10 C 100 100": feature 0x10 is brightness — current, maximum.
            let reply = run_lossy(Command::new("timeout").args(["8", "ddcutil", "getvcp", "10", "--brief", "-d", &number.to_string()]));
            let values: Vec<u32> = reply.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some(ExternalDisplay {
                number,
                connector: connector.split_once('-').map(|(_, c)| c.to_string()).unwrap_or(connector),
                model: field("Monitor:"),
                brightness: match values.as_slice() {
                    [_, current, max] if *max > 0 => Some(current * 100 / max),
                    _ => None,
                },
            })
        })
        .collect()
}

pub fn set_external_brightness(number: u32, percent: u32) -> ActionResult {
    let percent = percent.clamp(0, 100);
    match run(Command::new("timeout").args(["10", "ddcutil", "setvcp", "10", &percent.to_string(), "-d", &number.to_string()])) {
        Ok(_) => ok(format!("Monitor {number} brightness set to {percent}%")),
        Err(e) => err(format!("The monitor didn't accept it: {e}")),
    }
}

// ---- arranging monitors ----

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MonitorSetup {
    pub name: String,
    /// e.g. "1920x1080@60.00Hz"
    pub mode: String,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// 0 normal, 1 = 90°, 2 = 180°, 3 = 270°.
    pub transform: u32,
    pub vrr: bool,
    /// Name of the monitor this one copies, or "".
    pub mirror_of: String,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct MonitorInfo {
    pub setup: MonitorSetup,
    pub description: String,
    pub available_modes: Vec<String>,
    pub width_mm: u32,
    pub height_mm: u32,
}

fn same_mode(a: &str, b: &str) -> bool {
    let parse = |m: &str| {
        let m = m.trim_end_matches("Hz");
        let (size, rate) = m.split_once('@').unwrap_or((m, "0"));
        (size.to_string(), rate.parse::<f64>().unwrap_or(0.0))
    };
    let (sa, ra) = parse(a);
    let (sb, rb) = parse(b);
    sa == sb && (ra - rb).abs() < 0.6
}

pub fn monitors() -> Vec<MonitorInfo> {
    let Ok(out) = run(Command::new("hyprctl").args(["monitors", "all", "-j"])) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&out) else {
        return Vec::new();
    };
    list.iter()
        .map(|m| {
            let mut modes: Vec<String> = m["availableModes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            let mut seen = std::collections::HashSet::new();
            modes.retain(|mode| seen.insert(mode.clone()));
            let current = format!("{}x{}@{:.2}Hz", m["width"], m["height"], m["refreshRate"].as_f64().unwrap_or(0.0));
            // Report the advertised mode it corresponds to, so it can be re-applied as is.
            let mode = modes.iter().find(|x| same_mode(x, &current)).cloned().unwrap_or(current);
            let mirror = m["mirrorOf"].as_str().unwrap_or("none");
            MonitorInfo {
                setup: MonitorSetup {
                    name: m["name"].as_str().unwrap_or_default().to_string(),
                    mode,
                    x: m["x"].as_i64().unwrap_or(0) as i32,
                    y: m["y"].as_i64().unwrap_or(0) as i32,
                    scale: m["scale"].as_f64().unwrap_or(1.0),
                    transform: m["transform"].as_u64().unwrap_or(0) as u32,
                    vrr: m["vrr"].as_bool().unwrap_or(false),
                    mirror_of: if mirror == "none" { String::new() } else { mirror.to_string() },
                    enabled: !m["disabled"].as_bool().unwrap_or(false),
                },
                description: m["description"].as_str().unwrap_or_default().to_string(),
                available_modes: modes,
                width_mm: m["physicalWidth"].as_u64().unwrap_or(0) as u32,
                height_mm: m["physicalHeight"].as_u64().unwrap_or(0) as u32,
            }
        })
        .collect()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Checks a requested setup against what the monitor and the session allow.
fn validate_setup(setup: &MonitorSetup, current: &[MonitorInfo]) -> Result<(), String> {
    let info = current
        .iter()
        .find(|m| m.setup.name == setup.name)
        .ok_or(format!("No monitor called {}", setup.name))?;
    if !info.available_modes.iter().any(|m| m == &setup.mode) {
        return Err(format!("{} doesn't offer {}", setup.name, setup.mode));
    }
    if !(0.5..=3.0).contains(&setup.scale) || setup.transform > 3 {
        return Err("Scale must be 0.5–3 and rotation one of the four directions".to_string());
    }
    if setup.x.abs() > 32000 || setup.y.abs() > 32000 {
        return Err("Position is out of range".to_string());
    }
    if !setup.mirror_of.is_empty() && (!valid_name(&setup.mirror_of) || setup.mirror_of == setup.name) {
        return Err("A monitor can only mirror another connected monitor".to_string());
    }
    if !setup.enabled && current.iter().filter(|m| m.setup.enabled).count() <= 1 && info.setup.enabled {
        return Err("That is the only screen switched on — turning it off would leave you blind".to_string());
    }
    Ok(())
}

fn lua_monitor(setup: &MonitorSetup) -> String {
    if !setup.enabled {
        return format!("hl.monitor({{ output = \"{}\", disabled = true }})", setup.name);
    }
    format!(
        "hl.monitor({{ output = \"{}\", mode = \"{}\", position = \"{}x{}\", scale = {}, transform = {}, vrr = {}, mirror = \"{}\" }})",
        setup.name,
        setup.mode.trim_end_matches("Hz"),
        setup.x,
        setup.y,
        setup.scale,
        setup.transform,
        setup.vrr as u8,
        setup.mirror_of
    )
}

/// Applies monitor settings for this session; the Hyprland config on disk
/// is left alone, so a reload or re-login restores what it says.
pub fn apply_monitors(setups: &[MonitorSetup]) -> ActionResult {
    let current = monitors();
    for setup in setups {
        if !valid_name(&setup.name) {
            return err("Not a monitor name");
        }
        if let Err(e) = validate_setup(setup, &current) {
            return err(e);
        }
    }
    if !crate::hypr::is_lua() {
        return err("Arranging monitors needs a Hyprland with the Lua config");
    }
    for setup in setups {
        if let Err(e) = run(Command::new("hyprctl").args(["eval", &lua_monitor(setup)])) {
            return err(format!("{}: {e}", setup.name));
        }
    }
    ok(format!("Applied to {} monitor(s) for this session", setups.len()))
}

// ---- saved layouts ----

#[derive(Serialize, Deserialize, Clone)]
pub struct Layout {
    pub name: String,
    /// Apply by itself whenever exactly these monitors are connected.
    pub auto: bool,
    pub monitors: Vec<MonitorSetup>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct NightLight {
    pub enabled: bool,
    /// Colour temperature in Kelvin while active (lower is warmer).
    pub kelvin: u32,
    /// "HH:MM" local time.
    pub from: String,
    pub until: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct DisplayConfig {
    #[serde(default)]
    pub layouts: Vec<Layout>,
    #[serde(default)]
    pub night_light: NightLight,
}

fn config_path() -> std::path::PathBuf {
    data_dir().join("display.json")
}

pub fn load_config() -> DisplayConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &DisplayConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(config_path(), text).map_err(|e| e.to_string())
}

pub fn save_layout(name: &str, auto: bool) -> ActionResult {
    let name = name.trim();
    if name.is_empty() || name.len() > 40 {
        return err("Give the layout a name of 1–40 characters");
    }
    let mut config = load_config();
    config.layouts.retain(|l| l.name != name);
    config.layouts.push(Layout {
        name: name.to_string(),
        auto,
        monitors: monitors().into_iter().map(|m| m.setup).collect(),
    });
    match save_config(&config) {
        Ok(()) => ok(format!("Saved the current arrangement as “{name}”")),
        Err(e) => err(e),
    }
}

pub fn delete_layout(name: &str) -> ActionResult {
    let mut config = load_config();
    config.layouts.retain(|l| l.name != name);
    match save_config(&config) {
        Ok(()) => ok("Layout deleted"),
        Err(e) => err(e),
    }
}

pub fn apply_layout(name: &str) -> ActionResult {
    let Some(layout) = load_config().layouts.into_iter().find(|l| l.name == name) else {
        return err("No layout with that name");
    };
    let connected: Vec<String> = monitors().into_iter().map(|m| m.setup.name).collect();
    let usable: Vec<MonitorSetup> = layout.monitors.into_iter().filter(|m| connected.contains(&m.name)).collect();
    if usable.is_empty() {
        return err("None of that layout's monitors are connected");
    }
    apply_monitors(&usable)
}

fn minutes_of(clock: &str) -> Option<u32> {
    let (h, m) = clock.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Whether `now` (minutes since midnight) falls in the window, which may
/// cross midnight (20:00 → 07:00).
fn in_window(now: u32, from: u32, until: u32) -> bool {
    if from <= until {
        now >= from && now < until
    } else {
        now >= from || now < until
    }
}

/// What the background loop remembers about displays between ticks.
#[derive(Default)]
pub struct DisplayState {
    connected: Option<Vec<String>>,
    night_on: Option<bool>,
}

/// Called every tick: applies an automatic layout when the set of connected
/// monitors changes, and switches the night light at its set times.
pub fn tick(state: &mut DisplayState) -> Option<String> {
    let config = load_config();
    let mut note = None;

    if config.layouts.iter().any(|l| l.auto) {
        let mut connected: Vec<String> = monitors().into_iter().map(|m| m.setup.name).collect();
        connected.sort();
        let changed = state.connected.as_ref().is_some_and(|before| *before != connected);
        if changed {
            let matching = config.layouts.iter().find(|l| {
                let mut names: Vec<String> = l.monitors.iter().map(|m| m.name.clone()).collect();
                names.sort();
                l.auto && names == connected
            });
            if let Some(layout) = matching {
                let result = apply_layout(&layout.name);
                note = Some(format!("Display layout “{}”: {}", layout.name, result.message));
            }
        }
        state.connected = Some(connected);
    } else {
        state.connected = None;
    }

    let night = &config.night_light;
    if night.enabled {
        if let (Some(from), Some(until)) = (minutes_of(&night.from), minutes_of(&night.until)) {
            let now = crate::sys_util::local_time();
            let active = in_window(now.hour * 60 + now.minute, from, until);
            if state.night_on != Some(active) {
                crate::connectivity::set_night_light(active.then_some(night.kelvin.clamp(1000, 6500)));
                state.night_on = Some(active);
            }
        }
    } else if state.night_on.take() == Some(true) {
        crate::connectivity::set_night_light(None);
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drm_client_info() {
        let text = "pos:\t0\ndrm-driver:\tamdgpu\ndrm-client-id:\t38\ndrm-pdev:\t0000:04:00.0\ndrm-total-gtt:\t9 MiB\ndrm-resident-cpu:\t1 MiB\ndrm-resident-gtt:\t2 MiB\ndrm-resident-vram:\t12 KiB\ndrm-engine-gfx:\t1500000000 ns\ndrm-engine-enc:\t500000000 ns\n";
        let c = parse_fdinfo(text).unwrap();
        assert_eq!((c.pdev.as_str(), c.id.as_str()), ("0000:04:00.0", "38"));
        assert_eq!(c.memory, 2 * 1024 * 1024 + 12 * 1024);
        assert_eq!(c.engine_ns, 2_000_000_000);
        assert!(parse_fdinfo("pos:\t0\nflags:\t02\n").is_none());
    }

    #[test]
    fn night_window_can_cross_midnight() {
        let (from, until) = (minutes_of("20:00").unwrap(), minutes_of("07:00").unwrap());
        assert!(in_window(23 * 60, from, until));
        assert!(in_window(6 * 60, from, until));
        assert!(!in_window(12 * 60, from, until));
        assert!(in_window(13 * 60, 12 * 60, 14 * 60));
        assert!(minutes_of("24:00").is_none() && minutes_of("7").is_none());
    }

    fn info(name: &str, enabled: bool) -> MonitorInfo {
        MonitorInfo {
            setup: MonitorSetup {
                name: name.to_string(),
                mode: "1920x1080@60.00Hz".to_string(),
                x: 0,
                y: 0,
                scale: 1.0,
                transform: 0,
                vrr: false,
                mirror_of: String::new(),
                enabled,
            },
            description: String::new(),
            available_modes: vec!["1920x1080@60.00Hz".to_string()],
            width_mm: 0,
            height_mm: 0,
        }
    }

    #[test]
    fn refuses_bad_monitor_setups() {
        let current = vec![info("eDP-1", true), info("HDMI-A-1", false)];
        let mut setup = current[0].setup.clone();
        assert!(validate_setup(&setup, &current).is_ok());
        setup.mode = "9999x9999@1.00Hz".to_string();
        assert!(validate_setup(&setup, &current).is_err());
        let mut off = current[0].setup.clone();
        off.enabled = false;
        assert!(validate_setup(&off, &current).is_err(), "can't turn off the last screen");
        let mut mirror = current[0].setup.clone();
        mirror.mirror_of = "eDP-1".to_string();
        assert!(validate_setup(&mirror, &current).is_err());
        let mut rotated = current[0].setup.clone();
        rotated.transform = 9;
        assert!(validate_setup(&rotated, &current).is_err());
    }

    #[test]
    fn builds_monitor_lua() {
        let mut setup = info("HDMI-A-1", true).setup;
        setup.x = 1920;
        setup.transform = 1;
        assert_eq!(
            lua_monitor(&setup),
            r#"hl.monitor({ output = "HDMI-A-1", mode = "1920x1080@60.00", position = "1920x0", scale = 1, transform = 1, vrr = 0, mirror = "" })"#
        );
        setup.enabled = false;
        assert_eq!(lua_monitor(&setup), r#"hl.monitor({ output = "HDMI-A-1", disabled = true })"#);
    }
}
