//! The logbook: long-lived, compact records of what the machine did, kept
//! separately from the raw 10-second samples so that a year-long view stays
//! fast and survives the pruning of raw history.

use crate::db::{ActionRow, BootPoint, Db, EventRow};
use crate::sys_util::run_lossy;
use rusqlite::params;
use serde::Serialize;
use std::process::Command;

pub fn init(db: &Db) {
    let conn = db.conn.lock().unwrap();
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS hourly_stats (
            hour_ts INTEGER PRIMARY KEY,
            samples INTEGER NOT NULL,
            cpu_sum REAL NOT NULL,
            cpu_max REAL NOT NULL,
            ram_sum REAL NOT NULL,
            ram_max REAL NOT NULL,
            swap_max_bytes INTEGER NOT NULL,
            cpu_stall_sum REAL NOT NULL,
            mem_stall_sum REAL NOT NULL,
            io_stall_sum REAL NOT NULL
        );

        CREATE TABLE IF NOT EXISTS app_usage (
            day TEXT NOT NULL,
            app TEXT NOT NULL,
            running_secs INTEGER NOT NULL DEFAULT 0,
            focus_secs INTEGER NOT NULL DEFAULT 0,
            ram_sum INTEGER NOT NULL DEFAULT 0,
            ram_samples INTEGER NOT NULL DEFAULT 0,
            ram_peak INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (day, app)
        );

        CREATE TABLE IF NOT EXISTS battery_daily (
            day TEXT PRIMARY KEY,
            discharged_wh REAL NOT NULL DEFAULT 0,
            charged_wh REAL NOT NULL DEFAULT 0,
            on_battery_secs INTEGER NOT NULL DEFAULT 0,
            min_percent REAL NOT NULL DEFAULT 100,
            max_percent REAL NOT NULL DEFAULT 0,
            cycle_count INTEGER NOT NULL DEFAULT 0,
            full_wh REAL NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS net_daily (
            day TEXT NOT NULL,
            iface TEXT NOT NULL,
            rx_bytes INTEGER NOT NULL DEFAULT 0,
            tx_bytes INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (day, iface)
        );

        CREATE TABLE IF NOT EXISTS power_events (
            time_us INTEGER NOT NULL,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL,
            PRIMARY KEY (time_us, kind)
        );

        CREATE TABLE IF NOT EXISTS package_events (
            ts INTEGER NOT NULL,
            action TEXT NOT NULL,
            name TEXT NOT NULL,
            old_version TEXT NOT NULL,
            new_version TEXT NOT NULL,
            PRIMARY KEY (ts, name, action)
        );

        CREATE TABLE IF NOT EXISTS notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            critical INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_notifications_ts ON notifications(ts);

        CREATE TABLE IF NOT EXISTS logbook_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS crashes (
            time_us INTEGER NOT NULL,
            pid INTEGER NOT NULL,
            exe TEXT NOT NULL,
            signal INTEGER NOT NULL,
            kind TEXT NOT NULL,
            PRIMARY KEY (time_us, pid)
        );
        ",
    )
    .expect("failed to init logbook schema");

    // Temperature columns were added after the table first shipped.
    let has_temp = conn
        .prepare("SELECT temp_max FROM hourly_stats LIMIT 1")
        .is_ok();
    if !has_temp {
        conn.execute_batch(
            "ALTER TABLE hourly_stats ADD COLUMN temp_sum REAL NOT NULL DEFAULT 0;
             ALTER TABLE hourly_stats ADD COLUMN temp_max REAL NOT NULL DEFAULT 0;
             ALTER TABLE hourly_stats ADD COLUMN temp_samples INTEGER NOT NULL DEFAULT 0;",
        )
        .ok();
    }

    // First run after upgrading: seed the hourly table from the raw samples
    // already on disk so the logbook doesn't start empty.
    let empty: bool = conn
        .query_row("SELECT NOT EXISTS (SELECT 1 FROM hourly_stats)", [], |r| r.get(0))
        .unwrap_or(false);
    if empty {
        conn.execute(
            "INSERT OR IGNORE INTO hourly_stats
                (hour_ts, samples, cpu_sum, cpu_max, ram_sum, ram_max, swap_max_bytes,
                 cpu_stall_sum, mem_stall_sum, io_stall_sum)
             SELECT ts - ((ts + ?1) % 3600), COUNT(*), SUM(cpu_percent), MAX(cpu_percent),
                    SUM(used_memory_bytes * 100.0 / total_memory_bytes),
                    MAX(used_memory_bytes * 100.0 / total_memory_bytes),
                    MAX(used_swap_bytes), 0, 0, 0
             FROM resource_snapshots WHERE total_memory_bytes > 0 GROUP BY 1",
            params![utc_offset_secs()],
        )
        .ok();
    }
}

fn utc_offset_secs() -> i64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_gmtoff
    }
}

pub struct Sample {
    pub cpu_percent: f32,
    pub ram_percent: f32,
    pub swap_bytes: u64,
    pub cpu_stall: f32,
    pub mem_stall: f32,
    pub io_stall: f32,
    /// Hottest sensor, or 0 when the machine reports none.
    pub temp_c: f32,
}

/// Folds one 10-second sample into its local-time hour.
pub fn record_sample(db: &Db, ts: i64, s: &Sample) {
    let hour_ts = ts - ((ts + utc_offset_secs()).rem_euclid(3600));
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO hourly_stats
            (hour_ts, samples, cpu_sum, cpu_max, ram_sum, ram_max, swap_max_bytes,
             cpu_stall_sum, mem_stall_sum, io_stall_sum, temp_sum, temp_max, temp_samples)
         VALUES (?1, 1, ?2, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
         ON CONFLICT(hour_ts) DO UPDATE SET
            temp_sum = temp_sum + excluded.temp_sum,
            temp_max = MAX(temp_max, excluded.temp_max),
            temp_samples = temp_samples + excluded.temp_samples,
            samples = samples + 1,
            cpu_sum = cpu_sum + excluded.cpu_sum,
            cpu_max = MAX(cpu_max, excluded.cpu_max),
            ram_sum = ram_sum + excluded.ram_sum,
            ram_max = MAX(ram_max, excluded.ram_max),
            swap_max_bytes = MAX(swap_max_bytes, excluded.swap_max_bytes),
            cpu_stall_sum = cpu_stall_sum + excluded.cpu_stall_sum,
            mem_stall_sum = mem_stall_sum + excluded.mem_stall_sum,
            io_stall_sum = io_stall_sum + excluded.io_stall_sum",
        params![
            hour_ts,
            s.cpu_percent,
            s.ram_percent,
            s.swap_bytes as i64,
            s.cpu_stall,
            s.mem_stall,
            s.io_stall,
            s.temp_c,
            (s.temp_c > 0.0) as i32
        ],
    )
    .ok();
}

/// Credits `secs` of running time (and a memory sample) to each app for today.
pub fn record_running(db: &Db, ts: i64, secs: i64, apps: &[(String, u64)]) {
    let mut conn = db.conn.lock().unwrap();
    let Ok(tx) = conn.transaction() else {
        return;
    };
    for (app, ram) in apps {
        tx.execute(
            "INSERT INTO app_usage (day, app, running_secs, ram_sum, ram_samples, ram_peak)
             VALUES (date(?1, 'unixepoch', 'localtime'), ?2, ?3, ?4, 1, ?4)
             ON CONFLICT(day, app) DO UPDATE SET
                running_secs = running_secs + excluded.running_secs,
                ram_sum = ram_sum + excluded.ram_sum,
                ram_samples = ram_samples + 1,
                ram_peak = MAX(ram_peak, excluded.ram_peak)",
            params![ts, app, secs, *ram as i64],
        )
        .ok();
    }
    tx.commit().ok();
}

/// Credits `secs` of foreground time to the app whose window has focus.
pub fn record_focus(db: &Db, ts: i64, secs: i64, app: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO app_usage (day, app, focus_secs)
         VALUES (date(?1, 'unixepoch', 'localtime'), ?2, ?3)
         ON CONFLICT(day, app) DO UPDATE SET focus_secs = focus_secs + excluded.focus_secs",
        params![ts, app, secs],
    )
    .ok();
}

/// The app owning the focused window (Hyprland), named the same way as on
/// the Memory page so the two line up.
pub fn focused_app() -> Option<String> {
    let out = run_lossy(Command::new("hyprctl").args(["activewindow", "-j"]));
    let json: serde_json::Value = serde_json::from_str(&out).ok()?;
    let pid = json["pid"].as_u64()?;
    crate::memory_map::app_name(&pid.to_string())
}

/// Copies crash records out of the system journal, which rotates them away
/// after a while. Safe to call repeatedly: known crashes are skipped.
pub fn ingest_crashes(db: &Db, first_run: bool) {
    let mut found: Vec<(i64, i64, String, i64, &str)> = Vec::new();

    let mut cmd = Command::new("coredumpctl");
    cmd.args(["list", "--json=short", "--no-pager", "-q"]);
    if !first_run {
        cmd.arg("--since=-2d");
    }
    if let Ok(serde_json::Value::Array(rows)) = serde_json::from_str(&run_lossy(&mut cmd)) {
        for row in rows {
            let exe = row["exe"].as_str().unwrap_or("unknown");
            found.push((
                row["time"].as_i64().unwrap_or(0),
                row["pid"].as_i64().unwrap_or(0),
                exe.rsplit('/').next().unwrap_or(exe).to_string(),
                row["sig"].as_i64().unwrap_or(0),
                "crash",
            ));
        }
    }

    let since = if first_run { "--since=-60d" } else { "--since=-2d" };
    let oom = run_lossy(Command::new("journalctl").args([
        since,
        "-o",
        "json",
        "--no-pager",
        "-q",
        "-g",
        "Out of memory: Killed process|Killed .* due to memory (pressure|used)",
    ]));
    for line in oom.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let message = entry["MESSAGE"].as_str().unwrap_or_default();
        let time_us: i64 = entry["__REALTIME_TIMESTAMP"]
            .as_str()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        // "Out of memory: Killed process 1234 (name) ..." or
        // "Killed /user.slice/.../app-name.scope due to memory pressure ..."
        let (pid, name) = match message.split_once("Killed process ") {
            Some((_, rest)) => {
                let pid = rest.split_whitespace().next().and_then(|p| p.parse().ok()).unwrap_or(0);
                let name = rest
                    .split_once('(')
                    .and_then(|(_, r)| r.split_once(')'))
                    .map(|(n, _)| n.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                (pid, name)
            }
            None => {
                let unit = message
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.rsplit('/').next())
                    .unwrap_or("unknown");
                (0, unit.to_string())
            }
        };
        found.push((time_us, pid, name, 9, "oom"));
    }

    let mut conn = db.conn.lock().unwrap();
    let Ok(tx) = conn.transaction() else {
        return;
    };
    for (time_us, pid, exe, signal, kind) in found.into_iter().filter(|c| c.0 > 0) {
        tx.execute(
            "INSERT OR IGNORE INTO crashes (time_us, pid, exe, signal, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![time_us, pid, exe, signal, kind],
        )
        .ok();
    }
    tx.commit().ok();
}

/// Raw 10-second samples are only charted over the last month; the logbook
/// keeps the long-term picture, so anything older is dropped.
pub fn prune_raw(db: &Db, before_ts: i64) {
    let conn = db.conn.lock().unwrap();
    for table in ["resource_snapshots", "disk_snapshots", "pressure_snapshots"] {
        conn.execute(&format!("DELETE FROM {table} WHERE ts < ?1"), params![before_ts])
            .ok();
    }
}

// ---- periods ----

/// Local-time start of the period `offset` steps back (0 = the current one),
/// followed by the start of every bucket in it and finally the period's end.
/// Day → 24 hours, week → 7 days (from Monday), month → its days, year → 12 months.
fn period_edges(kind: &str, offset: i32) -> Option<Vec<i64>> {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        let buckets = match kind {
            "day" => {
                tm.tm_mday -= offset;
                24
            }
            "week" => {
                tm.tm_mday -= (tm.tm_wday + 6) % 7 + 7 * offset;
                7
            }
            "month" => {
                tm.tm_mday = 1;
                tm.tm_mon -= offset;
                // Normalise first so the month length is for the right month.
                tm.tm_isdst = -1;
                libc::mktime(&mut tm);
                let mut next = tm;
                next.tm_mon += 1;
                next.tm_isdst = -1;
                let days = (libc::mktime(&mut next) - libc::mktime(&mut tm.clone())) as f64 / 86_400.0;
                days.round() as i32
            }
            "year" => {
                tm.tm_mday = 1;
                tm.tm_mon = 0;
                tm.tm_year -= offset;
                12
            }
            _ => return None,
        };
        // mktime normalises out-of-range fields (day 32 → next month), which
        // is what makes stepping by calendar units safe across DST and month ends.
        let edges = (0..=buckets)
            .map(|i| {
                let mut step = tm;
                match kind {
                    "day" => step.tm_hour += i,
                    "year" => step.tm_mon += i,
                    _ => step.tm_mday += i,
                }
                step.tm_isdst = -1;
                libc::mktime(&mut step) as i64
            })
            .collect();
        Some(edges)
    }
}

#[derive(Serialize, Default, Clone)]
pub struct Totals {
    /// Hours the machine was on with Trace recording.
    pub hours_recorded: f64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub ram_avg: f64,
    pub ram_max: f64,
    pub swap_max_bytes: i64,
    pub cpu_stall_avg: f64,
    pub mem_stall_avg: f64,
    pub io_stall_avg: f64,
    pub temp_avg: f64,
    pub temp_max: f64,
}

#[derive(Serialize)]
pub struct Bucket {
    pub start_ts: i64,
    pub totals: Totals,
    pub crashes: i64,
}

#[derive(Serialize)]
pub struct AppUsage {
    pub app: String,
    pub running_secs: i64,
    pub focus_secs: i64,
    pub ram_avg_bytes: i64,
    pub ram_peak_bytes: i64,
}

#[derive(Serialize)]
pub struct Crash {
    pub ts: i64,
    pub pid: i64,
    pub exe: String,
    pub signal: i64,
    /// "crash" (core dump) or "oom" (killed for lack of memory)
    pub kind: String,
}

#[derive(Serialize)]
pub struct EventCount {
    pub event_type: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct LogReport {
    pub kind: String,
    pub offset: i32,
    pub start_ts: i64,
    pub end_ts: i64,
    pub totals: Totals,
    pub buckets: Vec<Bucket>,
    pub apps: Vec<AppUsage>,
    pub crashes: Vec<Crash>,
    pub crash_count: i64,
    pub oom_count: i64,
    pub events: Vec<EventRow>,
    pub event_counts: Vec<EventCount>,
    pub actions: Vec<ActionRow>,
    pub actions_failed: i64,
    pub boots: Vec<BootPoint>,
    pub battery: BatteryUsage,
    pub network: Vec<NetworkUsage>,
    pub power_events: Vec<PowerEvent>,
    pub packages: Vec<PackageEvent>,
    pub package_count: i64,
    pub notifications: Vec<NotificationRow>,
}

fn totals_between(conn: &rusqlite::Connection, start: i64, end: i64) -> Totals {
    conn.query_row(
        "SELECT COALESCE(SUM(samples), 0), COALESCE(SUM(cpu_sum), 0), COALESCE(MAX(cpu_max), 0),
                COALESCE(SUM(ram_sum), 0), COALESCE(MAX(ram_max), 0), COALESCE(MAX(swap_max_bytes), 0),
                COALESCE(SUM(cpu_stall_sum), 0), COALESCE(SUM(mem_stall_sum), 0), COALESCE(SUM(io_stall_sum), 0),
                COALESCE(SUM(temp_sum), 0), COALESCE(SUM(temp_samples), 0), COALESCE(MAX(temp_max), 0)
         FROM hourly_stats WHERE hour_ts >= ?1 AND hour_ts < ?2",
        params![start, end],
        |r| {
            let samples: i64 = r.get(0)?;
            let n = samples.max(1) as f64;
            Ok(Totals {
                hours_recorded: samples as f64 * 10.0 / 3600.0,
                cpu_avg: r.get::<_, f64>(1)? / n,
                cpu_max: r.get(2)?,
                ram_avg: r.get::<_, f64>(3)? / n,
                ram_max: r.get(4)?,
                swap_max_bytes: r.get(5)?,
                cpu_stall_avg: r.get::<_, f64>(6)? / n,
                mem_stall_avg: r.get::<_, f64>(7)? / n,
                io_stall_avg: r.get::<_, f64>(8)? / n,
                temp_avg: r.get::<_, f64>(9)? / (r.get::<_, i64>(10)?.max(1) as f64),
                temp_max: r.get(11)?,
            })
        },
    )
    .unwrap_or_default()
}

pub fn report(db: &Db, kind: &str, offset: i32) -> Result<LogReport, String> {
    let edges = period_edges(kind, offset.max(0)).ok_or("Unknown period")?;
    let (start, end) = (edges[0], *edges.last().unwrap());
    let conn = db.conn.lock().unwrap();
    let count = |sql: &str| -> i64 {
        conn.query_row(sql, params![start, end], |r| r.get(0)).unwrap_or(0)
    };

    let buckets = edges
        .windows(2)
        .map(|w| Bucket {
            start_ts: w[0],
            totals: totals_between(&conn, w[0], w[1]),
            crashes: conn
                .query_row(
                    "SELECT COUNT(*) FROM crashes WHERE time_us >= ?1 * 1000000 AND time_us < ?2 * 1000000",
                    params![w[0], w[1]],
                    |r| r.get(0),
                )
                .unwrap_or(0),
        })
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT app, SUM(running_secs), SUM(focus_secs),
                    SUM(ram_sum) / MAX(SUM(ram_samples), 1), MAX(ram_peak)
             FROM app_usage
             WHERE day >= date(?1, 'unixepoch', 'localtime') AND day < date(?2, 'unixepoch', 'localtime')
             GROUP BY app ORDER BY SUM(focus_secs) DESC, SUM(running_secs) DESC LIMIT 60",
        )
        .map_err(|e| e.to_string())?;
    let apps = stmt
        .query_map(params![start, end], |r| {
            Ok(AppUsage {
                app: r.get(0)?,
                running_secs: r.get(1)?,
                focus_secs: r.get(2)?,
                ram_avg_bytes: r.get(3)?,
                ram_peak_bytes: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT time_us / 1000000, pid, exe, signal, kind FROM crashes
             WHERE time_us >= ?1 * 1000000 AND time_us < ?2 * 1000000
             ORDER BY time_us DESC LIMIT 300",
        )
        .map_err(|e| e.to_string())?;
    let crashes = stmt
        .query_map(params![start, end], |r| {
            Ok(Crash {
                ts: r.get(0)?,
                pid: r.get(1)?,
                exe: r.get(2)?,
                signal: r.get(3)?,
                kind: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT ts, event_type, severity, message FROM events
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 300",
        )
        .map_err(|e| e.to_string())?;
    let events = stmt
        .query_map(params![start, end], |r| {
            Ok(EventRow {
                ts: r.get(0)?,
                event_type: r.get(1)?,
                severity: r.get(2)?,
                message: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT event_type, COUNT(*) FROM events WHERE ts >= ?1 AND ts < ?2
             GROUP BY event_type ORDER BY COUNT(*) DESC",
        )
        .map_err(|e| e.to_string())?;
    let event_counts = stmt
        .query_map(params![start, end], |r| {
            Ok(EventCount {
                event_type: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT ts, action, ok, message FROM actions_log
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 300",
        )
        .map_err(|e| e.to_string())?;
    let actions = stmt
        .query_map(params![start, end], |r| {
            Ok(ActionRow {
                ts: r.get(0)?,
                action: r.get(1)?,
                ok: r.get::<_, i32>(2)? != 0,
                message: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT ts, total_ms, userspace_ms FROM boots WHERE ts >= ?1 AND ts < ?2 ORDER BY ts DESC",
        )
        .map_err(|e| e.to_string())?;
    let boots = stmt
        .query_map(params![start, end], |r| {
            Ok(BootPoint {
                ts: r.get(0)?,
                total_ms: r.get::<_, i64>(1)? as u64,
                userspace_ms: r.get::<_, i64>(2)? as u64,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let battery = conn
        .query_row(
            "SELECT COALESCE(SUM(discharged_wh), 0), COALESCE(SUM(charged_wh), 0),
                    COALESCE(SUM(on_battery_secs), 0), COALESCE(MIN(min_percent), 0),
                    COALESCE(MAX(max_percent), 0), COALESCE(MAX(cycle_count), 0) - COALESCE(MIN(cycle_count), 0),
                    COUNT(*)
             FROM battery_daily
             WHERE day >= date(?1, 'unixepoch', 'localtime') AND day < date(?2, 'unixepoch', 'localtime')",
            params![start, end],
            |r| {
                Ok(BatteryUsage {
                    discharged_wh: r.get(0)?,
                    charged_wh: r.get(1)?,
                    on_battery_secs: r.get(2)?,
                    min_percent: r.get(3)?,
                    max_percent: r.get(4)?,
                    cycles_added: r.get(5)?,
                    days_recorded: r.get(6)?,
                })
            },
        )
        .unwrap_or_default();

    let mut stmt = conn
        .prepare(
            "SELECT iface, SUM(rx_bytes), SUM(tx_bytes) FROM net_daily
             WHERE day >= date(?1, 'unixepoch', 'localtime') AND day < date(?2, 'unixepoch', 'localtime')
             GROUP BY iface ORDER BY SUM(rx_bytes) + SUM(tx_bytes) DESC",
        )
        .map_err(|e| e.to_string())?;
    let network = stmt
        .query_map(params![start, end], |r| {
            Ok(NetworkUsage {
                iface: r.get(0)?,
                rx_bytes: r.get(1)?,
                tx_bytes: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT time_us / 1000000, kind, detail FROM power_events
             WHERE time_us >= ?1 * 1000000 AND time_us < ?2 * 1000000 ORDER BY time_us DESC LIMIT 400",
        )
        .map_err(|e| e.to_string())?;
    let power_events = stmt
        .query_map(params![start, end], |r| {
            Ok(PowerEvent {
                ts: r.get(0)?,
                kind: r.get(1)?,
                detail: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT ts, action, name, old_version, new_version FROM package_events
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts DESC, name ASC LIMIT 400",
        )
        .map_err(|e| e.to_string())?;
    let packages = stmt
        .query_map(params![start, end], |r| {
            Ok(PackageEvent {
                ts: r.get(0)?,
                action: r.get(1)?,
                name: r.get(2)?,
                old_version: r.get(3)?,
                new_version: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT ts, title, body, critical FROM notifications
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let notifications = stmt
        .query_map(params![start, end], |r| {
            Ok(NotificationRow {
                ts: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                critical: r.get::<_, i32>(3)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    Ok(LogReport {
        battery,
        network,
        power_events,
        packages,
        package_count: count("SELECT COUNT(*) FROM package_events WHERE ts >= ?1 AND ts < ?2"),
        notifications,
        kind: kind.to_string(),
        offset,
        start_ts: start,
        end_ts: end,
        totals: totals_between(&conn, start, end),
        buckets,
        apps,
        crashes,
        crash_count: count(
            "SELECT COUNT(*) FROM crashes WHERE kind = 'crash' AND time_us >= ?1 * 1000000 AND time_us < ?2 * 1000000",
        ),
        oom_count: count(
            "SELECT COUNT(*) FROM crashes WHERE kind = 'oom' AND time_us >= ?1 * 1000000 AND time_us < ?2 * 1000000",
        ),
        events,
        event_counts,
        actions,
        actions_failed: count("SELECT COUNT(*) FROM actions_log WHERE ok = 0 AND ts >= ?1 AND ts < ?2"),
        boots,
    })
}

#[derive(Serialize, Default)]
pub struct BatteryUsage {
    pub discharged_wh: f64,
    pub charged_wh: f64,
    pub on_battery_secs: i64,
    pub min_percent: f64,
    pub max_percent: f64,
    pub cycles_added: i64,
    pub days_recorded: i64,
}

#[derive(Serialize)]
pub struct NetworkUsage {
    pub iface: String,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
}

#[derive(Serialize)]
pub struct PowerEvent {
    pub ts: i64,
    /// lid_closed | lid_opened | suspend | resume | login | logout | power_key
    pub kind: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct PackageEvent {
    pub ts: i64,
    /// installed | upgraded | downgraded | removed
    pub action: String,
    pub name: String,
    pub old_version: String,
    pub new_version: String,
}

#[derive(Serialize)]
pub struct NotificationRow {
    pub ts: i64,
    pub title: String,
    pub body: String,
    pub critical: bool,
}

// ---- battery, network, power events, packages, notifications ----

/// What the battery looked like at the previous tick.
#[derive(Default)]
pub struct BatteryPrev {
    wh: Option<f64>,
}

/// Credits the change in stored energy since the last tick to today.
pub fn record_battery(db: &Db, ts: i64, tick_secs: i64, prev: &mut BatteryPrev) {
    let Some(battery) = crate::health::battery_health().into_iter().next() else {
        return;
    };
    let now_wh = battery.percent as f64 / 100.0 * battery.full_wh as f64;
    let delta = prev.wh.map(|before| now_wh - before).unwrap_or(0.0);
    prev.wh = Some(now_wh);
    // A jump bigger than any real charge rate means a resume from sleep or
    // a recalibration, not usage worth attributing.
    let delta = if delta.abs() > 5.0 { 0.0 } else { delta };
    let discharging = battery.status == "Discharging";
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO battery_daily (day, discharged_wh, charged_wh, on_battery_secs, min_percent, max_percent, cycle_count, full_wh)
         VALUES (date(?1, 'unixepoch', 'localtime'), ?2, ?3, ?4, ?5, ?5, ?6, ?7)
         ON CONFLICT(day) DO UPDATE SET
            discharged_wh = discharged_wh + excluded.discharged_wh,
            charged_wh = charged_wh + excluded.charged_wh,
            on_battery_secs = on_battery_secs + excluded.on_battery_secs,
            min_percent = MIN(min_percent, excluded.min_percent),
            max_percent = MAX(max_percent, excluded.max_percent),
            cycle_count = MAX(cycle_count, excluded.cycle_count),
            full_wh = excluded.full_wh",
        params![
            ts,
            (-delta).max(0.0),
            delta.max(0.0),
            if discharging { tick_secs } else { 0 },
            battery.percent as f64,
            battery.cycle_count.unwrap_or(0),
            battery.full_wh as f64,
        ],
    )
    .ok();
}

/// Adds the bytes each real network interface moved since the last tick.
/// Virtual interfaces (bridges, containers, tunnels) are skipped: their
/// traffic already passes through a physical one and would count twice.
pub fn record_network(db: &Db, ts: i64, prev: &mut std::collections::HashMap<String, (u64, u64)>) {
    let Ok(text) = std::fs::read_to_string("/proc/net/dev") else {
        return;
    };
    let mut deltas = Vec::new();
    for line in text.lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if !std::path::Path::new(&format!("/sys/class/net/{iface}/device")).exists() {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if f.len() < 9 {
            continue;
        }
        let (rx, tx) = (f[0], f[8]);
        if let Some((before_rx, before_tx)) = prev.insert(iface.to_string(), (rx, tx)) {
            // Counters restart from zero when an interface is re-created.
            let d_rx = rx.checked_sub(before_rx).unwrap_or(rx);
            let d_tx = tx.checked_sub(before_tx).unwrap_or(tx);
            if d_rx + d_tx > 0 {
                deltas.push((iface.to_string(), d_rx, d_tx));
            }
        }
    }
    if deltas.is_empty() {
        return;
    }
    let conn = db.conn.lock().unwrap();
    for (iface, rx, tx) in deltas {
        conn.execute(
            "INSERT INTO net_daily (day, iface, rx_bytes, tx_bytes)
             VALUES (date(?1, 'unixepoch', 'localtime'), ?2, ?3, ?4)
             ON CONFLICT(day, iface) DO UPDATE SET
                rx_bytes = rx_bytes + excluded.rx_bytes, tx_bytes = tx_bytes + excluded.tx_bytes",
            params![ts, iface, rx as i64, tx as i64],
        )
        .ok();
    }
}

fn classify_power_message(message: &str) -> Option<(&'static str, String)> {
    let kind = if message.starts_with("Lid closed") {
        "lid_closed"
    } else if message.starts_with("Lid opened") {
        "lid_opened"
    } else if message.starts_with("New session") {
        "login"
    } else if message.starts_with("Removed session") {
        "logout"
    } else if message.starts_with("Power key pressed") {
        "power_key"
    } else if message.contains("returned from sleep") || message.starts_with("System resumed") {
        "resume"
    } else if message.starts_with("Performing sleep operation")
        || message.starts_with("Entering sleep state")
        || message.starts_with("Suspending system")
    {
        "suspend"
    } else {
        return None;
    };
    Some((kind, message.trim_end_matches('.').to_string()))
}

/// Copies lid, sleep, wake and login events out of the journal.
pub fn ingest_power_events(db: &Db, first_run: bool) {
    let since = if first_run { "--since=-60d" } else { "--since=-2d" };
    let mut found = Vec::new();
    for selector in [["-u", "systemd-logind"], ["-t", "systemd-sleep"]] {
        let out = run_lossy(
            Command::new("journalctl").args([since, "-o", "json", "--no-pager", "-q"]).args(selector),
        );
        for line in out.lines() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let time_us: i64 = entry["__REALTIME_TIMESTAMP"]
                .as_str()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
            if let Some((kind, detail)) = entry["MESSAGE"].as_str().and_then(classify_power_message) {
                found.push((time_us, kind, detail));
            }
        }
    }
    let mut conn = db.conn.lock().unwrap();
    let Ok(tx) = conn.transaction() else {
        return;
    };
    for (time_us, kind, detail) in found.into_iter().filter(|e| e.0 > 0) {
        tx.execute(
            "INSERT OR IGNORE INTO power_events (time_us, kind, detail) VALUES (?1, ?2, ?3)",
            params![time_us, kind, detail],
        )
        .ok();
    }
    tx.commit().ok();
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-10-09T15:43:53+0530" -> unix seconds
fn parse_log_time(stamp: &str) -> Option<i64> {
    let (date, rest) = stamp.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, mo, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let sign_at = rest.rfind(['+', '-'])?;
    let (clock, zone) = rest.split_at(sign_at);
    let mut c = clock.split(':').map(|p| p.parse::<i64>());
    let (h, mi, sec) = (c.next()?.ok()?, c.next()?.ok()?, c.next()?.ok()?);
    let digits = zone[1..].replace(':', "");
    let offset = digits.get(0..2)?.parse::<i64>().ok()? * 3600 + digits.get(2..4)?.parse::<i64>().ok()? * 60;
    let offset = if zone.starts_with('-') { -offset } else { offset };
    Some(days_from_civil(y, mo, day) * 86400 + h * 3600 + mi * 60 + sec - offset)
}

/// "[2026-10-09T15:43:53+0530] [ALPM] upgraded linux (7.1.8-1 -> 7.1.9-2)"
fn parse_pacman_line(line: &str) -> Option<(i64, &'static str, String, String, String)> {
    let (stamp, rest) = line.strip_prefix('[')?.split_once("] [ALPM] ")?;
    let (action, rest) = rest.split_once(' ')?;
    let action = match action {
        "installed" => "installed",
        "upgraded" => "upgraded",
        "downgraded" => "downgraded",
        "removed" => "removed",
        _ => return None,
    };
    let (name, versions) = rest.split_once(" (")?;
    let versions = versions.trim_end_matches(')');
    let (old, new) = match versions.split_once(" -> ") {
        Some((old, new)) => (old.to_string(), new.to_string()),
        None if action == "removed" => (versions.to_string(), String::new()),
        None => (String::new(), versions.to_string()),
    };
    Some((parse_log_time(stamp)?, action, name.to_string(), old, new))
}

/// Reads what the package manager did since the last call (pacman's log).
pub fn ingest_packages(db: &Db) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open("/var/log/pacman.log") else {
        return;
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let offset: u64 = {
        let conn = db.conn.lock().unwrap();
        conn.query_row("SELECT value FROM logbook_state WHERE key = 'pacman_offset'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    // A smaller file than last time means the log was rotated: start over.
    let offset = if offset > size { 0 } else { offset };
    if offset == size {
        return;
    }
    let mut text = String::new();
    if file.seek(SeekFrom::Start(offset)).is_err() || file.read_to_string(&mut text).is_err() {
        return;
    }
    // Only whole lines are consumed; a line still being written waits.
    let consumed = text.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut conn = db.conn.lock().unwrap();
    let Ok(tx) = conn.transaction() else {
        return;
    };
    for (ts, action, name, old, new) in text[..consumed].lines().filter_map(parse_pacman_line) {
        tx.execute(
            "INSERT OR IGNORE INTO package_events (ts, action, name, old_version, new_version) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, action, name, old, new],
        )
        .ok();
    }
    tx.execute(
        "INSERT OR REPLACE INTO logbook_state (key, value) VALUES ('pacman_offset', ?1)",
        params![(offset + consumed as u64).to_string()],
    )
    .ok();
    tx.commit().ok();
}

/// Stores the desktop notifications Trace has sent since the last call.
pub fn flush_notifications(db: &Db) {
    let pending = crate::sys_util::take_notifications();
    if pending.is_empty() {
        return;
    }
    let conn = db.conn.lock().unwrap();
    for n in pending {
        conn.execute(
            "INSERT INTO notifications (ts, title, body, critical) VALUES (?1, ?2, ?3, ?4)",
            params![n.ts, n.title, n.body, n.critical as i32],
        )
        .ok();
    }
}

pub fn recent_notifications(db: &Db, limit: i64) -> Vec<NotificationRow> {
    let conn = db.conn.lock().unwrap();
    let Ok(mut stmt) = conn.prepare("SELECT ts, title, body, critical FROM notifications ORDER BY id DESC LIMIT ?1") else {
        return Vec::new();
    };
    stmt.query_map(params![limit], |r| {
        Ok(NotificationRow {
            ts: r.get(0)?,
            title: r.get(1)?,
            body: r.get(2)?,
            critical: r.get::<_, i32>(3)? != 0,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Writes the period's full report as JSON into the Downloads folder.
pub fn export(db: &Db, kind: &str, offset: i32) -> Result<String, String> {
    let report = report(db, kind, offset)?;
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or("No Downloads folder")?;
    let day: String = {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT date(?1, 'unixepoch', 'localtime')",
            params![report.start_ts],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?
    };
    let path = dir.join(format!("trace-log-{kind}-{day}.json"));
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_have_the_right_number_of_buckets_and_are_contiguous() {
        for (kind, buckets) in [("day", 24), ("week", 7), ("year", 12)] {
            let edges = period_edges(kind, 0).unwrap();
            assert_eq!(edges.len(), buckets + 1, "{kind}");
            assert!(edges.windows(2).all(|w| w[1] > w[0]), "{kind} edges ascend");
            // The previous period ends exactly where this one starts.
            let previous = period_edges(kind, 1).unwrap();
            assert_eq!(*previous.last().unwrap(), edges[0], "{kind} is contiguous");
        }
        let month = period_edges("month", 0).unwrap();
        assert!((28..=31).contains(&(month.len() - 1)));
        assert!(period_edges("decade", 0).is_none());
    }

    #[test]
    fn parses_pacman_log_lines() {
        let (ts, action, name, old, new) =
            parse_pacman_line("[2026-10-09T15:43:53+0530] [ALPM] upgraded linux (7.1.8.arch1-1 -> 7.1.9.arch1-2)").unwrap();
        assert_eq!((action, name.as_str(), old.as_str(), new.as_str()), ("upgraded", "linux", "7.1.8.arch1-1", "7.1.9.arch1-2"));
        // 2026-10-09 10:13:53 UTC
        assert_eq!(ts, 1_791_540_833);
        let (_, action, name, old, new) =
            parse_pacman_line("[2026-01-02T03:04:05-0800] [ALPM] installed htop (3.4.1-1)").unwrap();
        assert_eq!((action, name.as_str(), old.as_str(), new.as_str()), ("installed", "htop", "", "3.4.1-1"));
        let (_, action, _, old, new) =
            parse_pacman_line("[2026-01-02T03:04:05+0000] [ALPM] removed htop (3.4.1-1)").unwrap();
        assert_eq!((action, old.as_str(), new.as_str()), ("removed", "3.4.1-1", ""));
        assert!(parse_pacman_line("[2026-10-09T15:43:53+0530] [ALPM] running 'x.hook'...").is_none());
        assert!(parse_pacman_line("[2026-10-09T15:43:53+0530] [PACMAN] Running 'pacman -Syu'").is_none());
    }

    #[test]
    fn classifies_power_messages() {
        assert_eq!(classify_power_message("Lid closed.").unwrap().0, "lid_closed");
        assert_eq!(classify_power_message("New session 3 of user adi.").unwrap().0, "login");
        assert_eq!(classify_power_message("Performing sleep operation 'suspend'...").unwrap().0, "suspend");
        assert_eq!(classify_power_message("System returned from sleep operation 'suspend'.").unwrap().0, "resume");
        assert!(classify_power_message("Watching system buttons on /dev/input/event3").is_none());
    }

    #[test]
    fn current_period_contains_now() {
        let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
        for kind in ["day", "week", "month", "year"] {
            let edges = period_edges(kind, 0).unwrap();
            assert!(edges[0] <= now && now < *edges.last().unwrap(), "{kind}");
        }
    }
}
