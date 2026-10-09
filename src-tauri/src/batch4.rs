//! Measuring and explaining: power per app, idle detection, disk growth,
//! before/after measurements, physical-access log, connections per app,
//! "what is this?", speed tests, Wi-Fi channels and drive wear history.

use crate::db::Db;
use crate::sys_util::{data_dir, has_tool, read_num, read_trimmed, run, run_lossy};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::process::Command;
use std::sync::Mutex;

pub fn init(db: &Db) {
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS power_samples (ts INTEGER NOT NULL, cpu_percent REAL NOT NULL, watts REAL NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_power_samples_ts ON power_samples(ts);
             CREATE TABLE IF NOT EXISTS disk_folders (ts INTEGER NOT NULL, path TEXT NOT NULL, bytes INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_disk_folders_ts ON disk_folders(ts);
             CREATE TABLE IF NOT EXISTS seen_hosts (app TEXT NOT NULL, host TEXT NOT NULL, first_ts INTEGER NOT NULL, last_ts INTEGER NOT NULL, PRIMARY KEY (app, host));
             CREATE TABLE IF NOT EXISTS speed_tests (ts INTEGER NOT NULL, network TEXT NOT NULL, down_mbps REAL NOT NULL, latency_ms REAL NOT NULL);
             CREATE TABLE IF NOT EXISTS drive_wear (ts INTEGER NOT NULL, device TEXT NOT NULL, wear_percent INTEGER, power_on_hours INTEGER);",
        )
        .ok();
}

// ---- battery ----

pub struct BatteryNow {
    pub watts: f64,
    pub discharging: bool,
    pub energy_wh: f64,
    pub percent: u32,
}

pub fn battery_now() -> Option<BatteryNow> {
    let base = crate::tweaks::expand_globs(&["/sys/class/power_supply/BAT*"]).into_iter().next()?;
    let micro = |f: &str| read_num::<f64>(&format!("{base}/{f}")).map(|v| v / 1e6);
    // Some batteries report current and voltage instead of power and energy.
    let volts = micro("voltage_now").unwrap_or(0.0);
    let watts = micro("power_now").or_else(|| Some(micro("current_now")? * volts))?;
    let energy_wh = micro("energy_now").or_else(|| Some(micro("charge_now")? * volts)).unwrap_or(0.0);
    Some(BatteryNow {
        watts,
        discharging: read_trimmed(&format!("{base}/status")).as_deref() == Some("Discharging"),
        energy_wh,
        percent: read_num(&format!("{base}/capacity")).unwrap_or(0),
    })
}

/// Remembers how much the machine draws at each level of processor use,
/// while on battery. That is what lets drain be split between apps later.
pub fn record_power(db: &Db, now: i64, cpu_percent: f32) {
    let Some(b) = battery_now().filter(|b| b.discharging && b.watts > 0.5) else {
        return;
    };
    let conn = db.conn.lock().unwrap();
    conn.execute("INSERT INTO power_samples VALUES (?1, ?2, ?3)", params![now, cpu_percent as f64, b.watts]).ok();
    conn.execute("DELETE FROM power_samples WHERE ts < ?1", params![now - 14 * 86_400]).ok();
}

/// Least-squares line through (cpu %, watts): returns (watts at idle, watts per cpu %).
fn fit_power(samples: &[(f64, f64)]) -> Option<(f64, f64)> {
    if samples.len() < 30 {
        return None;
    }
    let n = samples.len() as f64;
    let (sx, sy) = samples.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (mx, my) = (sx / n, sy / n);
    let var: f64 = samples.iter().map(|(x, _)| (x - mx).powi(2)).sum();
    if var < n * 4.0 {
        return None; // the load never varied enough to tell
    }
    let cov: f64 = samples.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let slope = (cov / var).max(0.0);
    Some(((my - slope * mx).max(0.0), slope))
}

#[derive(Serialize)]
pub struct AppPower {
    pub app: String,
    /// Share of the whole processor, 0–100.
    pub cpu_percent: f64,
    pub watts: f64,
    /// Battery minutes gained if this app weren't running (on battery only).
    pub minutes_gained: Option<f64>,
}

#[derive(Serialize)]
pub struct PowerReport {
    pub on_battery: bool,
    pub total_watts: f64,
    /// What the machine draws doing nothing: screen, memory, radios.
    pub idle_watts: f64,
    /// True when the split comes from this machine's own recorded history.
    pub measured: bool,
    pub apps: Vec<AppPower>,
}

fn cpu_ticks_by_app() -> (HashMap<String, u64>, u64) {
    let mut by_app: HashMap<String, u64> = HashMap::new();
    if let Ok(dir) = fs::read_dir("/proc") {
        for entry in dir.flatten() {
            let pid = entry.file_name().to_string_lossy().to_string();
            if !pid.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            // Fields after the "(name)" — utime and stime are the 12th and 13th.
            let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
                continue;
            };
            let f: Vec<&str> = rest.split_whitespace().collect();
            let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            if let Some(name) = crate::memory_map::app_name(&pid) {
                *by_app.entry(name).or_default() += ticks;
            }
        }
    }
    let total: u64 = fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.split_whitespace().skip(1).filter_map(|v| v.parse::<u64>().ok()).sum()))
        .unwrap_or(0);
    (by_app, total)
}

/// Watches every app's processor time for three seconds and splits the
/// machine's power draw between them.
pub fn power_report(db: &Db) -> PowerReport {
    let (before, total_before) = cpu_ticks_by_app();
    std::thread::sleep(std::time::Duration::from_secs(3));
    let (after, total_after) = cpu_ticks_by_app();
    let total = total_after.saturating_sub(total_before).max(1) as f64;

    let battery = battery_now();
    let on_battery = battery.as_ref().is_some_and(|b| b.discharging);
    let samples: Vec<(f64, f64)> = {
        let conn = db.conn.lock().unwrap();
        conn.prepare("SELECT cpu_percent, watts FROM power_samples ORDER BY ts DESC LIMIT 5000")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default()
    };
    let fit = fit_power(&samples);
    // Without enough history, assume a typical laptop: about a third of a
    // watt for each percent of the whole processor in use.
    let (idle_watts, per_percent) = fit.unwrap_or((0.0, 0.35));
    // While charging, the battery's figure is what flows in, not what the machine uses.
    let total_watts = battery.as_ref().filter(|b| b.discharging).map(|b| b.watts).unwrap_or(0.0);

    let mut apps: Vec<AppPower> = after
        .into_iter()
        .filter_map(|(app, ticks)| {
            let used = ticks.saturating_sub(*before.get(&app).unwrap_or(&ticks)) as f64;
            let cpu_percent = used / total * 100.0;
            if cpu_percent < 0.2 {
                return None;
            }
            let watts = cpu_percent * per_percent;
            let minutes_gained = battery.as_ref().filter(|b| b.discharging && b.watts > watts + 1.0).map(|b| (b.energy_wh / (b.watts - watts) - b.energy_wh / b.watts) * 60.0);
            Some(AppPower { app, cpu_percent, watts, minutes_gained })
        })
        .collect();
    apps.sort_by(|a, b| b.watts.total_cmp(&a.watts));
    apps.truncate(12);
    PowerReport {
        on_battery,
        total_watts,
        idle_watts: if fit.is_some() { idle_watts } else if on_battery { (total_watts - apps.iter().map(|a| a.watts).sum::<f64>()).max(0.0) } else { 0.0 },
        measured: fit.is_some(),
        apps,
    }
}

// ---- are you actually there? ----

struct IdleWatch {
    signature: String,
    last_active: i64,
}

static IDLE: Mutex<IdleWatch> = Mutex::new(IdleWatch { signature: String::new(), last_active: 0 });

const AWAY_AFTER_SECS: i64 = 300;

/// False once neither the pointer nor the built-in keyboard has moved for
/// five minutes and nothing is playing sound — so a window left focused
/// overnight stops counting as use.
pub fn user_present(now: i64) -> bool {
    let keyboard = fs::read_to_string("/proc/interrupts")
        .ok()
        .and_then(|t| t.lines().find(|l| l.contains("i8042")).map(|l| l.split_whitespace().take(17).collect::<Vec<_>>().join(" ")))
        .unwrap_or_default();
    let signature = format!("{}|{keyboard}", run_lossy(Command::new("hyprctl").arg("cursorpos")));
    let mut idle = IDLE.lock().unwrap();
    if idle.signature != signature || idle.last_active == 0 {
        idle.signature = signature;
        idle.last_active = now;
        return true;
    }
    if now - idle.last_active < AWAY_AFTER_SECS {
        return true;
    }
    drop(idle);
    // Watching a film or on a call: hands off the keyboard, but not away.
    sound_playing()
}

fn sound_playing() -> bool {
    let out = run_lossy(Command::new("pactl").args(["-f", "json", "list", "sink-inputs"]));
    serde_json::from_str::<serde_json::Value>(&out)
        .ok()
        .and_then(|v| v.as_array().map(|list| list.iter().any(|i| i["corked"].as_bool() == Some(false))))
        .unwrap_or(false)
}

// ---- what grew on the disk ----

static SNAPSHOT_RUNNING: Mutex<bool> = Mutex::new(false);

fn folder_sizes() -> Vec<(String, u64)> {
    let mut rows = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return rows;
    };
    let mut scan = |path: &str, depth: &str| {
        // Idle priority: this reads the whole tree and must not make the desktop stutter.
        let out = run_lossy(Command::new("nice").args(["-n", "19", "ionice", "-c", "3", "du", "-x", "-B1", "-d", depth, path]));
        for line in out.lines() {
            if let Some((bytes, folder)) = line.split_once('\t') {
                if let Ok(bytes) = bytes.trim().parse::<u64>() {
                    if bytes >= 50 * 1024 * 1024 {
                        rows.push((folder.to_string(), bytes));
                    }
                }
            }
        }
    };
    scan(&home.to_string_lossy(), "2");
    scan("/var", "2");
    rows
}

static SNAPSHOT_DONE: Mutex<Option<(i64, Vec<(String, u64)>)>> = Mutex::new(None);

/// Measures every sizeable folder. Slow (it walks the disk), so it runs on
/// its own thread and `store_folder_snapshot` saves the result afterwards.
/// Returns false if one is already running.
pub fn snapshot_folders(now: i64) -> bool {
    {
        let mut running = SNAPSHOT_RUNNING.lock().unwrap();
        if *running {
            return false;
        }
        *running = true;
    }
    std::thread::spawn(move || {
        let rows = folder_sizes();
        *SNAPSHOT_DONE.lock().unwrap() = Some((now, rows));
        *SNAPSHOT_RUNNING.lock().unwrap() = false;
    });
    true
}

/// Saves a finished measurement, if there is one waiting.
pub fn store_folder_snapshot(db: &Db) {
    let Some((ts, rows)) = SNAPSHOT_DONE.lock().unwrap().take() else {
        return;
    };
    let mut conn = db.conn.lock().unwrap();
    let Ok(tx) = conn.transaction() else {
        return;
    };
    for (path, bytes) in &rows {
        tx.execute("INSERT INTO disk_folders VALUES (?1, ?2, ?3)", params![ts, path, *bytes as i64]).ok();
    }
    tx.execute("DELETE FROM disk_folders WHERE ts < ?1", params![ts - 200 * 86_400]).ok();
    tx.commit().ok();
}

pub fn last_folder_snapshot(db: &Db) -> i64 {
    db.conn.lock().unwrap().query_row("SELECT COALESCE(MAX(ts), 0) FROM disk_folders", [], |r| r.get(0)).unwrap_or(0)
}

#[derive(Serialize)]
pub struct FolderGrowth {
    pub path: String,
    pub before: u64,
    pub now: u64,
}

#[derive(Serialize)]
pub struct GrowthReport {
    pub running: bool,
    pub newest: i64,
    pub compared_to: i64,
    pub snapshots: Vec<i64>,
    pub folders: Vec<FolderGrowth>,
}

/// Compares the newest snapshot against `against` (or the one before it).
pub fn folder_growth(db: &Db, against: Option<i64>) -> GrowthReport {
    store_folder_snapshot(db);
    let conn = db.conn.lock().unwrap();
    let snapshots: Vec<i64> = conn
        .prepare("SELECT DISTINCT ts FROM disk_folders ORDER BY ts DESC")
        .and_then(|mut s| s.query_map([], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default();
    let running = *SNAPSHOT_RUNNING.lock().unwrap();
    let newest = snapshots.first().copied().unwrap_or(0);
    let compared_to = against.filter(|ts| snapshots.contains(ts) && *ts != newest).or(snapshots.get(1).copied()).unwrap_or(0);
    let load = |ts: i64| -> BTreeMap<String, u64> {
        conn.prepare("SELECT path, bytes FROM disk_folders WHERE ts = ?1")
            .and_then(|mut s| s.query_map(params![ts], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default()
    };
    let mut folders = Vec::new();
    if compared_to != 0 {
        let (old, new) = (load(compared_to), load(newest));
        folders = pick_growth(&old, &new);
    }
    GrowthReport { running, newest, compared_to, snapshots, folders }
}

/// The folders that changed most, deepest first: when `~/a/b` explains the
/// growth of `~/a`, only `~/a/b` is listed.
fn pick_growth(old: &BTreeMap<String, u64>, new: &BTreeMap<String, u64>) -> Vec<FolderGrowth> {
    let mut changed: Vec<FolderGrowth> = new
        .iter()
        .map(|(path, now)| FolderGrowth { path: path.clone(), before: *old.get(path).unwrap_or(&0), now: *now })
        .filter(|f| f.now.abs_diff(f.before) >= 100 * 1024 * 1024)
        .collect();
    let deltas: Vec<(String, i64)> = changed.iter().map(|f| (f.path.clone(), f.now as i64 - f.before as i64)).collect();
    changed.retain(|f| {
        let delta = f.now as i64 - f.before as i64;
        let prefix = format!("{}/", f.path);
        // Drop a parent when its children account for most of its change.
        let children: i64 = deltas.iter().filter(|(p, _)| p.starts_with(&prefix) && !p[prefix.len()..].contains('/')).map(|(_, d)| *d).sum();
        (delta - children).abs() >= 100 * 1024 * 1024 || children == 0
    });
    changed.sort_by_key(|f| std::cmp::Reverse((f.now as i64 - f.before as i64).abs()));
    changed.truncate(25);
    changed
}

// ---- measuring before and after ----

#[derive(Serialize, Deserialize, Clone)]
pub struct Measurement {
    pub ts: i64,
    pub label: String,
    pub seconds: u64,
    pub cpu_percent: f64,
    pub temp_c: f64,
    /// Battery draw; None when plugged in.
    pub watts: Option<f64>,
    /// Share of time something was stalled waiting for memory / disk.
    pub memory_stall: f64,
    pub io_stall: f64,
}

fn measurements_path() -> std::path::PathBuf {
    data_dir().join("measurements.json")
}

pub fn measurements() -> Vec<Measurement> {
    fs::read_to_string(measurements_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn cpu_times() -> (u64, u64) {
    let line = fs::read_to_string("/proc/stat").unwrap_or_default();
    let v: Vec<u64> = line.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    (v.iter().sum(), idle)
}

/// Total microseconds stalled, from the kernel's pressure counters.
fn stall_total(resource: &str) -> u64 {
    fs::read_to_string(format!("/proc/pressure/{resource}"))
        .ok()
        .and_then(|t| t.lines().next().and_then(|l| l.split("total=").nth(1)).and_then(|v| v.trim().parse().ok()))
        .unwrap_or(0)
}

/// Watches the machine for `seconds` while you use it normally and stores
/// the averages under `label`, to compare against another run.
pub fn measure(label: &str, seconds: u64, now: i64) -> Vec<Measurement> {
    let seconds = seconds.clamp(15, 600);
    let (total0, idle0) = cpu_times();
    let (mem0, io0) = (stall_total("memory"), stall_total("io"));
    let (mut temp_sum, mut watt_sum, mut watt_n) = (0.0, 0.0, 0u32);
    for _ in 0..seconds {
        std::thread::sleep(std::time::Duration::from_secs(1));
        temp_sum += crate::hardware::sensors().iter().filter(|s| s.kind == "temp").map(|s| s.value as f64).fold(0.0, f64::max);
        if let Some(b) = battery_now().filter(|b| b.discharging) {
            watt_sum += b.watts;
            watt_n += 1;
        }
    }
    let (total1, idle1) = cpu_times();
    let span = (total1 - total0).max(1) as f64;
    let micros = seconds as f64 * 1e6;
    let m = Measurement {
        ts: now,
        label: label.trim().chars().take(40).collect(),
        seconds,
        cpu_percent: (1.0 - (idle1 - idle0) as f64 / span) * 100.0,
        temp_c: temp_sum / seconds as f64,
        watts: (watt_n > 0).then(|| watt_sum / watt_n as f64),
        memory_stall: (stall_total("memory") - mem0) as f64 / micros * 100.0,
        io_stall: (stall_total("io") - io0) as f64 / micros * 100.0,
    };
    let mut all = measurements();
    all.insert(0, m);
    all.truncate(12);
    fs::write(measurements_path(), serde_json::to_string(&all).unwrap_or_default()).ok();
    all
}

pub fn clear_measurements() {
    fs::remove_file(measurements_path()).ok();
}

// ---- physical access ----

#[derive(Serialize)]
pub struct AccessEvent {
    pub when: String,
    /// "usb" | "login" | "lid"
    pub kind: String,
    pub text: String,
}

fn journal(args: &[&str], pattern: &str) -> Vec<(String, String)> {
    let mut cmd = Command::new("journalctl");
    cmd.args(["--since=-7d", "-q", "--no-pager", "-o", "short-iso"]).args(args).args(["-g", pattern]);
    run_lossy(&mut cmd)
        .lines()
        .filter_map(|l| {
            // "2026-10-09T18:39:45+05:30 host program[pid]: message"
            let mut parts = l.splitn(3, ' ');
            let when = parts.next()?.get(..16)?.replace('T', " ");
            parts.next()?;
            Some((when, parts.next()?.to_string()))
        })
        .collect()
}

pub fn failed_logins() -> Vec<(String, String)> {
    journal(&[], "authentication failure|Failed password|FAILED LOGIN|incorrect password")
        .into_iter()
        .map(|(when, rest)| {
            let program = rest.split(['[', ':']).next().unwrap_or("");
            let what = match program {
                "qs" | "hyprlock" | "swaylock" | "gtklock" => "lock screen",
                "sshd" | "sshd-session" => "remote login (SSH)",
                "sudo" | "su" => "sudo in a terminal",
                p if p.starts_with("polkit") => "password prompt",
                "login" | "sddm-helper" | "gdm-password" | "greetd" => "login screen",
                other => other,
            };
            (when, format!("Wrong password at the {what}"))
        })
        .collect()
}

/// Things someone at the machine did in the last week: devices plugged in,
/// wrong passwords, the lid.
pub fn access_log() -> Vec<AccessEvent> {
    let mut events: Vec<AccessEvent> = Vec::new();
    for (when, rest) in journal(&["_TRANSPORT=kernel"], "usb [0-9.-]+: Product: ") {
        if let Some(name) = rest.split("Product: ").nth(1) {
            events.push(AccessEvent { when, kind: "usb".into(), text: format!("USB device plugged in: {}", name.trim()) });
        }
    }
    for (when, text) in failed_logins() {
        events.push(AccessEvent { when, kind: "login".into(), text });
    }
    for (when, rest) in journal(&["-t", "systemd-logind"], "Lid opened|Lid closed") {
        events.push(AccessEvent { when, kind: "lid".into(), text: if rest.contains("opened") { "Lid opened".into() } else { "Lid closed".into() } });
    }
    events.sort_by(|a, b| b.when.cmp(&a.when));
    events.truncate(150);
    events
}

// ---- who is each app talking to ----

#[derive(Serialize)]
pub struct Peer {
    pub host: String,
    pub port: u16,
    /// First seen less than a day ago.
    pub new: bool,
    pub first_seen: i64,
}

#[derive(Serialize)]
pub struct AppPeers {
    pub app: String,
    pub peers: Vec<Peer>,
    /// Every address this app has ever been seen talking to.
    pub known_hosts: u32,
}

fn is_local_address(ip: &str) -> bool {
    ip.starts_with("127.") || ip == "::1" || ip.starts_with("[::1]") || ip.starts_with("::ffff:127.")
}

fn live_connections() -> Vec<(String, String, u16)> {
    let out = run_lossy(Command::new("ss").args(["-tnpH", "state", "established"]));
    let mut found = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(peer), Some(users)) = (f.get(3), f.iter().find(|c| c.starts_with("users:"))) else {
            continue;
        };
        let Some((host, port)) = peer.rsplit_once(':') else {
            continue;
        };
        let host = host.trim_matches(['[', ']']).to_string();
        if is_local_address(&host) {
            continue;
        }
        let Some(pid) = users.split("pid=").nth(1).map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>()) else {
            continue;
        };
        if let Some(app) = crate::memory_map::app_name(&pid) {
            let entry = (app, host, port.parse().unwrap_or(0));
            if !found.contains(&entry) {
                found.push(entry);
            }
        }
    }
    found
}

/// Notes which addresses each app is connected to. Called once a minute so
/// short-lived connections are caught too.
pub fn record_connections(db: &Db, now: i64) {
    let live = live_connections();
    let conn = db.conn.lock().unwrap();
    for (app, host, _) in &live {
        conn.execute(
            "INSERT INTO seen_hosts VALUES (?1, ?2, ?3, ?3) ON CONFLICT(app, host) DO UPDATE SET last_ts = ?3",
            params![app, host, now],
        )
        .ok();
    }
    conn.execute("DELETE FROM seen_hosts WHERE last_ts < ?1", params![now - 90 * 86_400]).ok();
}

pub fn connections(db: &Db, now: i64) -> Vec<AppPeers> {
    record_connections(db, now);
    let live = live_connections();
    let conn = db.conn.lock().unwrap();
    let mut by_app: BTreeMap<String, Vec<Peer>> = BTreeMap::new();
    for (app, host, port) in live {
        let first_seen: i64 = conn.query_row("SELECT first_ts FROM seen_hosts WHERE app = ?1 AND host = ?2", params![app, host], |r| r.get(0)).unwrap_or(now);
        by_app.entry(app).or_default().push(Peer { host, port, new: now - first_seen < 86_400, first_seen });
    }
    let mut list: Vec<AppPeers> = by_app
        .into_iter()
        .map(|(app, mut peers)| {
            peers.sort_by(|a, b| b.first_seen.cmp(&a.first_seen));
            let known_hosts = conn.query_row("SELECT COUNT(*) FROM seen_hosts WHERE app = ?1", params![app], |r| r.get(0)).unwrap_or(0);
            AppPeers { app, peers, known_hosts }
        })
        .collect();
    list.sort_by_key(|a| std::cmp::Reverse(a.peers.len()));
    list
}

/// The name behind an address, if it has one.
pub fn host_name(ip: &str) -> String {
    if ip.parse::<std::net::IpAddr>().is_err() {
        return String::new();
    }
    run_lossy(Command::new("timeout").args(["3", "getent", "hosts", ip])).split_whitespace().nth(1).unwrap_or("").to_string()
}

// ---- what is this? ----

#[derive(Serialize)]
pub struct WhatIs {
    pub name: String,
    pub what: String,
    /// "yes" | "careful" | "no" | "" (unknown)
    pub safe_to_stop: String,
    pub note: String,
    /// "Trace's reference" | "its package description" | "its service description"
    pub source: String,
}

/// (name, what it is, safe to stop, note). Names are matched without a
/// ".service" suffix and case-insensitively.
const KNOWN: &[(&str, &str, &str, &str)] = &[
    ("systemd", "The first program Linux starts; it starts and supervises everything else.", "no", "Stopping it stops the machine."),
    ("systemd-journald", "Collects the system's log messages.", "no", "It restarts by itself; heavy use means something is logging a lot."),
    ("systemd-logind", "Tracks who is logged in and handles the lid and power buttons.", "no", "Stopping it ends your session."),
    ("systemd-udevd", "Reacts to hardware being plugged in or removed.", "no", ""),
    ("systemd-resolved", "Looks up site names (DNS) for the whole system.", "careful", "Stopping it breaks name lookups until it is started again."),
    ("systemd-oomd", "Ends apps when memory runs out, before the machine freezes.", "careful", "Without it a full memory can lock the desktop up."),
    ("systemd-timesyncd", "Keeps the clock right over the network.", "yes", ""),
    ("dbus-broker", "The message bus programs use to talk to each other.", "no", "Most of the desktop stops working without it."),
    ("dbus-daemon", "The message bus programs use to talk to each other.", "no", "Most of the desktop stops working without it."),
    ("NetworkManager", "Manages Wi-Fi, wired and VPN connections.", "careful", "Stopping it drops the network."),
    ("wpa_supplicant", "Does the Wi-Fi password handshake for NetworkManager.", "careful", "Stopping it drops Wi-Fi."),
    ("iwd", "Wi-Fi connection service.", "careful", "Stopping it drops Wi-Fi."),
    ("bluetoothd", "The Bluetooth service.", "yes", "Bluetooth devices disconnect."),
    ("pipewire", "Carries all sound and screen-sharing video.", "careful", "Sound stops until it restarts (it usually restarts by itself)."),
    ("pipewire-pulse", "Lets apps written for PulseAudio play sound through PipeWire.", "careful", "Most apps lose sound until it restarts."),
    ("wireplumber", "Decides which app's sound goes to which speaker or microphone.", "careful", "Sound routing stops until it restarts."),
    ("Hyprland", "Your desktop: it draws every window.", "no", "Stopping it closes every app and logs you out."),
    ("hyprland", "Your desktop: it draws every window.", "no", "Stopping it closes every app and logs you out."),
    ("Xwayland", "Runs older apps that don't speak Wayland yet (many games, some Electron apps).", "careful", "Those apps close."),
    ("qs", "Quickshell: your bar, launcher, notifications and lock screen.", "careful", "The bar and notifications disappear until it is started again."),
    ("quickshell", "Your bar, launcher, notifications and lock screen.", "careful", "The bar and notifications disappear until it is started again."),
    ("waybar", "The status bar.", "yes", "The bar disappears."),
    ("hypridle", "Dims, locks and sleeps the machine when you're away.", "yes", "The screen stops locking by itself."),
    ("hyprsunset", "Warms the screen colour at night.", "yes", ""),
    ("hyprpaper", "Draws the wallpaper.", "yes", ""),
    ("xdg-desktop-portal", "Lets apps open file pickers, share the screen and ask for permissions.", "careful", "File pickers and screen sharing stop working until it restarts."),
    ("xdg-desktop-portal-hyprland", "The screen-sharing and screenshot part of the portal for Hyprland.", "careful", "Screen sharing stops."),
    ("polkitd", "Decides who may do administrator things; shows the password prompt.", "careful", "Nothing can ask for administrator rights without it."),
    ("upowerd", "Reports battery and charger state to the desktop.", "yes", "Battery indicators stop updating."),
    ("power-profiles-daemon", "Switches between power saver, balanced and performance.", "yes", "The power profile can't be changed."),
    ("thermald", "Slows the processor before it overheats (Intel).", "careful", ""),
    ("firewalld", "The firewall.", "careful", "Every listening program becomes reachable from the network."),
    ("sshd", "Lets people log into this machine remotely.", "yes", "Remote logins stop; turn it off if you don't use them."),
    ("cupsd", "The printing service.", "yes", "Printing stops."),
    ("avahi-daemon", "Announces this machine on the local network and finds printers and other devices.", "yes", "Network printers and .local names stop being found."),
    ("dockerd", "Runs Docker containers.", "careful", "Every container stops."),
    ("containerd", "The engine underneath Docker.", "careful", "Every container stops."),
    ("kdeconnectd", "Links this machine with your phone (KDE Connect).", "yes", ""),
    ("gnome-keyring-daemon", "Stores saved passwords and unlocks them when you log in.", "careful", "Apps ask for passwords again."),
    ("kwalletd6", "Stores saved passwords (KDE Wallet).", "careful", "Apps ask for passwords again."),
    ("gvfsd", "Shows phones, network shares and the trash in file managers.", "yes", ""),
    ("tracker-miner-fs-3", "Indexes your files for search.", "yes", "It can use a lot of disk and CPU right after big file changes."),
    ("baloo_file", "Indexes your files for search (KDE).", "yes", "It can use a lot of disk and CPU right after big file changes."),
    ("packagekitd", "Checks for and installs updates for graphical software centres.", "yes", ""),
    ("fwupd", "Checks for firmware updates.", "yes", ""),
    ("nvidia-persistenced", "Keeps the NVIDIA card initialised so it starts faster.", "yes", ""),
    ("nvidia-powerd", "Balances power between processor and NVIDIA card on laptops.", "yes", ""),
    ("asusd", "ASUS laptop control: keyboard light, charge limit, fan profiles.", "yes", ""),
    ("kworker", "A kernel worker thread: the kernel doing its own housekeeping.", "no", "It can't be stopped; high use points at a driver or hardware problem."),
    ("kswapd0", "The kernel freeing memory by moving data to swap.", "no", "Busy means memory is short — close something."),
    ("kcompactd0", "The kernel tidying memory into larger free blocks.", "no", ""),
    ("irq", "The kernel handling signals from hardware.", "no", "High use usually means a misbehaving device."),
    ("zram", "Compresses memory instead of writing it to disk.", "no", ""),
    ("cliphist", "Remembers your clipboard history.", "yes", ""),
    ("wl-paste", "Feeds the clipboard to the clipboard history.", "yes", ""),
    ("ydotoold", "Lets scripts type and click for you.", "yes", ""),
    ("crashpad_handler", "Collects crash reports for a Chromium-based app.", "yes", ""),
    ("chrome_crashpad_handler", "Collects crash reports for a Chromium-based app.", "yes", ""),
    ("at-spi-bus-launcher", "Accessibility support (screen readers).", "yes", ""),
    ("rtkit-daemon", "Lets the sound system run at high priority so audio doesn't crackle.", "yes", ""),
    ("trace", "This app.", "yes", "Recording, rules and schedules stop."),
];

pub fn what_is(name: &str) -> WhatIs {
    let name = name.trim();
    let bare = name.trim_end_matches(".service").trim_end_matches(".socket").trim_end_matches(".timer");
    let key = bare.split(['/', ':', '-']).next().filter(|k| ["kworker", "irq", "zram"].contains(k)).unwrap_or(bare);
    if let Some((_, what, safe, note)) = KNOWN.iter().find(|(n, ..)| n.eq_ignore_ascii_case(key)) {
        return WhatIs { name: name.into(), what: what.to_string(), safe_to_stop: safe.to_string(), note: note.to_string(), source: "Trace's reference".into() };
    }
    let unknown = |what: String, source: &str| WhatIs { name: name.into(), what, safe_to_stop: String::new(), note: String::new(), source: source.into() };
    if !crate::sys_util::is_safe_token(bare) {
        return unknown(String::new(), "");
    }
    if name.contains('.') {
        let description = run_lossy(Command::new("systemctl").args(["show", "-p", "Description", "--value", name]));
        let user = run_lossy(Command::new("systemctl").args(["--user", "show", "-p", "Description", "--value", name]));
        let description = if description.is_empty() || description == name { user } else { description };
        if !description.is_empty() && description != name {
            return unknown(description, "its service description");
        }
    }
    // Fall back to what the package that installed the program says about itself.
    if let Ok(path) = run(Command::new("which").arg(bare)) {
        if let Ok(owner) = run(Command::new("pacman").args(["-Qoq", &path])) {
            let info = run_lossy(Command::new("pacman").args(["-Qi", owner.trim()]));
            if let Some(description) = info.lines().find(|l| l.starts_with("Description")).and_then(|l| l.split_once(": ")).map(|(_, d)| d.trim()) {
                return unknown(format!("{description} (from the {} package)", owner.trim()), "its package description");
            }
        }
    }
    unknown(String::new(), "")
}

// ---- speed history ----

#[derive(Serialize)]
pub struct SpeedTest {
    pub ts: i64,
    pub network: String,
    pub down_mbps: f64,
    pub latency_ms: f64,
}

pub fn speed_history(db: &Db) -> Vec<SpeedTest> {
    let conn = db.conn.lock().unwrap();
    conn.prepare("SELECT ts, network, down_mbps, latency_ms FROM speed_tests ORDER BY ts DESC LIMIT 60")
        .and_then(|mut s| {
            s.query_map([], |r| Ok(SpeedTest { ts: r.get(0)?, network: r.get(1)?, down_mbps: r.get(2)?, latency_ms: r.get(3)? }))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default()
}

fn active_network() -> String {
    run_lossy(Command::new("nmcli").args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]))
        .lines()
        .find(|l| l.ends_with("wireless") || l.ends_with("ethernet"))
        .and_then(|l| l.rsplit_once(':'))
        .map(|(name, _)| name.replace("\\:", ":"))
        .unwrap_or_else(|| "unknown".into())
}

/// Downloads 25 MB from a content network and records the speed. This is
/// the only thing in Trace that deliberately spends your data.
pub fn speed_test(db: &Db, now: i64) -> Result<Vec<SpeedTest>, String> {
    let out = run(Command::new("curl").args([
        "-s", "-o", "/dev/null", "--max-time", "20", "-w", "%{speed_download} %{time_connect} %{size_download}",
        "https://speed.cloudflare.com/__down?bytes=25000000",
    ]))
    .map_err(|_| "Couldn't reach the test server — is the internet up?".to_string())?;
    let f: Vec<f64> = out.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    let (Some(bytes_per_sec), Some(connect), Some(size)) = (f.first(), f.get(1), f.get(2)) else {
        return Err("The test gave no result".into());
    };
    if *size < 1_000_000.0 {
        return Err("The download stopped too early to measure".into());
    }
    db.conn
        .lock()
        .unwrap()
        .execute("INSERT INTO speed_tests VALUES (?1, ?2, ?3, ?4)", params![now, active_network(), bytes_per_sec * 8.0 / 1e6, connect * 1000.0])
        .ok();
    Ok(speed_history(db))
}

// ---- Wi-Fi channels and sharing ----

#[derive(Serialize)]
pub struct Channel {
    pub channel: u32,
    pub networks: u32,
    pub strongest: u32,
    pub yours: bool,
}

#[derive(Serialize)]
pub struct WifiAir {
    pub signal: Option<u32>,
    pub channels: Vec<Channel>,
    pub advice: String,
}

/// How crowded each Wi-Fi channel around you is, from the last scan.
pub fn wifi_air(rescan: bool) -> WifiAir {
    let out = run_lossy(Command::new("nmcli").args(["-t", "-f", "IN-USE,CHAN,SIGNAL", "device", "wifi", "list", "--rescan", if rescan { "yes" } else { "no" }]));
    let mut channels: BTreeMap<u32, Channel> = BTreeMap::new();
    let mut signal = None;
    for line in out.lines() {
        let f: Vec<&str> = line.split(':').collect();
        let (Some(chan), Some(strength)) = (f.get(1).and_then(|c| c.parse::<u32>().ok()), f.get(2).and_then(|s| s.parse::<u32>().ok())) else {
            continue;
        };
        let entry = channels.entry(chan).or_insert(Channel { channel: chan, networks: 0, strongest: 0, yours: false });
        entry.networks += 1;
        entry.strongest = entry.strongest.max(strength);
        if f[0] == "*" {
            entry.yours = true;
            signal = Some(strength);
        }
    }
    let channels: Vec<Channel> = channels.into_values().collect();
    let advice = match channels.iter().find(|c| c.yours) {
        None => String::new(),
        Some(mine) if mine.channel > 14 => "You're on the 5 GHz band, which has room for everyone; channel crowding rarely matters here.".into(),
        Some(mine) => {
            // On 2.4 GHz only 1, 6 and 11 don't overlap each other.
            let load = |c: u32| channels.iter().filter(|x| x.channel <= 14 && x.channel.abs_diff(c) <= 2).map(|x| x.networks).sum::<u32>();
            let best = [1u32, 6, 11].into_iter().min_by_key(|c| load(*c)).unwrap_or(mine.channel);
            if load(best) + 2 < load(mine.channel) {
                format!("Your channel ({}) is shared with about {} networks. Channel {best} has about {} — changing it in the router's settings should help.", mine.channel, load(mine.channel), load(best))
            } else {
                format!("Your channel ({}) is as good as any other here.", mine.channel)
            }
        }
    };
    WifiAir { signal, channels, advice }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn qr_escape(s: &str) -> String {
    s.chars().flat_map(|c| if matches!(c, '\\' | ';' | ',' | ':' | '"') { vec!['\\', c] } else { vec![c] }).collect()
}

/// An SVG QR code a phone camera can scan to join the network.
pub fn wifi_qr(uuid: &str) -> Result<String, String> {
    if !is_uuid(uuid) {
        return Err("Not a saved network".into());
    }
    if !has_tool("qrencode") {
        return Err("Install qrencode to make QR codes".into());
    }
    let get = |field: &str| run(Command::new("nmcli").args(["-s", "-g", field, "connection", "show", "uuid", uuid]));
    let ssid = get("802-11-wireless.ssid")?;
    let password = get("802-11-wireless-security.psk").unwrap_or_default();
    let payload = if password.is_empty() {
        format!("WIFI:T:nopass;S:{};;", qr_escape(&ssid))
    } else {
        format!("WIFI:T:WPA;S:{};P:{};;", qr_escape(&ssid), qr_escape(&password))
    };
    run(Command::new("qrencode").args(["-t", "SVG", "-m", "2", "-o", "-", &payload]))
}

// ---- drive wear over time ----

#[derive(Serialize)]
pub struct WearPoint {
    pub ts: i64,
    pub device: String,
    pub wear_percent: Option<i64>,
    pub power_on_hours: Option<i64>,
}

/// Keeps one reading per drive per day, each time drive health is checked.
pub fn record_wear(db: &Db, now: i64, health: &[crate::health::DriveHealth]) {
    let conn = db.conn.lock().unwrap();
    for h in health {
        conn.execute("DELETE FROM drive_wear WHERE device = ?1 AND ts > ?2", params![h.device, now - 86_400]).ok();
        conn.execute("INSERT INTO drive_wear VALUES (?1, ?2, ?3, ?4)", params![now, h.device, h.wear_percent, h.power_on_hours]).ok();
    }
}

pub fn wear_history(db: &Db) -> Vec<WearPoint> {
    let conn = db.conn.lock().unwrap();
    conn.prepare("SELECT ts, device, wear_percent, power_on_hours FROM drive_wear ORDER BY ts")
        .and_then(|mut s| {
            s.query_map([], |r| Ok(WearPoint { ts: r.get(0)?, device: r.get(1)?, wear_percent: r.get(2)?, power_on_hours: r.get(3)? }))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_idle_power_and_cost_per_percent() {
        // 6 W idle plus 0.3 W per percent of processor.
        let samples: Vec<(f64, f64)> = (0..60).map(|i| (i as f64, 6.0 + 0.3 * i as f64)).collect();
        let (idle, slope) = fit_power(&samples).unwrap();
        assert!((idle - 6.0).abs() < 0.01 && (slope - 0.3).abs() < 0.001);
        assert!(fit_power(&samples[..10]).is_none(), "too little history");
        let flat: Vec<(f64, f64)> = (0..60).map(|_| (5.0, 8.0)).collect();
        assert!(fit_power(&flat).is_none(), "load that never varies says nothing");
    }

    #[test]
    fn growth_lists_the_folder_that_actually_grew() {
        let gb = 1024 * 1024 * 1024u64;
        let old: BTreeMap<String, u64> = [("/h".to_string(), 10 * gb), ("/h/a".to_string(), 4 * gb), ("/h/b".to_string(), 6 * gb)].into();
        let new: BTreeMap<String, u64> = [("/h".to_string(), 13 * gb), ("/h/a".to_string(), 7 * gb), ("/h/b".to_string(), 6 * gb), ("/h/c".to_string(), gb / 20)].into();
        let grown = pick_growth(&old, &new);
        assert_eq!(grown.len(), 1, "the parent is explained by its child, the others didn't change");
        assert_eq!(grown[0].path, "/h/a");
        assert_eq!(grown[0].now - grown[0].before, 3 * gb);
    }

    #[test]
    fn known_names_are_explained_and_odd_input_is_harmless() {
        assert_eq!(what_is("NetworkManager.service").safe_to_stop, "careful");
        assert_eq!(what_is("kworker/3:1-events").safe_to_stop, "no");
        assert_eq!(what_is("irq/45-ASUE").source, "Trace's reference");
        assert!(what_is("$(rm -rf ~)").what.is_empty());
    }

    #[test]
    fn qr_payload_escapes_what_the_format_reserves() {
        assert_eq!(qr_escape(r#"my;net:"x""#), r#"my\;net\:\"x\""#);
        assert!(wifi_qr("not-a-uuid").is_err());
    }

    #[test]
    fn local_addresses_are_not_reported_as_connections() {
        assert!(is_local_address("127.0.0.1") && is_local_address("::1") && is_local_address("127.0.0.53"));
        assert!(!is_local_address("192.168.1.10") && !is_local_address("104.16.1.1"));
    }
}
