//! Smaller conveniences: a plain answer to "why is it like this right now",
//! scheduled jobs, "tell me when it's done", countdown timers, hardware
//! trouble spotted in the kernel log, and putting unused apps to sleep.

use crate::db::Db;
use crate::sys_util::{data_dir, notify, run_lossy};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

// ---- scheduled jobs ----

#[derive(Serialize)]
pub struct TimerJob {
    pub unit: String,
    pub activates: String,
    pub user: bool,
    /// Unix seconds; 0 when not scheduled / never ran.
    pub next: i64,
    pub last: i64,
}

pub fn timers() -> Vec<TimerJob> {
    let mut jobs = Vec::new();
    for user in [false, true] {
        let mut cmd = Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        let out = run_lossy(cmd.args(["list-timers", "--all", "--output=json", "--no-pager"]));
        let Ok(serde_json::Value::Array(rows)) = serde_json::from_str(&out) else {
            continue;
        };
        for row in rows {
            let micros = |key: &str| row[key].as_i64().unwrap_or(0) / 1_000_000;
            jobs.push(TimerJob {
                unit: row["unit"].as_str().unwrap_or_default().to_string(),
                activates: row["activates"].as_str().unwrap_or_default().to_string(),
                user,
                next: micros("next"),
                last: micros("last"),
            });
        }
    }
    jobs.sort_by_key(|j| if j.next == 0 { i64::MAX } else { j.next });
    jobs
}

// ---- watches and countdowns ----

#[derive(Serialize, Clone)]
pub struct Watch {
    pub pid: u32,
    pub name: String,
    pub since: i64,
}

#[derive(Serialize, Clone)]
pub struct Countdown {
    /// "lock" | "suspend" | "poweroff" | "awake_off"
    pub action: String,
    pub at: i64,
}

#[derive(Default)]
pub struct Pending {
    pub watches: Vec<Watch>,
    pub countdown: Option<Countdown>,
}

/// Fires finished watches and a due countdown. Returns the session action
/// to run, if the countdown came due.
pub fn tick_pending(pending: &mut Pending, now: i64) -> Option<String> {
    pending.watches.retain(|w| {
        if std::path::Path::new(&format!("/proc/{}", w.pid)).exists() {
            return true;
        }
        let took = now - w.since;
        notify(
            &format!("{} has finished", w.name),
            &format!("It ran for {} min {} s after you asked to be told.", took / 60, took % 60),
            false,
        );
        false
    });
    match &pending.countdown {
        Some(c) if c.at <= now => pending.countdown.take().map(|c| c.action),
        // A one-minute warning before anything that ends the session.
        Some(c) if c.at - now <= 60 && c.at - now > 50 && c.action != "awake_off" => {
            notify("Trace timer", &format!("The machine will {} in one minute.", c.action), true);
            None
        }
        _ => None,
    }
}

// ---- why is it like this right now ----

pub struct NowFacts {
    pub cpu_percent: f32,
    pub top_cpu: Vec<(String, f32)>,
    pub top_memory: Vec<(String, u64)>,
    pub mem_available_percent: f32,
    pub swap_used_percent: f32,
    pub hottest_c: f32,
    pub mem_stall: f32,
    pub io_stall: f32,
    pub cpu_stall: f32,
    pub cores: usize,
    pub on_battery: bool,
    pub power_profile: String,
    pub fans_rpm: Vec<u32>,
}

#[derive(Serialize)]
pub struct Explanation {
    pub headline: String,
    pub points: Vec<String>,
}

/// A plain-language account of the machine's current state, most pressing
/// thing first. Purely rules over measurements — no guessing.
pub fn explain(f: &NowFacts) -> Explanation {
    let mut points = Vec::new();
    let mut headline = "Nothing is straining the machine right now.".to_string();
    let top_cpu = f.top_cpu.first();
    let gb = |b: u64| b as f64 / 1024.0 / 1024.0 / 1024.0;

    if f.mem_stall > 10.0 {
        headline = "It is slow because it has run short of memory.".to_string();
        points.push(format!(
            "Programs spent {:.0}% of the last few seconds waiting while the kernel made room. Only {:.0}% of RAM is available and swap is {:.0}% full.",
            f.mem_stall, f.mem_available_percent, f.swap_used_percent
        ));
        if let Some((app, bytes)) = f.top_memory.first() {
            points.push(format!("The largest app is {app} at {:.1} GB — closing or restarting it helps most.", gb(*bytes)));
        }
    } else if f.io_stall > 25.0 {
        headline = "It is slow because the disk is the bottleneck.".to_string();
        points.push(format!(
            "Programs spent {:.0}% of the last few seconds waiting on reads or writes. The Activity page shows who is using the disk.",
            f.io_stall
        ));
    } else if f.cpu_percent > 70.0 || f.cpu_stall > 40.0 {
        headline = "The processor is busy.".to_string();
        if let Some((app, percent)) = top_cpu {
            points.push(format!(
                "{app} is using {:.0}% of one core's worth ({:.1} of your {} cores).",
                percent,
                percent / 100.0,
                f.cores
            ));
        }
    }

    if f.hottest_c >= 85.0 {
        if headline.starts_with("Nothing") {
            headline = format!("It is hot: {:.0}°C.", f.hottest_c);
        }
        match top_cpu {
            Some((app, percent)) if *percent > 40.0 => {
                points.push(format!("The heat ({:.0}°C) follows the load: {app} is the main source.", f.hottest_c))
            }
            _ => points.push(format!(
                "It is {:.0}°C without much load, which points at blocked vents, dust, or a hot surface under the laptop.",
                f.hottest_c
            )),
        }
    }
    let fans: Vec<u32> = f.fans_rpm.iter().copied().filter(|r| *r > 0).collect();
    if !fans.is_empty() {
        let fastest = fans.iter().max().copied().unwrap_or(0);
        points.push(if fastest > 4500 {
            format!("The fans are working hard ({fastest} RPM) to hold {:.0}°C.", f.hottest_c)
        } else {
            format!("Fans are at {fastest} RPM; temperature {:.0}°C.", f.hottest_c)
        });
    }
    if !f.power_profile.is_empty() {
        points.push(match (f.power_profile.as_str(), f.on_battery) {
            ("performance", true) => "Power profile is performance while on battery — fast, but it drains quickly and runs hot.".to_string(),
            ("power-saver", false) => "Power profile is power-saver although you are plugged in, which holds speed back.".to_string(),
            (profile, battery) => format!("Power profile: {profile}, {}.", if battery { "on battery" } else { "plugged in" }),
        });
    }
    if headline.starts_with("Nothing") && f.swap_used_percent > 50.0 {
        points.push(format!(
            "Swap is {:.0}% used, so switching back to apps you haven't touched in a while may be sluggish for a moment.",
            f.swap_used_percent
        ));
    }
    if points.is_empty() {
        points.push(format!("CPU at {:.0}%, {:.0}% of RAM available, no stalls.", f.cpu_percent, f.mem_available_percent));
    }
    Explanation { headline, points }
}

// ---- hardware trouble in the kernel log ----

/// Kernel messages that mean hardware is misbehaving, and what to say about them.
const TROUBLE: &[(&str, &str)] = &[
    ("I/O error", "A drive reported a read or write error"),
    ("failed command", "A drive failed a command"),
    ("nvme nvme", "The SSD controller logged a problem"),
    ("EXT4-fs error", "The filesystem found damage"),
    ("BTRFS error", "The filesystem found damage"),
    ("Machine check", "The processor reported a hardware fault"),
    ("mce: [Hardware Error]", "The processor reported a hardware fault"),
    ("clock throttled", "The processor slowed itself down because of heat"),
    ("AER: ", "A device on the PCIe bus reported errors"),
    ("NVRM: Xid", "The NVIDIA driver reported a GPU fault"),
    ("fallen off the bus", "The graphics card stopped responding"),
    ("over-current", "A USB port drew too much power"),
    ("device descriptor read", "A USB device is failing to connect properly"),
    ("Out of memory:", "The kernel ran out of memory and ended a program"),
];

fn classify_trouble(message: &str) -> Option<&'static str> {
    // The SSD line is only trouble when it mentions a failure.
    TROUBLE.iter().find_map(|(needle, meaning)| {
        let hit = message.contains(needle)
            && (*needle != "nvme nvme" || message.contains("timeout") || message.contains("error") || message.contains("reset"));
        hit.then_some(*meaning)
    })
}

/// Copies new hardware trouble from the kernel log into Trace's events, in
/// plain words. Remembers how far it has read.
pub fn ingest_trouble(db: &Db) {
    let last: i64 = {
        let conn = db.conn.lock().unwrap();
        conn.query_row("SELECT value FROM logbook_state WHERE key = 'trouble_us'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let out = run_lossy(Command::new("journalctl").args(["-k", "-b", "-p", "4", "-o", "json", "--no-pager", "-q"]));
    let mut newest = last;
    let mut found: Vec<(i64, String)> = Vec::new();
    for line in out.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let time_us: i64 = entry["__REALTIME_TIMESTAMP"].as_str().and_then(|t| t.parse().ok()).unwrap_or(0);
        if time_us <= last {
            continue;
        }
        newest = newest.max(time_us);
        let message = entry["MESSAGE"].as_str().unwrap_or_default();
        if let Some(meaning) = classify_trouble(message) {
            let detail: String = message.chars().take(140).collect();
            found.push((time_us / 1_000_000, format!("{meaning} ({detail})")));
        }
    }
    let conn = db.conn.lock().unwrap();
    // The same fault often repeats many times a second; one entry per kind per minute is plenty.
    let mut seen: HashMap<(i64, String), ()> = HashMap::new();
    for (ts, message) in found {
        let kind = message.split(" (").next().unwrap_or("").to_string();
        if seen.insert((ts / 60, kind), ()).is_none() {
            conn.execute(
                "INSERT INTO events (ts, event_type, severity, message) VALUES (?1, 'hardware_trouble', 'warn', ?2)",
                params![ts, message],
            )
            .ok();
        }
    }
    conn.execute(
        "INSERT OR REPLACE INTO logbook_state (key, value) VALUES ('trouble_us', ?1)",
        params![newest.to_string()],
    )
    .ok();
}

// ---- putting unused apps to sleep ----

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sleeper {
    pub app: String,
    /// Minutes without focus before the app is frozen.
    pub minutes: u32,
}

fn frozen_path() -> std::path::PathBuf {
    data_dir().join("frozen-apps.json")
}

fn store_frozen(frozen: &[String]) {
    std::fs::write(frozen_path(), serde_json::to_string(frozen).unwrap_or_default()).ok();
}

/// Resumes anything a previous run froze and never woke (a crash, a kill).
pub fn recover_frozen() {
    let frozen: Vec<String> = std::fs::read_to_string(frozen_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for app in &frozen {
        crate::memory_map::app_action(app, "resume");
    }
    store_frozen(&[]);
}

#[derive(Default)]
pub struct SleeperState {
    last_focus: HashMap<String, i64>,
    pub frozen: Vec<String>,
}

impl SleeperState {
    /// One pass: wake the focused app at once, freeze listed apps that have
    /// gone unfocused long enough. Returns what changed, for the log.
    pub fn step(&mut self, sleepers: &[Sleeper], focused: Option<&str>, now: i64) -> Vec<String> {
        let mut log = Vec::new();
        if let Some(app) = focused {
            self.last_focus.insert(app.to_string(), now);
            if self.frozen.iter().any(|f| f == app) {
                crate::memory_map::app_action(app, "resume");
                self.frozen.retain(|f| f != app);
                store_frozen(&self.frozen);
                log.push(format!("Woke {app}"));
            }
        }
        // Anything frozen that is no longer on the list wakes up too.
        let dropped: Vec<String> = self
            .frozen
            .iter()
            .filter(|f| !sleepers.iter().any(|s| s.app == **f))
            .cloned()
            .collect();
        for app in dropped {
            crate::memory_map::app_action(&app, "resume");
            self.frozen.retain(|f| *f != app);
            store_frozen(&self.frozen);
            log.push(format!("Woke {app}"));
        }
        for sleeper in sleepers {
            if self.frozen.contains(&sleeper.app) || focused == Some(sleeper.app.as_str()) {
                continue;
            }
            // An app is counted from when it was first seen, not from Trace's start.
            let since = *self.last_focus.entry(sleeper.app.clone()).or_insert(now);
            if now - since >= sleeper.minutes.max(1) as i64 * 60
                && !crate::memory_map::pids_of_app(&sleeper.app).is_empty()
                && crate::memory_map::app_action(&sleeper.app, "freeze").ok
            {
                self.frozen.push(sleeper.app.clone());
                store_frozen(&self.frozen);
                log.push(format!("Put {} to sleep after {} min unused", sleeper.app, sleeper.minutes));
            }
        }
        log
    }

    pub fn wake_all(&mut self) {
        for app in std::mem::take(&mut self.frozen) {
            crate::memory_map::app_action(&app, "resume");
        }
        store_frozen(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> NowFacts {
        NowFacts {
            cpu_percent: 5.0,
            top_cpu: vec![("brave".to_string(), 12.0)],
            top_memory: vec![("code".to_string(), 3 * 1024 * 1024 * 1024)],
            mem_available_percent: 40.0,
            swap_used_percent: 10.0,
            hottest_c: 55.0,
            mem_stall: 0.0,
            io_stall: 0.0,
            cpu_stall: 0.0,
            cores: 16,
            on_battery: false,
            power_profile: "balanced".to_string(),
            fans_rpm: vec![2400],
        }
    }

    #[test]
    fn explains_the_most_pressing_thing_first() {
        assert!(explain(&facts()).headline.starts_with("Nothing"));
        let mut f = facts();
        f.mem_stall = 22.0;
        f.cpu_percent = 95.0;
        let e = explain(&f);
        assert!(e.headline.contains("memory"), "{}", e.headline);
        assert!(e.points.iter().any(|p| p.contains("code")));
        let mut hot = facts();
        hot.hottest_c = 91.0;
        assert!(explain(&hot).points.iter().any(|p| p.contains("vents")));
    }

    #[test]
    fn recognises_trouble_but_not_routine_lines() {
        assert!(classify_trouble("blk_update_request: I/O error, dev sda, sector 1").is_some());
        assert!(classify_trouble("nvme nvme0: I/O 12 QID 3 timeout, aborting").is_some());
        assert!(classify_trouble("nvme nvme0: 16/0/0 default/read/poll queues").is_none());
        assert!(classify_trouble("asus-nb-wmi: Using throttle_thermal_policy for platform_profile").is_none());
        assert!(classify_trouble("CPU3: Core temperature above threshold, cpu clock throttled").is_some());
    }

    #[test]
    fn countdown_fires_once_when_due() {
        let mut pending = Pending { watches: Vec::new(), countdown: Some(Countdown { action: "lock".to_string(), at: 100 }) };
        assert_eq!(tick_pending(&mut pending, 10), None);
        assert_eq!(tick_pending(&mut pending, 100).as_deref(), Some("lock"));
        assert_eq!(tick_pending(&mut pending, 110), None);
    }

    #[test]
    fn sleeper_waits_its_minutes_and_never_freezes_the_focused_app() {
        let mut state = SleeperState::default();
        // An app that isn't running is never frozen, however long it is unfocused.
        let sleepers = [Sleeper { app: "trace-test-not-running".to_string(), minutes: 5 }];
        assert!(state.step(&sleepers, Some("other"), 0).is_empty());
        assert!(state.step(&sleepers, Some("other"), 10_000).is_empty());
        assert!(state.frozen.is_empty());
        // Focus resets the clock.
        state.step(&sleepers, Some("trace-test-not-running"), 20_000);
        assert_eq!(state.last_focus["trace-test-not-running"], 20_000);
    }
}
