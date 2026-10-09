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

    // First run after upgrading: seed the hourly table from the raw samples
    // already on disk so the logbook doesn't start empty.
    let empty: bool = conn
        .query_row("SELECT NOT EXISTS (SELECT 1 FROM hourly_stats)", [], |r| r.get(0))
        .unwrap_or(false);
    if empty {
        conn.execute(
            "INSERT OR IGNORE INTO hourly_stats
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
}

/// Folds one 10-second sample into its local-time hour.
pub fn record_sample(db: &Db, ts: i64, s: &Sample) {
    let hour_ts = ts - ((ts + utc_offset_secs()).rem_euclid(3600));
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO hourly_stats VALUES (?1, 1, ?2, ?2, ?3, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(hour_ts) DO UPDATE SET
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
            s.io_stall
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
}

fn totals_between(conn: &rusqlite::Connection, start: i64, end: i64) -> Totals {
    conn.query_row(
        "SELECT COALESCE(SUM(samples), 0), COALESCE(SUM(cpu_sum), 0), COALESCE(MAX(cpu_max), 0),
                COALESCE(SUM(ram_sum), 0), COALESCE(MAX(ram_max), 0), COALESCE(MAX(swap_max_bytes), 0),
                COALESCE(SUM(cpu_stall_sum), 0), COALESCE(SUM(mem_stall_sum), 0), COALESCE(SUM(io_stall_sum), 0)
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

    Ok(LogReport {
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
    fn current_period_contains_now() {
        let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
        for kind in ["day", "week", "month", "year"] {
            let edges = period_edges(kind, 0).unwrap();
            assert!(edges[0] <= now && now < *edges.last().unwrap(), "{kind}");
        }
    }
}
