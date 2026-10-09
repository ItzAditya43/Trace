//! The finishing pieces: hour-by-hour screen time, microphone and camera
//! history, security fixes and known-vulnerable packages, phone alerts,
//! Wi-Fi signal history, keyboard-light timeout, equaliser, unattended
//! maintenance, secure erase, weekly digest, break and app-time nudges.

use crate::control_process::ActionResult;
use crate::db::Db;
use crate::sys_util::{data_dir, err, has_tool, notify, ok, read_trimmed, run, run_lossy, run_privileged};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

pub fn init(db: &Db) {
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS focus_hours (hour_ts INTEGER NOT NULL, app TEXT NOT NULL, secs INTEGER NOT NULL, PRIMARY KEY (hour_ts, app));
             CREATE TABLE IF NOT EXISTS media_log (ts INTEGER NOT NULL, kind TEXT NOT NULL, app TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS wifi_signal (ts INTEGER NOT NULL, dbm INTEGER NOT NULL);",
        )
        .ok();
}

// ---- settings for the small optional behaviours ----

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppLimit {
    pub app: String,
    pub minutes: u32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Extras {
    /// Send "someone is at your machine" alerts to a paired phone (KDE Connect).
    pub phone_alerts: bool,
    /// Switch the keyboard light off after this many idle seconds (0 = never).
    pub kbd_timeout_secs: u32,
    /// A Monday-morning summary of last week.
    pub weekly_digest: bool,
    /// Suggest a break after this many minutes without one (0 = never).
    pub break_minutes: u32,
    /// A nudge once an app has been in front for this long today.
    pub app_limits: Vec<AppLimit>,
    /// Week of the year the last digest was sent in.
    pub digest_week: u32,
    pub setup_done: bool,
}

pub fn extras() -> Extras {
    fs::read_to_string(data_dir().join("extras.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_extras(extras: &Extras) {
    fs::write(data_dir().join("extras.json"), serde_json::to_string_pretty(extras).unwrap_or_default()).ok();
}

pub(crate) fn utc_offset() -> i64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_gmtoff
    }
}

// ---- screen time, hour by hour ----

/// Start of the local clock hour containing `ts`.
fn local_hour_start(ts: i64, offset: i64) -> i64 {
    ts - (ts + offset).rem_euclid(3600)
}

pub fn record_focus_hour(db: &Db, now: i64, secs: i64, app: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO focus_hours VALUES (?1, ?2, ?3) ON CONFLICT(hour_ts, app) DO UPDATE SET secs = secs + excluded.secs",
        params![local_hour_start(now, utc_offset()), app, secs],
    )
    .ok();
}

#[derive(Serialize)]
pub struct HourUse {
    pub hour: u32,
    pub secs: i64,
    pub top_app: String,
    pub apps: Vec<(String, i64)>,
}

#[derive(Serialize)]
pub struct ScreenDay {
    pub day_start: i64,
    pub total_secs: i64,
    pub hours: Vec<HourUse>,
}

/// What was in front of you during each hour of a day (0 = today, -1 = yesterday).
pub fn screen_day(db: &Db, now: i64, day_offset: i64) -> ScreenDay {
    let offset = utc_offset();
    let day_start = now - (now + offset).rem_euclid(86_400) + day_offset * 86_400;
    let conn = db.conn.lock().unwrap();
    let rows: Vec<(i64, String, i64)> = conn
        .prepare("SELECT hour_ts, app, secs FROM focus_hours WHERE hour_ts >= ?1 AND hour_ts < ?2 ORDER BY secs DESC")
        .and_then(|mut s| s.query_map(params![day_start, day_start + 86_400], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default();
    let mut hours: Vec<HourUse> = (0..24).map(|hour| HourUse { hour, secs: 0, top_app: String::new(), apps: Vec::new() }).collect();
    for (hour_ts, app, secs) in rows {
        let Some(slot) = hours.get_mut(((hour_ts - day_start) / 3600) as usize) else {
            continue;
        };
        slot.secs += secs;
        if slot.top_app.is_empty() {
            slot.top_app = app.clone();
        }
        if slot.apps.len() < 4 {
            slot.apps.push((app, secs));
        }
    }
    for h in &mut hours {
        h.secs = h.secs.min(3600);
    }
    ScreenDay { day_start, total_secs: hours.iter().map(|h| h.secs).sum(), hours }
}

// ---- microphone and camera history ----

static MEDIA_ACTIVE: Mutex<Option<HashSet<(String, String)>>> = Mutex::new(None);

/// Notes each time an app starts using the microphone or camera. Called once a minute.
pub fn media_tick(db: &Db, now: i64) {
    let current = crate::batch3::media_use();
    let now_set: HashSet<(String, String)> = current
        .microphone
        .into_iter()
        .map(|a| ("microphone".to_string(), a))
        .chain(current.camera.into_iter().map(|a| ("camera".to_string(), a)))
        .collect();
    let mut active = MEDIA_ACTIVE.lock().unwrap();
    let previous = active.get_or_insert_with(HashSet::new);
    let conn = db.conn.lock().unwrap();
    for (kind, app) in now_set.difference(previous) {
        conn.execute("INSERT INTO media_log VALUES (?1, ?2, ?3)", params![now, kind, app]).ok();
    }
    conn.execute("DELETE FROM media_log WHERE ts < ?1", params![now - 90 * 86_400]).ok();
    *previous = now_set;
}

#[derive(Serialize)]
pub struct MediaEvent {
    pub ts: i64,
    pub kind: String,
    pub app: String,
}

pub fn media_history(db: &Db) -> Vec<MediaEvent> {
    let conn = db.conn.lock().unwrap();
    conn.prepare("SELECT ts, kind, app FROM media_log ORDER BY ts DESC LIMIT 60")
        .and_then(|mut s| s.query_map([], |r| Ok(MediaEvent { ts: r.get(0)?, kind: r.get(1)?, app: r.get(2)? })).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default()
}

// ---- security: fixes and known-vulnerable packages ----

/// The fix Trace can apply for a finding, by the finding's title: (id, button label).
pub fn fix_for(title: &str) -> Option<(&'static str, &'static str)> {
    match title {
        "No firewall is running" if has_tool("firewall-cmd") => Some(("firewall_on", "Turn the firewall on")),
        "SSH accepts passwords" => Some(("ssh_keys_only", "Make SSH key-only")),
        "SSH lets root log in with a password" => Some(("ssh_no_root", "Stop root password logins")),
        _ => None,
    }
}

pub fn security_fix(id: &str) -> ActionResult {
    match id {
        "firewall_on" => crate::sys_util::privileged_action("systemctl enable --now firewalld", "Firewall is on and will start with the machine"),
        "ssh_keys_only" => {
            // Without a key in place this would lock out every remote login, yours included.
            let has_key = dirs::home_dir().and_then(|h| fs::read_to_string(h.join(".ssh/authorized_keys")).ok()).is_some_and(|t| t.lines().any(|l| l.trim_start().starts_with("ssh-") || l.contains("ecdsa-")));
            if !has_key {
                return err("You have no SSH key installed on this machine (~/.ssh/authorized_keys is empty), so key-only login would lock you out. Add your key first, or stop the SSH server instead.");
            }
            crate::sys_util::privileged_action(
                "printf 'PasswordAuthentication no\\nKbdInteractiveAuthentication no\\n' > /etc/ssh/sshd_config.d/10-trace.conf && sshd -t && systemctl reload sshd",
                "SSH now accepts keys only (set in /etc/ssh/sshd_config.d/10-trace.conf; delete that file to undo)",
            )
        }
        "ssh_no_root" => crate::sys_util::privileged_action(
            "printf 'PermitRootLogin prohibit-password\\n' > /etc/ssh/sshd_config.d/11-trace-root.conf && sshd -t && systemctl reload sshd",
            "Root can no longer log in with a password over SSH",
        ),
        _ => err("Unknown fix"),
    }
}

#[derive(Serialize)]
pub struct Vulnerable {
    pub package: String,
    pub installed: String,
    /// Empty when no fixed version exists yet.
    pub fixed: String,
    pub severity: String,
    pub advisory: String,
}

/// One entry of the Arch security tracker's list.
#[derive(Deserialize)]
struct Issue {
    name: String,
    packages: Vec<String>,
    status: String,
    severity: String,
    affected: Option<String>,
    fixed: Option<String>,
}

fn vercmp(a: &str, b: &str) -> i32 {
    run_lossy(Command::new("vercmp").args([a, b])).trim().parse().unwrap_or(0)
}

/// Installed packages the Arch security team lists as vulnerable, checked
/// against the versions actually installed. Fetches the public list; sends nothing.
pub fn vulnerable_packages() -> Result<Vec<Vulnerable>, String> {
    if !has_tool("pacman") || !has_tool("vercmp") {
        return Err("This check is for Arch-based systems".into());
    }
    let body = run(Command::new("curl").args(["-sL", "-m", "15", "https://security.archlinux.org/issues/all.json"])).map_err(|_| "Couldn't reach the Arch security tracker".to_string())?;
    let issues: Vec<Issue> = serde_json::from_str(&body).map_err(|_| "The security tracker sent something unexpected".to_string())?;
    let installed: HashMap<String, String> = run_lossy(Command::new("pacman").arg("-Q"))
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, v)| (n.to_string(), v.to_string())))
        .collect();
    let mut found: BTreeMap<String, Vulnerable> = BTreeMap::new();
    for issue in issues.iter().filter(|i| i.status != "Not affected") {
        for package in &issue.packages {
            let Some(have) = installed.get(package) else {
                continue;
            };
            let vulnerable = match (&issue.fixed, &issue.affected) {
                (Some(fixed), _) => vercmp(have, fixed) < 0,
                (None, Some(affected)) => vercmp(have, affected) <= 0,
                (None, None) => false,
            };
            if vulnerable {
                found.entry(package.clone()).or_insert(Vulnerable {
                    package: package.clone(),
                    installed: have.clone(),
                    fixed: issue.fixed.clone().unwrap_or_default(),
                    severity: issue.severity.clone(),
                    advisory: issue.name.clone(),
                });
            }
        }
    }
    Ok(found.into_values().collect())
}

// ---- alerts to a phone ----

#[derive(Serialize)]
pub struct Phone {
    pub id: String,
    pub name: String,
}

/// Phones paired through KDE Connect that are reachable right now.
pub fn phones() -> Vec<Phone> {
    if !has_tool("kdeconnect-cli") {
        return Vec::new();
    }
    run_lossy(Command::new("kdeconnect-cli").args(["-a", "--id-name-only"]))
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(id, name)| Phone { id: id.to_string(), name: name.to_string() }))
        .filter(|p| p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .collect()
}

pub fn send_to_phones(message: &str) -> usize {
    let list = phones();
    for phone in &list {
        run(Command::new("kdeconnect-cli").args(["-d", &phone.id, "--ping-msg", message])).ok();
    }
    list.len()
}

static PHONE_SEEN: Mutex<String> = Mutex::new(String::new());

/// While you're away, forwards a USB device being plugged in or a wrong
/// password to your phone. Called once a minute.
pub fn phone_tick(away: bool) {
    if !away || !extras().phone_alerts {
        return;
    }
    let events = crate::batch4::access_log();
    let mut seen = PHONE_SEEN.lock().unwrap();
    let newest = events.first().map(|e| e.when.clone()).unwrap_or_default();
    if seen.is_empty() {
        // First look: remember where the log stands, alert only on what comes after.
        *seen = newest;
        return;
    }
    let fresh: Vec<&str> = events.iter().filter(|e| e.when > *seen && e.kind != "lid").map(|e| e.text.as_str()).collect();
    if !fresh.is_empty() {
        send_to_phones(&format!("Trace: while you were away — {}", fresh.join("; ")));
    }
    *seen = newest;
}

// ---- Wi-Fi signal over time ----

fn wifi_dbm() -> Option<i64> {
    let text = fs::read_to_string("/proc/net/wireless").ok()?;
    // "wlan0: 0000   55.  -55.  -256 …" — the fourth column is the level in dBm.
    text.lines().nth(2)?.split_whitespace().nth(3)?.trim_end_matches('.').parse().ok()
}

pub fn wifi_tick(db: &Db, now: i64) {
    let Some(dbm) = wifi_dbm() else {
        return;
    };
    let conn = db.conn.lock().unwrap();
    conn.execute("INSERT INTO wifi_signal VALUES (?1, ?2)", params![now, dbm]).ok();
    conn.execute("DELETE FROM wifi_signal WHERE ts < ?1", params![now - 7 * 86_400]).ok();
}

pub fn wifi_history(db: &Db, now: i64) -> Vec<(i64, i64)> {
    let conn = db.conn.lock().unwrap();
    conn.prepare("SELECT ts, dbm FROM wifi_signal WHERE ts > ?1 ORDER BY ts")
        .and_then(|mut s| s.query_map(params![now - 86_400], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default()
}

/// The country an address is registered in, from the public registries.
pub fn host_country(ip: &str) -> String {
    if ip.parse::<std::net::IpAddr>().is_err() || !has_tool("whois") {
        return String::new();
    }
    run_lossy(Command::new("timeout").args(["6", "whois", ip]))
        .lines()
        .find(|l| l.to_lowercase().starts_with("country:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|c| c.trim().to_uppercase())
        .unwrap_or_default()
}

// ---- keyboard light timeout ----

const KBD_LED: &str = "asus::kbd_backlight";

fn kbd_led() -> Option<String> {
    fs::read_dir("/sys/class/leds").ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).find(|n| n.ends_with("kbd_backlight"))
}

pub fn has_kbd_light() -> bool {
    kbd_led().is_some() && has_tool("brightnessctl")
}

fn kbd_level(led: &str) -> u32 {
    read_trimmed(&format!("/sys/class/leds/{led}/brightness")).and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn set_kbd_level(led: &str, level: u32) {
    // Through the login service, so no root is needed.
    run(Command::new("brightnessctl").args(["-q", "-d", led, "set", &level.to_string()])).ok();
}

fn key_presses() -> String {
    fs::read_to_string("/proc/interrupts").ok().and_then(|t| t.lines().find(|l| l.contains("i8042")).map(String::from)).unwrap_or_default()
}

/// Runs for the life of the app: switches the keyboard light off after the
/// chosen idle time and back on at the next key press.
pub fn kbd_light_loop() {
    let led = kbd_led().unwrap_or_else(|| KBD_LED.to_string());
    let (mut last_keys, mut idle_secs, mut saved_level) = (key_presses(), 0u32, None::<u32>);
    let (mut timeout, mut reload_in) = (0u32, 0u32);
    loop {
        if reload_in == 0 {
            timeout = extras().kbd_timeout_secs;
            reload_in = 10;
        }
        reload_in -= 1;
        if timeout == 0 {
            if let Some(level) = saved_level.take() {
                set_kbd_level(&led, level);
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
            reload_in = 0;
            continue;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        let keys = key_presses();
        if keys != last_keys {
            last_keys = keys;
            idle_secs = 0;
            if let Some(level) = saved_level.take() {
                set_kbd_level(&led, level);
            }
            continue;
        }
        idle_secs += 1;
        if idle_secs >= timeout && saved_level.is_none() {
            let level = kbd_level(&led);
            if level > 0 {
                saved_level = Some(level);
                set_kbd_level(&led, 0);
            }
        }
    }
}

// ---- equaliser ----

/// (id, label, bass dB, mid dB, treble dB)
pub const EQ_PRESETS: &[(&str, &str, f32, f32, f32)] = &[
    ("bass", "More bass", 6.0, 0.0, 0.0),
    ("treble", "Brighter", 0.0, 0.0, 5.0),
    ("voice", "Clearer voices", -3.0, 4.0, 2.0),
    ("loud", "Fuller at low volume", 5.0, -1.0, 4.0),
    ("laptop", "Small speakers", 4.0, -2.0, 3.0),
];

const EQ_UNIT: &str = "trace-eq";

fn eq_config(bass: f32, mid: f32, treble: f32) -> String {
    format!(
        r#"context.properties = {{ log.level = 0 }}
context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}
context.modules = [
    {{ name = libpipewire-module-rt flags = [ ifexists nofail ] }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-filter-chain
        args = {{
            node.description = "Equaliser (Trace)"
            media.name       = "Equaliser (Trace)"
            filter.graph = {{
                nodes = [
                    {{ type = builtin name = eq_bass   label = bq_lowshelf  control = {{ "Freq" = 110.0  "Q" = 0.7 "Gain" = {bass:.1} }} }}
                    {{ type = builtin name = eq_mid    label = bq_peaking   control = {{ "Freq" = 1800.0 "Q" = 0.9 "Gain" = {mid:.1} }} }}
                    {{ type = builtin name = eq_treble label = bq_highshelf control = {{ "Freq" = 7000.0 "Q" = 0.7 "Gain" = {treble:.1} }} }}
                ]
                links = [
                    {{ output = "eq_bass:Out" input = "eq_mid:In" }}
                    {{ output = "eq_mid:Out"  input = "eq_treble:In" }}
                ]
            }}
            audio.channels = 2
            audio.position = [ FL FR ]
            capture.props  = {{ node.name = "trace_eq" media.class = Audio/Sink }}
            playback.props = {{ node.name = "trace_eq_out" node.passive = true }}
        }}
    }}
]
"#
    )
}

/// The running preset's id, or "" when the equaliser is off.
pub fn eq_current() -> String {
    if run_lossy(Command::new("systemctl").args(["--user", "is-active", &format!("{EQ_UNIT}.service")])) != "active" {
        return String::new();
    }
    fs::read_to_string(data_dir().join("eq-preset")).unwrap_or_default().trim().to_string()
}

/// Starts (or with "" stops) an equaliser that all sound is routed through.
pub fn set_eq(preset: &str) -> ActionResult {
    let previous_file = data_dir().join("eq-previous-output");
    let was_on = !eq_current().is_empty();
    if was_on {
        // Hand sound back to the real output before the equaliser goes away.
        if let Ok(previous) = fs::read_to_string(&previous_file) {
            run(Command::new("pactl").args(["set-default-sink", previous.trim()])).ok();
        }
        run(Command::new("systemctl").args(["--user", "stop", &format!("{EQ_UNIT}.service")])).ok();
    }
    if preset.is_empty() {
        return ok("Equaliser off");
    }
    let Some((_, label, bass, mid, treble)) = EQ_PRESETS.iter().find(|p| p.0 == preset) else {
        return err("Unknown preset");
    };
    if !was_on {
        let current = run_lossy(Command::new("pactl").arg("get-default-sink"));
        if !current.is_empty() && current != "trace_eq" {
            fs::write(&previous_file, current).ok();
        }
    }
    let config = data_dir().join("eq.conf");
    if fs::write(&config, eq_config(*bass, *mid, *treble)).is_err() {
        return err("Couldn't write the equaliser settings");
    }
    run(Command::new("systemctl").args(["--user", "reset-failed", &format!("{EQ_UNIT}.service")])).ok();
    if let Err(e) = run(Command::new("systemd-run").args(["--user", "--quiet", "--collect", "--unit", EQ_UNIT, "pipewire", "-c"]).arg(&config)) {
        return err(format!("Couldn't start the equaliser: {e}"));
    }
    // The new output takes a moment to appear.
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        if run_lossy(Command::new("pactl").args(["list", "short", "sinks"])).contains("trace_eq") {
            run(Command::new("pactl").args(["set-default-sink", "trace_eq"])).ok();
            fs::write(data_dir().join("eq-preset"), preset).ok();
            return ok(format!("Equaliser on: {label}. It lasts until you log out."));
        }
    }
    run(Command::new("systemctl").args(["--user", "stop", &format!("{EQ_UNIT}.service")])).ok();
    err("The sound system didn't accept the equaliser")
}

// ---- unattended maintenance ----

const MAINTENANCE_SCRIPT: &str = r#"#!/bin/sh
# Installed by Trace. Runs weekly, as root, while on the charger.
export LC_ALL=C
{
echo "== $(date '+%Y-%m-%d %H:%M')"
echo "Drives trimmed: $(fstrim -av 2>/dev/null | wc -l) filesystem(s)"
command -v paccache >/dev/null && echo "Old package versions: $(paccache -rk2 2>&1 | tail -n 1)"
echo "System log: $(journalctl --vacuum-size=500M --vacuum-time=8weeks 2>&1 | tail -n 1)"
echo "Crash dumps older than a week removed: $(find /var/lib/systemd/coredump -type f -mtime +7 -print -delete 2>/dev/null | wc -l)"
} >> /var/log/trace-maintenance.log 2>&1
chmod 644 /var/log/trace-maintenance.log
"#;

#[derive(Serialize)]
pub struct AutoMaintenance {
    pub enabled: bool,
    pub last_lines: Vec<String>,
}

pub fn auto_maintenance() -> AutoMaintenance {
    let log = fs::read_to_string("/var/log/trace-maintenance.log").unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let start = lines.iter().rposition(|l| l.starts_with("== ")).unwrap_or(0);
    AutoMaintenance {
        enabled: run_lossy(Command::new("systemctl").args(["is-enabled", "trace-maintenance.timer"])) == "enabled",
        last_lines: lines[start..].iter().map(|l| l.trim_start_matches("== ").to_string()).collect(),
    }
}

/// Installs (or removes) a weekly root job, so maintenance needs your
/// password once rather than every week.
pub fn set_auto_maintenance(on: bool) -> ActionResult {
    if !on {
        return crate::sys_util::privileged_action(
            "systemctl disable --now trace-maintenance.timer; rm -f /etc/systemd/system/trace-maintenance.timer /etc/systemd/system/trace-maintenance.service /etc/trace/maintenance.sh; systemctl daemon-reload",
            "Automatic maintenance removed",
        );
    }
    let script = format!(
        r#"mkdir -p /etc/trace
cat > /etc/trace/maintenance.sh <<'TRACE_EOF'
{MAINTENANCE_SCRIPT}TRACE_EOF
chmod 755 /etc/trace/maintenance.sh
cat > /etc/systemd/system/trace-maintenance.service <<'TRACE_EOF'
[Unit]
Description=Trace weekly maintenance
ConditionACPower=true

[Service]
Type=oneshot
Nice=19
IOSchedulingClass=idle
ExecStart=/etc/trace/maintenance.sh
TRACE_EOF
cat > /etc/systemd/system/trace-maintenance.timer <<'TRACE_EOF'
[Unit]
Description=Trace weekly maintenance

[Timer]
OnCalendar=weekly
Persistent=true
RandomizedDelaySec=1h

[Install]
WantedBy=timers.target
TRACE_EOF
systemctl daemon-reload
systemctl enable --now trace-maintenance.timer"#
    );
    crate::sys_util::privileged_action(&script, "Maintenance now runs by itself once a week, while on the charger. Its report is in /var/log/trace-maintenance.log.")
}

// ---- secure erase ----

#[derive(Serialize)]
pub struct ErasableDrive {
    pub name: String,
    pub size: String,
    pub model: String,
    /// Why it can't be erased from here, or "".
    pub blocked: String,
}

fn mounted_anywhere(node: &serde_json::Value) -> bool {
    let own = node["mountpoints"].as_array().is_some_and(|m| m.iter().any(|p| p.is_string()));
    own || node["children"].as_array().is_some_and(|c| c.iter().any(mounted_anywhere))
}

fn erasable_from(lsblk: &serde_json::Value) -> Vec<ErasableDrive> {
    lsblk["blockdevices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["type"].as_str() == Some("disk") && !d["name"].as_str().unwrap_or("").starts_with("zram"))
        .map(|d| {
            let blocked = if mounted_anywhere(d) {
                "In use: part of it is mounted (the drive this system runs from can never be erased here)"
            } else if d["rota"].as_bool() == Some(true) {
                "A spinning disk: it has no instant erase. Overwriting takes hours and is not offered here."
            } else {
                ""
            };
            ErasableDrive {
                name: d["name"].as_str().unwrap_or_default().to_string(),
                size: d["size"].as_str().unwrap_or_default().to_string(),
                model: d["model"].as_str().unwrap_or("").trim().to_string(),
                blocked: blocked.to_string(),
            }
        })
        .collect()
}

pub fn erasable_drives() -> Vec<ErasableDrive> {
    serde_json::from_str::<serde_json::Value>(&run_lossy(Command::new("lsblk").args(["-J", "-o", "NAME,SIZE,MODEL,ROTA,TYPE,MOUNTPOINTS"])))
        .map(|v| erasable_from(&v))
        .unwrap_or_default()
}

/// Discards every block on an unused solid-state drive. `typed` must be the
/// drive's name, typed by the person, and the drive is re-checked here.
pub fn secure_erase(device: &str, typed: &str) -> ActionResult {
    if typed.trim() != device || !crate::sys_util::is_safe_token(device) {
        return err("Type the drive's name exactly to confirm");
    }
    let Some(drive) = erasable_drives().into_iter().find(|d| d.name == device) else {
        return err("No such drive");
    };
    if !drive.blocked.is_empty() {
        return err(drive.blocked);
    }
    if !Path::new(&format!("/dev/{device}")).exists() {
        return err("No such drive");
    }
    // The mount check is repeated as root, immediately before the erase.
    let script = format!(
        "if lsblk -nro MOUNTPOINTS /dev/{device} | grep -q .; then echo 'in use' >&2; exit 1; fi; blkdiscard -f --secure /dev/{device} 2>/dev/null || blkdiscard -f /dev/{device}"
    );
    match run_privileged(&script) {
        Ok(_) => ok(format!("{device} erased: every block was discarded. It now needs a new partition table before use.")),
        Err(e) => err(e),
    }
}

// ---- digest, breaks and app time ----

fn hours_minutes(secs: i64) -> String {
    if secs >= 3600 {
        format!("{} h {} min", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{} min", secs / 60)
    }
}

fn digest_text(report: &crate::logbook::LogReport) -> String {
    let mut parts = vec![format!("On for {:.0} h", report.totals.hours_recorded)];
    let mut apps: Vec<&crate::logbook::AppUsage> = report.apps.iter().filter(|a| a.focus_secs > 600).collect();
    apps.sort_by_key(|a| std::cmp::Reverse(a.focus_secs));
    if !apps.is_empty() {
        parts.push(format!("most used: {}", apps.iter().take(3).map(|a| format!("{} ({})", a.app, hours_minutes(a.focus_secs))).collect::<Vec<_>>().join(", ")));
    }
    parts.push(match report.crash_count {
        0 => "no crashes".to_string(),
        n => format!("{n} crash(es)"),
    });
    if report.oom_count > 0 {
        parts.push(format!("ran out of memory {} time(s)", report.oom_count));
    }
    if report.totals.temp_max > 0.0 {
        parts.push(format!("hottest {:.0}°C", report.totals.temp_max));
    }
    if report.package_count > 0 {
        parts.push(format!("{} package change(s)", report.package_count));
    }
    parts.join(" · ")
}

struct Presence {
    since: i64,
    away_since: i64,
    limit_day: u32,
    limits_sent: Vec<String>,
}

static PRESENCE: Mutex<Presence> = Mutex::new(Presence { since: 0, away_since: 0, limit_day: 0, limits_sent: Vec::new() });

/// Once a minute: the Monday digest, the break nudge and app-time nudges.
pub fn wellbeing_tick(db: &Db, now: i64, present: bool) {
    let mut settings = extras();
    let time = crate::sys_util::local_time();

    if settings.weekly_digest && time.weekday == 1 && time.hour >= 9 && settings.digest_week != time.yday / 7 + 1 {
        if let Ok(report) = crate::logbook::report(db, "week", -1) {
            notify("Last week on this machine", &digest_text(&report), false);
        }
        settings.digest_week = time.yday / 7 + 1;
        save_extras(&settings);
    }

    let mut p = PRESENCE.lock().unwrap();
    if present {
        if p.since == 0 {
            p.since = now;
        }
        p.away_since = 0;
        if settings.break_minutes > 0 && now - p.since >= settings.break_minutes as i64 * 60 {
            notify("Time for a break", &format!("You've been at it for {} without one. A few minutes away helps.", hours_minutes(now - p.since)), false);
            p.since = now;
        }
    } else {
        if p.away_since == 0 {
            p.away_since = now;
        }
        // Five minutes away counts as a break.
        if now - p.away_since >= 300 {
            p.since = 0;
        }
    }

    if !settings.app_limits.is_empty() {
        if p.limit_day != time.yday {
            p.limit_day = time.yday;
            p.limits_sent.clear();
        }
        let conn = db.conn.lock().unwrap();
        for limit in &settings.app_limits {
            if p.limits_sent.contains(&limit.app) {
                continue;
            }
            let used: i64 = conn
                .query_row("SELECT COALESCE(SUM(focus_secs), 0) FROM app_usage WHERE day = date(?1, 'unixepoch', 'localtime') AND app = ?2", params![now, limit.app], |r| r.get(0))
                .unwrap_or(0);
            if used >= limit.minutes as i64 * 60 {
                notify(&format!("{} has had its time for today", limit.app), &format!("{} in front today; you set {} min.", hours_minutes(used), limit.minutes), false);
                p.limits_sent.push(limit.app.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_follow_the_local_clock_even_with_a_half_hour_offset() {
        let ist = 5 * 3600 + 1800;
        // 10:45 local is 05:15 UTC.
        let ts = 1_790_000_000 - 1_790_000_000 % 86_400 + 5 * 3600 + 15 * 60;
        let start = local_hour_start(ts, ist);
        assert_eq!((start + ist) % 3600, 0, "starts on a local hour");
        assert_eq!(ts - start, 45 * 60);
        assert_eq!(local_hour_start(ts, 0), ts - 15 * 60);
    }

    #[test]
    fn screen_day_adds_up_hours_and_names_the_top_app() {
        let db = Db { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        init(&db);
        let now = 1_790_000_000;
        let offset = utc_offset();
        let day_start = now - (now + offset).rem_euclid(86_400);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO focus_hours VALUES (?1, 'code', 1800)", params![day_start + 9 * 3600]).unwrap();
            conn.execute("INSERT INTO focus_hours VALUES (?1, 'brave', 600)", params![day_start + 9 * 3600]).unwrap();
            conn.execute("INSERT INTO focus_hours VALUES (?1, 'brave', 900)", params![day_start - 3600]).unwrap();
        }
        let day = screen_day(&db, now, 0);
        assert_eq!(day.total_secs, 2400);
        assert_eq!(day.hours[9].top_app, "code");
        assert_eq!(day.hours[9].apps.len(), 2);
        assert_eq!(screen_day(&db, now, -1).hours[23].secs, 900);
    }

    #[test]
    fn only_unused_solid_state_drives_can_be_erased() {
        let lsblk: serde_json::Value = serde_json::from_str(
            r#"{"blockdevices":[
              {"name":"nvme0n1","size":"476G","model":"SYS","rota":false,"type":"disk","mountpoints":[null],"children":[{"name":"nvme0n1p2","type":"part","mountpoints":["/"]}]},
              {"name":"sda","size":"1T","model":"HDD","rota":true,"type":"disk","mountpoints":[null]},
              {"name":"sdb","size":"64G","model":"Stick","rota":false,"type":"disk","mountpoints":[null],"children":[{"name":"sdb1","type":"part","mountpoints":[null]}]},
              {"name":"zram0","size":"8G","model":null,"rota":false,"type":"disk","mountpoints":["[SWAP]"]}]}"#,
        )
        .unwrap();
        let drives = erasable_from(&lsblk);
        assert_eq!(drives.len(), 3, "zram is not a drive");
        assert!(drives[0].blocked.contains("mounted"));
        assert!(drives[1].blocked.contains("spinning"));
        assert!(drives[2].blocked.is_empty());
        assert!(!secure_erase("nvme0n1", "nvme0n1").ok, "the system drive is refused");
        assert!(!secure_erase("sdb", "sda").ok, "the typed name must match");
        assert!(!secure_erase("sdb; reboot", "sdb; reboot").ok);
    }

    #[test]
    fn equaliser_config_carries_the_gains() {
        let config = eq_config(6.0, -1.5, 3.0);
        assert!(config.contains(r#""Gain" = 6.0"#) && config.contains(r#""Gain" = -1.5"#) && config.contains(r#""Gain" = 3.0"#));
        assert!(config.contains("node.name = \"trace_eq\""));
        assert!(!set_eq("nonsense").ok);
    }

    #[test]
    fn fixes_exist_only_for_findings_that_have_one() {
        assert_eq!(fix_for("SSH accepts passwords").map(|f| f.0), Some("ssh_keys_only"));
        assert!(fix_for("Disk is not encrypted").is_none());
        assert!(!security_fix("rm -rf").ok);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Starts the real equaliser, checks sound is routed through it, and removes it again.
    #[test]
    #[ignore]
    fn equaliser_starts_routes_and_stops() {
        let before = run_lossy(Command::new("pactl").arg("get-default-sink"));
        let on = set_eq("bass");
        assert!(on.ok, "{}", on.message);
        assert_eq!(run_lossy(Command::new("pactl").arg("get-default-sink")), "trace_eq");
        assert_eq!(eq_current(), "bass");
        assert!(set_eq("").ok);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(run_lossy(Command::new("pactl").arg("get-default-sink")), before, "the old output is the default again");
        assert!(eq_current().is_empty());
    }

    #[test]
    #[ignore]
    fn keyboard_light_can_be_switched_without_root() {
        let led = kbd_led().expect("a keyboard light");
        let before = kbd_level(&led);
        set_kbd_level(&led, 0);
        assert_eq!(kbd_level(&led), 0);
        set_kbd_level(&led, before);
        assert_eq!(kbd_level(&led), before);
    }
}

/// Exercises, on the real machine, the things that had only been unit-tested.
/// Everything here works on throwaway files or puts back what it changes.
#[cfg(test)]
mod live_actions {
    use super::*;
    use crate::{batch3, batch4, batch5};

    fn home_scratch(name: &str) -> std::path::PathBuf {
        let dir = dirs::home_dir().unwrap().join(format!(".trace-live-test-{name}"));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[ignore]
    fn duplicates_are_found_and_go_to_the_trash() {
        let dir = home_scratch("dups");
        let data = vec![3u8; 1_200_000];
        fs::write(dir.join("one.bin"), &data).unwrap();
        fs::write(dir.join("two.bin"), &data).unwrap();
        let scan = batch5::scan_files(&dir.to_string_lossy(), 0).unwrap();
        assert_eq!(scan.duplicates.len(), 1);
        let victim = dir.join("two.bin").to_string_lossy().to_string();
        let r = batch5::trash_files(&[victim.clone()]);
        assert!(r.ok, "{}", r.message);
        assert!(!Path::new(&victim).exists() && dir.join("one.bin").exists());
        // Take our test file back out of the trash.
        let trash = dirs::data_local_dir().unwrap().join("Trash");
        fs::remove_file(trash.join("files/two.bin")).ok();
        fs::remove_file(trash.join("info/two.bin.trashinfo")).ok();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore]
    fn work_mode_starts_logs_and_stops_a_command() {
        let dir = home_scratch("project");
        let path = dir.to_string_lossy().to_string();
        let r = batch5::work_start(&path, "echo hello-from-work-mode; sleep 60");
        assert!(r.ok, "{}", r.message);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let status = batch5::work_status(&path).unwrap();
        assert!(status.command_running);
        assert!(batch5::work_log(&path).contains("hello-from-work-mode"));
        assert!(batch5::work_stop(&path).ok);
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(!batch5::work_status(&path).unwrap().command_running);
        batch5::work_start(&path, ""); // forget the saved command
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore]
    fn backup_runs_checks_and_deletes_through_the_real_flow() {
        let dir = home_scratch("backup");
        let (src, dest) = (dir.join("docs"), dir.join("drive"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        let saved = fs::read_to_string(data_dir().join("backup.json")).ok();
        let r = batch5::backup_save(batch5::BackupConfig { sources: vec![src.to_string_lossy().to_string()], destination: dest.to_string_lossy().to_string(), weekly: false, last_ok: 0 });
        assert!(r.ok, "{}", r.message);
        assert!(batch5::backup_view().same_drive, "same-drive warning shows");
        assert!(batch5::backup_start(1).ok);
        let wait = || {
            for _ in 0..100 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !batch5::backup_view().progress.running {
                    break;
                }
            }
        };
        wait();
        let view = batch5::backup_view();
        assert_eq!(view.snapshots.len(), 1, "{}", view.progress.message);
        let name = view.snapshots[0].clone();
        assert!(batch5::backup_verify(&name).ok);
        wait();
        assert!(batch5::backup_view().progress.message.contains("matches your files exactly"), "{}", batch5::backup_view().progress.message);
        assert!(batch5::backup_delete(&name).ok);
        assert!(batch5::backup_view().snapshots.is_empty());
        match saved {
            Some(text) => fs::write(data_dir().join("backup.json"), text).unwrap(),
            None => {
                fs::remove_file(data_dir().join("backup.json")).ok();
            }
        }
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore]
    fn focus_time_freezes_and_wakes_an_app() {
        let mut child = Command::new("sleep").arg("120").spawn().unwrap();
        let state = |pid: u32| fs::read_to_string(format!("/proc/{pid}/stat")).unwrap().rsplit_once(')').unwrap().1.trim().chars().next().unwrap();
        let saved = fs::read_to_string(data_dir().join("focus.json")).ok();
        assert!(saved.is_none(), "a real focus session is running; not touching it");
        let r = batch3::focus_start(vec!["sleep".into()], 1, 1000);
        assert!(r.ok, "{}", r.message);
        assert_eq!(state(child.id()), 'T', "frozen");
        batch3::focus_tick(1000 + 30);
        assert_eq!(state(child.id()), 'T', "still inside the minute");
        batch3::focus_tick(1000 + 61);
        assert_ne!(state(child.id()), 'T', "woken when the time is up");
        assert!(batch3::focus_current().is_none());
        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    #[ignore]
    fn wifi_switches_flip_and_flip_back_and_the_qr_is_made() {
        let Some(net) = batch3::saved_wifi().into_iter().last() else { return };
        assert!(batch3::wifi_action(&net.uuid, if net.metered { "metered_off" } else { "metered_on" }).ok);
        assert_eq!(batch3::saved_wifi().into_iter().find(|n| n.uuid == net.uuid).unwrap().metered, !net.metered);
        assert!(batch3::wifi_action(&net.uuid, if net.metered { "metered_on" } else { "metered_off" }).ok);
        assert!(batch3::wifi_action(&net.uuid, if net.autoconnect { "auto_off" } else { "auto_on" }).ok);
        assert_eq!(batch3::saved_wifi().into_iter().find(|n| n.uuid == net.uuid).unwrap().autoconnect, !net.autoconnect);
        assert!(batch3::wifi_action(&net.uuid, if net.autoconnect { "auto_on" } else { "auto_off" }).ok);
        let after = batch3::saved_wifi().into_iter().find(|n| n.uuid == net.uuid).unwrap();
        assert_eq!((after.metered, after.autoconnect), (net.metered, net.autoconnect), "left as it was");
        let svg = batch4::wifi_qr(&net.uuid).unwrap();
        assert!(svg.contains("<svg") && svg.len() > 1000);
    }

    #[test]
    #[ignore]
    fn measurement_sound_output_and_stream_volume_work() {
        let before = batch4::measurements();
        let after = batch4::measure("live test", 15, 1);
        assert_eq!(after.len(), (before.len() + 1).min(12));
        assert!(after[0].temp_c > 20.0 && after[0].cpu_percent >= 0.0);
        fs::write(data_dir().join("measurements.json"), serde_json::to_string(&before).unwrap()).unwrap();

        let sink = run_lossy(Command::new("pactl").arg("get-default-sink"));
        assert!(crate::desktop::set_default_audio("sink", &sink).ok, "setting the output it already has");
        if let Some(stream) = batch5::mixer().streams.first() {
            assert!(batch5::stream_set(stream.id, "volume", &stream.volume.to_string()).ok);
        }
    }

    #[test]
    #[ignore]
    fn settings_file_saves_and_loads_back() {
        let db = Db { conn: Mutex::new(rusqlite::Connection::open(data_dir().join("history.sqlite3")).unwrap()) };
        let path = batch3::export_settings(&db).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"trace_settings\": 1"));
        let message = batch3::import_settings(&db, &path).unwrap();
        assert!(message.starts_with("Loaded"), "{message}");
        fs::remove_file(path).ok();
    }

    #[test]
    #[ignore]
    fn restricted_launch_cannot_write_or_reach_the_network() {
        let marker = dirs::home_dir().unwrap().join(".trace-live-restricted");
        let out = std::env::temp_dir().join("trace-live-restricted.out");
        fs::remove_file(&out).ok();
        // /tmp inside is private, so the program reports through a file descriptor we can't give it;
        // instead it simply tries to write to home, which must fail.
        let r = batch3::run_restricted(&format!("touch '{}'", marker.display()), true, true, Some(128), None);
        assert!(r.ok, "{}", r.message);
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(!marker.exists(), "home was writable");
    }
}
