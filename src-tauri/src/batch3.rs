//! Security check-up, microphone/camera use, saved Wi-Fi networks, privacy
//! cleaning, restricted launching, CPU stress test, focus sessions,
//! settings export/import and leak detection.

use crate::control_process::ActionResult;
use crate::db::Db;
use crate::sys_util::{data_dir, err, ok, read_trimmed, run, run_lossy};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;

// ---- security check-up ----

#[derive(Serialize)]
pub struct Finding {
    /// "good" | "warn" | "bad" | "info"
    pub level: String,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct SecurityReport {
    pub findings: Vec<Finding>,
    pub failed_logins: Vec<String>,
}

fn unit_active(unit: &str) -> bool {
    run_lossy(Command::new("systemctl").args(["is-active", unit])) == "active"
}

/// Read-only look at the things that most often leave a laptop exposed.
pub fn security_report() -> SecurityReport {
    let mut findings = Vec::new();
    let mut add = |level: &str, title: &str, detail: String| {
        findings.push(Finding { level: level.into(), title: title.into(), detail });
    };

    let firewall = unit_active("firewalld") || unit_active("ufw") || unit_active("nftables");
    let exposed: Vec<String> = {
        let mut ports: Vec<String> = crate::more::listeners()
            .into_iter()
            .filter(|l| l.exposed && l.protocol == "TCP")
            .map(|l| format!("{} ({})", l.port, l.process))
            .collect();
        ports.dedup();
        ports
    };
    if firewall {
        add("good", "Firewall is running", "Incoming connections are blocked unless you opened the port.".into());
    } else {
        add("bad", "No firewall is running", "Every listening program is reachable from any network you join. Turn it on under Tweaks → Background Services.".into());
    }
    if !exposed.is_empty() {
        add(
            if firewall { "info" } else { "warn" },
            &format!("{} program(s) listen on the network", exposed.len()),
            format!("{}. {}", exposed.join(", "), if firewall { "The firewall decides which are actually reachable." } else { "With no firewall, all of them are reachable." }),
        );
    }

    if unit_active("sshd") {
        let config = run_lossy(Command::new("sh").args(["-c", "cat /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf 2>/dev/null"]));
        let setting = |key: &str| {
            config
                .lines()
                .map(str::trim)
                .filter(|l| !l.starts_with('#'))
                .find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_lowercase()))
        };
        let passwords = setting("PasswordAuthentication").unwrap_or_else(|| "yes".into());
        let root = setting("PermitRootLogin").unwrap_or_else(|| "prohibit-password".into());
        if passwords != "no" {
            add("warn", "SSH accepts passwords", "Anyone who can reach this machine can guess at your password. Key-only login (PasswordAuthentication no) is far safer.".into());
        } else {
            add("good", "SSH is key-only", "Password guessing can't get in.".into());
        }
        if root == "yes" {
            add("bad", "SSH lets root log in with a password", "Set PermitRootLogin to prohibit-password or no.".into());
        }
    } else {
        add("good", "No SSH server running", "Nothing accepts remote logins.".into());
    }

    let encrypted = run_lossy(Command::new("lsblk").args(["-rno", "TYPE"])).lines().any(|t| t == "crypt");
    if encrypted {
        add("good", "Disk is encrypted", "A lost or stolen laptop doesn't give up your files.".into());
    } else {
        add("warn", "Disk is not encrypted", "Anyone with the laptop in hand can read your files by booting another system. Encrypting needs a reinstall or a careful migration, so this is information rather than a button.".into());
    }

    // The last byte of the SecureBoot EFI variable is 1 when it is enforced.
    let secure_boot = fs::read_dir("/sys/firmware/efi/efivars")
        .ok()
        .and_then(|d| d.flatten().find(|e| e.file_name().to_string_lossy().starts_with("SecureBoot-")))
        .and_then(|e| fs::read(e.path()).ok())
        .and_then(|bytes| bytes.last().copied());
    match secure_boot {
        Some(1) => add("good", "Secure Boot is on", "The firmware only starts signed boot software.".into()),
        Some(_) => add("info", "Secure Boot is off", "Common on Linux laptops; it mainly matters against someone tampering with the boot files.".into()),
        None => {}
    }

    if crate::sys_util::has_tool("arch-audit") {
        let vulnerable = run_lossy(Command::new("arch-audit").arg("-u"));
        let count = vulnerable.lines().filter(|l| !l.trim().is_empty()).count();
        if count > 0 {
            add("warn", &format!("{count} installed package(s) have a known vulnerability with a fix available"), "Updating the system installs the fixes.".into());
        } else {
            add("good", "No known-vulnerable packages with a fix waiting", String::new());
        }
    }

    let failed: Vec<String> = crate::batch4::failed_logins().into_iter().rev().take(20).map(|(when, what)| format!("{when} — {}", what.to_lowercase())).collect();
    if failed.len() >= 10 {
        add("warn", "Many failed logins this week", "Listed below. A handful of your own mistyped passwords is normal; a long run from sshd is someone guessing.".into());
    }

    SecurityReport { findings, failed_logins: failed }
}

// ---- microphone and camera ----

#[derive(Serialize)]
pub struct MediaUse {
    pub microphone: Vec<String>,
    pub camera: Vec<String>,
}

/// Which programs are recording from a microphone or holding a camera open, right now.
pub fn media_use() -> MediaUse {
    let mut microphone = Vec::new();
    let out = run_lossy(Command::new("pactl").args(["-f", "json", "list", "source-outputs"]));
    if let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&out) {
        for item in list {
            let props = &item["properties"];
            // Volume meters open a "peak detect" stream; that isn't listening in.
            if props["application.id"].as_str() == Some("org.PulseAudio.pavucontrol") || props["media.name"].as_str() == Some("Peak detect") {
                continue;
            }
            let name = props["application.name"].as_str().or(props["application.process.binary"].as_str()).unwrap_or("an app");
            if !microphone.iter().any(|m| m == name) {
                microphone.push(name.to_string());
            }
        }
    }
    let mut camera = Vec::new();
    if let Ok(procs) = fs::read_dir("/proc") {
        for entry in procs.flatten() {
            let pid = entry.file_name().to_string_lossy().to_string();
            if pid.parse::<u32>().is_err() {
                continue;
            }
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            let holds = fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|t| t.to_string_lossy().starts_with("/dev/video")));
            if holds {
                if let Some(name) = crate::memory_map::app_name(&pid) {
                    if !camera.contains(&name) {
                        camera.push(name);
                    }
                }
            }
        }
    }
    MediaUse { microphone, camera }
}

// ---- saved Wi-Fi networks ----

#[derive(Serialize)]
pub struct SavedWifi {
    pub name: String,
    pub uuid: String,
    pub autoconnect: bool,
    pub last_used: i64,
    /// Marked as costing data, so apps that check hold back.
    pub metered: bool,
}

pub fn saved_wifi() -> Vec<SavedWifi> {
    let out = run_lossy(Command::new("nmcli").args(["-t", "-f", "TYPE,UUID,AUTOCONNECT,TIMESTAMP,NAME", "connection", "show"]));
    let mut list: Vec<SavedWifi> = out
        .lines()
        .filter_map(|l| {
            // The name comes last because it is the only field that can contain colons.
            let mut f = l.splitn(5, ':');
            if f.next()? != "802-11-wireless" {
                return None;
            }
            let uuid = f.next()?.to_string();
            let metered = run_lossy(Command::new("nmcli").args(["-g", "connection.metered", "connection", "show", "uuid", &uuid])) == "yes";
            Some(SavedWifi {
                metered,
                uuid,
                autoconnect: f.next()? == "yes",
                last_used: f.next()?.parse().unwrap_or(0),
                name: f.next()?.replace("\\:", ":"),
            })
        })
        .collect();
    list.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    list
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// `action`: "forget" | "auto_on" | "auto_off" | "metered_on" | "metered_off" | "password".
pub fn wifi_action(uuid: &str, action: &str) -> ActionResult {
    if !is_uuid(uuid) {
        return err("Not a saved network");
    }
    let result = match action {
        "forget" => run(Command::new("nmcli").args(["connection", "delete", "uuid", uuid])).map(|_| "Network forgotten".to_string()),
        "auto_on" | "auto_off" => run(Command::new("nmcli").args([
            "connection", "modify", "uuid", uuid, "connection.autoconnect", if action == "auto_on" { "yes" } else { "no" },
        ]))
        .map(|_| if action == "auto_on" { "Will join automatically".to_string() } else { "Won't join automatically".to_string() }),
        "metered_on" | "metered_off" => run(Command::new("nmcli").args([
            "connection", "modify", "uuid", uuid, "connection.metered", if action == "metered_on" { "yes" } else { "unknown" },
        ]))
        .map(|_| if action == "metered_on" { "Marked as limited data: Trace skips its update check here, and apps that respect the mark hold back downloads".to_string() } else { "No longer marked as limited data".to_string() }),
        "password" => run(Command::new("nmcli").args(["-s", "-g", "802-11-wireless-security.psk", "connection", "show", "uuid", uuid]))
            .map(|p| if p.is_empty() { "This network has no saved password".to_string() } else { format!("Password: {p}") }),
        _ => Err("Unknown action".to_string()),
    };
    match result {
        Ok(message) => ok(message),
        Err(e) => err(e),
    }
}

// ---- privacy cleaner ----

#[derive(Serialize)]
pub struct PrivacyItem {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub bytes: u64,
}

fn du(path: &std::path::Path) -> u64 {
    run_lossy(Command::new("du").arg("-sb").arg(path)).split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

const BROWSER_CACHES: &[(&str, &str)] = &[
    ("BraveSoftware", "Brave"),
    ("chromium", "Chromium"),
    ("google-chrome", "Chrome"),
    ("mozilla", "Firefox"),
    ("zen", "Zen"),
    ("vivaldi", "Vivaldi"),
    ("microsoft-edge", "Edge"),
];

pub fn privacy_items() -> Vec<PrivacyItem> {
    let mut items = Vec::new();
    if let Some(data) = dirs::data_local_dir() {
        let recent = data.join("recently-used.xbel");
        if let Ok(meta) = fs::metadata(&recent) {
            items.push(PrivacyItem {
                id: "recent".into(),
                title: "Recently used files list".into(),
                detail: "The list file pickers show under \"Recent\". The files themselves are untouched.".into(),
                bytes: meta.len(),
            });
        }
    }
    if let Some(cache) = dirs::cache_dir() {
        for (dir, name) in BROWSER_CACHES {
            let path = cache.join(dir);
            if path.is_dir() {
                items.push(PrivacyItem {
                    id: format!("browser:{dir}"),
                    title: format!("{name} cache"),
                    detail: "Copies of pages and images kept to load sites faster. Logins, history and bookmarks live elsewhere and are kept. Close the browser first.".into(),
                    bytes: du(&path),
                });
            }
        }
    }
    if crate::sys_util::has_tool("cliphist") {
        let entries = run_lossy(Command::new("cliphist").arg("list")).lines().count();
        if entries > 0 {
            items.push(PrivacyItem {
                id: "clipboard".into(),
                title: format!("Clipboard history ({entries} entries)"),
                detail: "Everything you copied that the clipboard manager still remembers, passwords included.".into(),
                bytes: 0,
            });
        }
    }
    items
}

pub fn privacy_clean(id: &str) -> ActionResult {
    match id {
        "recent" => match dirs::data_local_dir().map(|d| fs::write(d.join("recently-used.xbel"), "")) {
            Some(Ok(())) => ok("Recent files list cleared"),
            _ => err("Couldn't clear the list"),
        },
        "clipboard" => match run(Command::new("cliphist").arg("wipe")) {
            Ok(_) => ok("Clipboard history wiped"),
            Err(e) => err(e),
        },
        other => {
            let Some(dir) = other.strip_prefix("browser:") else {
                return err("Unknown item");
            };
            let Some((_, name)) = BROWSER_CACHES.iter().find(|(d, _)| *d == dir) else {
                return err("Unknown browser");
            };
            let Some(path) = dirs::cache_dir().map(|c| c.join(dir)) else {
                return err("No cache folder");
            };
            let before = du(&path);
            match fs::remove_dir_all(&path) {
                Ok(()) => ok(format!("{name} cache cleared ({:.0} MB)", before as f64 / 1024.0 / 1024.0)),
                Err(e) => err(format!("Couldn't clear it: {e}")),
            }
        }
    }
}

// ---- launching a program with restrictions ----

/// Starts `command` as you, optionally cut off from the network and capped
/// in memory and CPU. Uses systemd for the caps and an empty network
/// namespace for "no network"; no root involved.
pub fn run_restricted(command: &str, no_network: bool, memory_mb: Option<u64>, cpu_percent: Option<u32>) -> ActionResult {
    let command = command.trim();
    if command.is_empty() {
        return err("Type the command to run");
    }
    let mut cmd = Command::new("systemd-run");
    cmd.args(["--user", "--scope", "--quiet", "--collect"]);
    if let Some(mb) = memory_mb.filter(|m| *m >= 64) {
        cmd.arg("-p").arg(format!("MemoryMax={mb}M"));
    }
    if let Some(p) = cpu_percent.filter(|p| (5..=6400).contains(p)) {
        cmd.arg("-p").arg(format!("CPUQuota={p}%"));
    }
    cmd.arg("--");
    if no_network {
        // A fresh user + network namespace: the program sees no interfaces at all.
        cmd.args(["unshare", "-rn", "--"]);
    }
    cmd.args(["sh", "-c", command]);
    use std::os::unix::process::CommandExt;
    match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(_) => ok(format!(
            "Started {command}{}{}{}",
            if no_network { " with no network" } else { "" },
            memory_mb.map(|m| format!(", at most {m} MB")).unwrap_or_default(),
            cpu_percent.map(|p| format!(", at most {p}% CPU")).unwrap_or_default(),
        )),
        Err(e) => err(format!("Couldn't start it: {e}")),
    }
}

// ---- stress test ----

#[derive(Serialize)]
pub struct StressResult {
    pub seconds: u64,
    pub start_c: f32,
    pub peak_c: f32,
    pub end_c: f32,
    /// Lowest clock speed seen under load as a share of the highest — well
    /// under 100 means the processor slowed itself to stay cool.
    pub lowest_speed_percent: f32,
    pub peak_fan_rpm: u32,
    pub verdict: String,
}

fn hottest() -> f32 {
    crate::hardware::sensors().iter().filter(|s| s.kind == "temp").map(|s| s.value as f32).fold(0.0, f32::max)
}

fn mean_mhz() -> f32 {
    let values: Vec<f32> = crate::tweaks::expand_globs(&["/sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq"])
        .iter()
        .filter_map(|f| read_trimmed(f)?.parse::<f32>().ok())
        .collect();
    if values.is_empty() { 0.0 } else { values.iter().sum::<f32>() / values.len() as f32 / 1000.0 }
}

/// Loads every core for `seconds` and watches how the cooling copes.
pub fn stress_test(seconds: u64) -> StressResult {
    let seconds = seconds.clamp(10, 120);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let workers: Vec<_> = (0..cores)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for i in 0..200_000u64 {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
                    }
                    std::hint::black_box(x);
                }
            })
        })
        .collect();
    let start_c = hottest();
    let (mut peak_c, mut peak_fan, mut top_mhz, mut low_mhz) = (start_c, 0u32, 0f32, f32::MAX);
    for second in 0..seconds {
        std::thread::sleep(std::time::Duration::from_secs(1));
        peak_c = peak_c.max(hottest());
        peak_fan = peak_fan.max(crate::tuning::list_fans().iter().map(|f| f.rpm).max().unwrap_or(0));
        let mhz = mean_mhz();
        top_mhz = top_mhz.max(mhz);
        // The first seconds are the ramp-up, not throttling.
        if second >= 5 && mhz > 0.0 {
            low_mhz = low_mhz.min(mhz);
        }
        // Stop early rather than cook the machine.
        if peak_c >= 98.0 {
            break;
        }
    }
    let end_c = hottest();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for w in workers {
        w.join().ok();
    }
    let lowest = if top_mhz > 0.0 && low_mhz < f32::MAX { low_mhz / top_mhz * 100.0 } else { 100.0 };
    let verdict = if peak_c >= 98.0 {
        "Stopped early: it reached 98°C. The cooling can't hold a full load — check the vents and fans, and consider fresh thermal paste."
    } else if peak_c >= 92.0 && lowest < 85.0 {
        "It runs very hot and slows itself down to cope. Cleaning the fans and vents would likely help."
    } else if lowest < 80.0 {
        "The processor cut its speed noticeably under sustained load. That is the power limit or the heat, depending on the temperature above."
    } else if peak_c >= 90.0 {
        "Hot but holding its speed. Normal for a thin laptop at full load, with little headroom left."
    } else {
        "The cooling holds a full load comfortably."
    };
    StressResult { seconds, start_c, peak_c, end_c, lowest_speed_percent: lowest, peak_fan_rpm: peak_fan, verdict: verdict.into() }
}

// ---- exporting and importing Trace's own settings ----

const SETTINGS_FILES: &[&str] = &["automation.json", "tweak-presets.json", "display.json"];

#[derive(Serialize, Deserialize)]
struct Bundle {
    trace_settings: u32,
    files: std::collections::BTreeMap<String, serde_json::Value>,
    profiles: Vec<crate::profiles::Profile>,
    kept_tweaks: Vec<(String, String)>,
}

pub fn export_settings(db: &Db) -> Result<String, String> {
    let mut files = std::collections::BTreeMap::new();
    for name in SETTINGS_FILES {
        if let Some(value) = fs::read_to_string(data_dir().join(name)).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            files.insert(name.to_string(), value);
        }
    }
    let kept_tweaks = {
        let conn = db.conn.lock().unwrap();
        conn.prepare("SELECT tweak_id, value FROM tweak_persist")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default()
    };
    let bundle = Bundle { trace_settings: 1, files, profiles: crate::profiles::list_profiles(), kept_tweaks };
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("No Downloads folder")?;
    let path = dir.join("trace-settings.json");
    fs::write(&path, serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Loads a bundle written by `export_settings`. Kept settings are recorded
/// but not applied — applying is left to you, on the Tweaks page.
pub fn import_settings(db: &Db, path: &str) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let bundle: Bundle = serde_json::from_str(&text).map_err(|_| "That isn't a Trace settings file".to_string())?;
    let mut restored = 0;
    for (name, value) in &bundle.files {
        if SETTINGS_FILES.contains(&name.as_str()) {
            fs::write(data_dir().join(name), serde_json::to_string_pretty(value).unwrap_or_default()).map_err(|e| e.to_string())?;
            restored += 1;
        }
    }
    for profile in &bundle.profiles {
        crate::profiles::save_profile(profile)?;
    }
    let conn = db.conn.lock().unwrap();
    for (id, value) in &bundle.kept_tweaks {
        if crate::tweaks::find(id).is_some() {
            conn.execute("INSERT OR REPLACE INTO tweak_persist (tweak_id, value) VALUES (?1, ?2)", params![id, value]).ok();
        }
    }
    Ok(format!(
        "Loaded {restored} settings file(s), {} profile(s) and {} kept setting(s). Kept settings are listed on the Tweaks page but not applied until you apply them.",
        bundle.profiles.len(),
        bundle.kept_tweaks.len()
    ))
}

// ---- focus sessions ----

/// A stretch of time during which chosen apps are frozen. Kept on disk so a
/// crash or a reboot can't leave them frozen for good.
#[derive(Serialize, Deserialize, Clone)]
pub struct Focus {
    pub apps: Vec<String>,
    pub until: i64,
}

fn focus_path() -> std::path::PathBuf {
    data_dir().join("focus.json")
}

pub fn focus_current() -> Option<Focus> {
    serde_json::from_str(&fs::read_to_string(focus_path()).ok()?).ok()
}

pub fn focus_start(apps: Vec<String>, minutes: u32, now: i64) -> ActionResult {
    if apps.is_empty() || !(1..=480).contains(&minutes) {
        return err("Pick at least one app and 1–480 minutes");
    }
    focus_end();
    let frozen: Vec<String> = apps.into_iter().filter(|app| crate::memory_map::app_action(app, "freeze").ok).collect();
    if frozen.is_empty() {
        return err("None of those apps could be frozen (not running, or part of your session)");
    }
    let focus = Focus { apps: frozen.clone(), until: now + minutes as i64 * 60 };
    fs::write(focus_path(), serde_json::to_string(&focus).unwrap_or_default()).ok();
    ok(format!("Focus on for {minutes} min: {} frozen", frozen.join(", ")))
}

/// Wakes the frozen apps. Returns their names if a session was running.
pub fn focus_end() -> Option<Vec<String>> {
    let focus = focus_current()?;
    for app in &focus.apps {
        crate::memory_map::app_action(app, "resume");
    }
    fs::remove_file(focus_path()).ok();
    Some(focus.apps)
}

/// Ends the session once its time is up. Called from the background loop.
pub fn focus_tick(now: i64) {
    if focus_current().is_some_and(|f| f.until <= now) {
        if let Some(apps) = focus_end() {
            crate::sys_util::notify("Focus time is over", &format!("{} can run again.", apps.join(", ")), false);
        }
    }
}

// ---- leak detection ----

#[derive(Serialize)]
pub struct Leak {
    pub app: String,
    pub hours: f64,
    pub start_bytes: u64,
    pub end_bytes: u64,
    pub per_hour_bytes: f64,
}

/// True when a series climbs steadily: it ends well above where it began and
/// almost never steps down along the way (a busy app goes up *and* down).
fn climbs_steadily(values: &[f64]) -> bool {
    if values.len() < 8 {
        return false;
    }
    let (first, last) = (values[0], values[values.len() - 1]);
    let rises = values.windows(2).filter(|w| w[1] >= w[0] * 0.98).count();
    last > first * 1.3 && last - first > 200.0 * 1024.0 * 1024.0 && rises as f64 >= (values.len() - 1) as f64 * 0.8
}

/// Apps whose memory has climbed steadily over the last day.
pub fn leaks(db: &Db, now: i64) -> Vec<Leak> {
    let points = db.app_memory_history(now - 86_400, 1800);
    let mut by_app: std::collections::BTreeMap<String, Vec<(i64, f64)>> = std::collections::BTreeMap::new();
    for p in points {
        by_app.entry(p.app).or_default().push((p.ts, (p.ram_bytes + p.swap_bytes) as f64));
    }
    let mut found: Vec<Leak> = by_app
        .into_iter()
        .filter_map(|(app, series)| {
            let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
            if !climbs_steadily(&values) {
                return None;
            }
            let hours = (series[series.len() - 1].0 - series[0].0) as f64 / 3600.0;
            (hours >= 3.0).then(|| Leak {
                app,
                hours,
                start_bytes: values[0] as u64,
                end_bytes: values[values.len() - 1] as u64,
                per_hour_bytes: (values[values.len() - 1] - values[0]) / hours,
            })
        })
        .collect();
    found.sort_by(|a, b| b.per_hour_bytes.total_cmp(&a.per_hour_bytes));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_steady_climb_from_ordinary_ups_and_downs() {
        let mb = 1024.0 * 1024.0;
        let leak: Vec<f64> = (0..12).map(|i| (500.0 + i as f64 * 60.0) * mb).collect();
        assert!(climbs_steadily(&leak));
        let busy: Vec<f64> = (0..12).map(|i| (800.0 + if i % 2 == 0 { 400.0 } else { -300.0 }) * mb).collect();
        assert!(!climbs_steadily(&busy));
        let small: Vec<f64> = (0..12).map(|i| (50.0 + i as f64 * 2.0) * mb).collect();
        assert!(!climbs_steadily(&small), "tiny growth isn't worth flagging");
        assert!(!climbs_steadily(&leak[..4]), "too few samples");
    }

    #[test]
    fn settings_round_trip_through_a_bundle() {
        let db = Db { conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        crate::tweaks::init(&db);
        db.conn.lock().unwrap().execute("INSERT INTO tweak_persist VALUES ('vm.swappiness', '100')", []).unwrap();
        let bundle = Bundle {
            trace_settings: 1,
            files: Default::default(),
            profiles: Vec::new(),
            kept_tweaks: vec![("vm.swappiness".into(), "120".into()), ("not.a.setting".into(), "1".into())],
        };
        let path = std::env::temp_dir().join(format!("trace-bundle-{}.json", std::process::id()));
        fs::write(&path, serde_json::to_string(&bundle).unwrap()).unwrap();
        let message = import_settings(&db, &path.to_string_lossy()).unwrap();
        assert!(message.contains("2 kept"));
        let value: String = db.conn.lock().unwrap().query_row("SELECT value FROM tweak_persist WHERE tweak_id = 'vm.swappiness'", [], |r| r.get(0)).unwrap();
        assert_eq!(value, "120");
        let unknown: i64 = db.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM tweak_persist WHERE tweak_id = 'not.a.setting'", [], |r| r.get(0)).unwrap();
        assert_eq!(unknown, 0, "unknown settings are ignored");
        assert!(import_settings(&db, "/etc/hostname").is_err());
        fs::remove_file(path).ok();
    }

    #[test]
    fn restricted_program_really_has_no_network() {
        // `unshare -rn` gives a namespace with only a down loopback: no route out.
        let out = Command::new("unshare").args(["-rn", "--", "sh", "-c", "tail -n +3 /proc/net/dev | cut -d: -f1 | tr -d ' ' | tr '\\n' ' '"]).output();
        let Ok(out) = out else { return };
        if !out.status.success() {
            return; // unprivileged namespaces disabled here
        }
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "lo");
    }
}
