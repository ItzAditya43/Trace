//! Connection doctor, listening ports, per-project build junk, sleep
//! sessions, battery capacity history and data budgets.

use crate::control_process::ActionResult;
use crate::db::Db;
use crate::sys_util::{err, notify, ok, run_lossy};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
use std::time::Instant;

// ---- connection doctor ----

#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize)]
pub struct Diagnosis {
    pub verdict: String,
    pub checks: Vec<Check>,
}

/// Average round-trip time of three pings, or None when nothing came back.
fn ping_ms(host: &str) -> Option<f32> {
    let out = run_lossy(Command::new("ping").args(["-c", "3", "-i", "0.25", "-W", "2", "-q", host]));
    // "rtt min/avg/max/mdev = 1.2/1.5/1.9/0.2 ms"
    out.lines()
        .find(|l| l.contains("min/avg/max"))?
        .split('=')
        .nth(1)?
        .trim()
        .split('/')
        .nth(1)?
        .parse()
        .ok()
}

/// Works outward one hop at a time — link, router, internet, names — so the
/// verdict can say where the chain breaks rather than just "offline".
pub fn connection_doctor() -> Diagnosis {
    let mut checks = Vec::new();
    let route = run_lossy(Command::new("ip").args(["-4", "route", "show", "default"]));
    let tokens: Vec<&str> = route.lines().next().unwrap_or("").split_whitespace().collect();
    let after = |key: &str| tokens.iter().position(|t| *t == key).and_then(|i| tokens.get(i + 1)).copied();
    let (gateway, device) = (after("via"), after("dev"));

    checks.push(Check {
        name: "Network link".into(),
        ok: device.is_some(),
        detail: match device {
            Some(d) => format!("connected through {d}"),
            None => "no network is connected (no default route)".into(),
        },
    });
    let Some(gateway) = gateway else {
        return Diagnosis { verdict: "This machine isn't connected to any network.".into(), checks };
    };

    let router = ping_ms(gateway);
    checks.push(Check {
        name: "Your router".into(),
        ok: router.is_some(),
        detail: match router {
            Some(ms) => format!("{gateway} answers in {ms:.0} ms"),
            None => format!("{gateway} does not answer"),
        },
    });
    let internet = ping_ms("1.1.1.1");
    checks.push(Check {
        name: "The internet".into(),
        ok: internet.is_some(),
        detail: match internet {
            Some(ms) => format!("reachable, {ms:.0} ms round trip"),
            None => "an outside address (1.1.1.1) does not answer".into(),
        },
    });
    let started = Instant::now();
    let resolved = !run_lossy(Command::new("getent").args(["ahostsv4", "archlinux.org"])).is_empty();
    let lookup_ms = started.elapsed().as_millis();
    checks.push(Check {
        name: "Looking up names (DNS)".into(),
        ok: resolved,
        detail: if resolved { format!("works, took {lookup_ms} ms") } else { "names can't be looked up".into() },
    });

    let verdict = match (router, internet, resolved) {
        (None, None, _) => "The problem is between this machine and the router: Wi-Fi signal, cable, or the router itself.",
        (Some(_), None, _) => "Your router is fine but the internet beyond it isn't answering — this is your provider or the router's own connection.",
        (_, Some(_), false) => "The internet works but names can't be looked up. Change the DNS server under Tweaks → Network.",
        (Some(r), Some(_), true) if r > 50.0 => "Everything works, but the hop to your router is slow — on Wi-Fi that means a weak or crowded signal.",
        (_, Some(i), true) if i > 150.0 => "Everything works, but the internet is slow to answer — the delay is beyond your router.",
        (_, Some(_), true) if lookup_ms > 800 => "Everything works, but looking up names is slow, which makes pages start late. A different DNS server (Tweaks → Network) may help.",
        _ => "Everything checks out: link, router, internet and name lookups are all healthy.",
    };
    Diagnosis { verdict: verdict.into(), checks }
}

// ---- listening ports ----

#[derive(Serialize)]
pub struct Listener {
    pub port: u16,
    /// Reachable from other machines (not bound to this machine only).
    pub exposed: bool,
    pub protocol: String,
    pub pid: u32,
    pub process: String,
    /// The folder the program was started in — usually the project.
    pub folder: String,
}

pub fn listeners() -> Vec<Listener> {
    let mut found: Vec<Listener> = Vec::new();
    for (flag, protocol) in [("-ltnpH", "TCP"), ("-lunpH", "UDP")] {
        for line in run_lossy(Command::new("ss").arg(flag)).lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let Some(local) = f.get(3) else {
                continue;
            };
            let Some((address, port)) = local.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            let field = |key: &str| {
                let start = line.find(key)? + key.len();
                let rest = &line[start..];
                Some(&rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())])
            };
            let pid: u32 = field("pid=").and_then(|p| p.parse().ok()).unwrap_or(0);
            if found.iter().any(|l| l.port == port && l.pid == pid && l.protocol == protocol) {
                continue;
            }
            let folder = fs::read_link(format!("/proc/{pid}/cwd"))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            found.push(Listener {
                port,
                exposed: !(address.starts_with("127.") || address == "[::1]" || address.starts_with("[::ffff:127.")),
                protocol: protocol.into(),
                pid,
                process: line
                    .split("((\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap_or(if pid == 0 { "a system service" } else { "?" })
                    .to_string(),
                folder,
            });
        }
    }
    found.sort_by_key(|l| (l.port, l.pid));
    found
}

// ---- build junk inside a project ----

/// Folders that are safe to delete because a build or install recreates them.
const JUNK: &[(&str, &str)] = &[
    ("node_modules", "installed JavaScript packages (npm install brings them back)"),
    ("target", "Rust build output"),
    (".venv", "Python virtual environment"),
    ("venv", "Python virtual environment"),
    ("__pycache__", "Python bytecode cache"),
    (".pytest_cache", "test cache"),
    (".mypy_cache", "type-checker cache"),
    (".ruff_cache", "linter cache"),
    ("dist", "build output"),
    ("build", "build output"),
    (".next", "Next.js build cache"),
    (".nuxt", "Nuxt build cache"),
    (".turbo", "Turborepo cache"),
    (".parcel-cache", "Parcel cache"),
    (".gradle", "Gradle cache"),
    (".cache", "tool cache"),
];

#[derive(Serialize)]
pub struct JunkDir {
    pub path: String,
    pub name: String,
    pub what: String,
    pub bytes: u64,
}

fn dir_bytes(path: &std::path::Path) -> u64 {
    run_lossy(Command::new("du").arg("-sb").arg(path))
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Rebuildable folders up to three levels inside a project (never descending
/// into one that was already found, or into .git).
pub fn project_junk(project: &str) -> Vec<JunkDir> {
    fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<JunkDir>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !entry.file_type().is_ok_and(|t| t.is_dir()) || name == ".git" {
                continue;
            }
            if let Some((_, what)) = JUNK.iter().find(|(junk, _)| *junk == name) {
                // "build"/"dist"/"target" only count when the folder is not tracked by git.
                let bytes = dir_bytes(&path);
                if bytes > 1024 * 1024 {
                    out.push(JunkDir { path: path.to_string_lossy().to_string(), name, what: what.to_string(), bytes });
                }
            } else if depth < 3 {
                walk(&path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(std::path::Path::new(project), 0, &mut out);
    // Anything git tracks is source, not junk — whatever its name.
    out.retain(|j| {
        let tracked = run_lossy(
            Command::new("git").arg("-C").arg(project).args(["ls-files", "--error-unmatch", "--"]).arg(&j.path),
        );
        tracked.is_empty()
    });
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

/// Deletes folders previously listed by `project_junk`. Each path is checked
/// again here: it must sit inside the project and be one of the known names.
pub fn clean_junk(project: &str, paths: &[String]) -> ActionResult {
    let Ok(root) = fs::canonicalize(project) else {
        return err("That project folder no longer exists");
    };
    let mut freed = 0u64;
    let mut removed = 0;
    for path in paths {
        let Ok(full) = fs::canonicalize(path) else {
            continue;
        };
        let name = full.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let inside = full.starts_with(&root) && full != root;
        let known = JUNK.iter().any(|(junk, _)| *junk == name);
        // A symlink was resolved by canonicalize, so a link pointing outside fails `inside`.
        if !inside || !known || !full.is_dir() {
            return err(format!("{path} is not a rebuildable folder inside this project — nothing more was deleted"));
        }
        let bytes = dir_bytes(&full);
        if fs::remove_dir_all(&full).is_ok() {
            freed += bytes;
            removed += 1;
        }
    }
    ok(format!("Deleted {removed} folder(s), freeing {:.2} GB", freed as f64 / 1024.0 / 1024.0 / 1024.0))
}

// ---- sleep sessions and battery capacity ----

pub fn init(db: &Db) {
    let conn = db.conn.lock().unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sleep_sessions (
            start_ts INTEGER PRIMARY KEY,
            end_ts INTEGER NOT NULL,
            percent_before REAL NOT NULL,
            percent_after REAL NOT NULL,
            on_battery INTEGER NOT NULL
        );",
    )
    .ok();
}

#[derive(Default)]
pub struct SleepWatch {
    last_tick: Option<(i64, f32, bool)>,
}

/// Called every tick with the wall clock and battery. A gap far longer than
/// the tick interval means the machine was asleep in between.
pub fn watch_sleep(db: &Db, watch: &mut SleepWatch, now: i64, percent: f32, on_battery: bool, tick_secs: i64) {
    if let Some((before_ts, before_percent, was_on_battery)) = watch.last_tick {
        if now - before_ts > tick_secs * 6 {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO sleep_sessions VALUES (?1, ?2, ?3, ?4, ?5)",
                params![before_ts, now, before_percent, percent, was_on_battery as i32],
            )
            .ok();
        }
    }
    watch.last_tick = Some((now, percent, on_battery));
}

#[derive(Serialize)]
pub struct SleepSession {
    pub start_ts: i64,
    pub end_ts: i64,
    pub percent_before: f32,
    pub percent_after: f32,
    pub on_battery: bool,
}

#[derive(Serialize)]
pub struct CapacityPoint {
    pub day: String,
    pub full_wh: f32,
    pub cycle_count: i64,
}

#[derive(Serialize)]
pub struct BatteryCare {
    pub sleeps: Vec<SleepSession>,
    pub capacity: Vec<CapacityPoint>,
}

pub fn battery_care(db: &Db) -> BatteryCare {
    let conn = db.conn.lock().unwrap();
    let sleeps = conn
        .prepare("SELECT start_ts, end_ts, percent_before, percent_after, on_battery FROM sleep_sessions ORDER BY start_ts DESC LIMIT 20")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(SleepSession {
                    start_ts: r.get(0)?,
                    end_ts: r.get(1)?,
                    percent_before: r.get(2)?,
                    percent_after: r.get(3)?,
                    on_battery: r.get::<_, i32>(4)? != 0,
                })
            })
            .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    let capacity = conn
        .prepare("SELECT day, full_wh, cycle_count FROM battery_daily WHERE full_wh > 0 ORDER BY day ASC")
        .and_then(|mut s| {
            s.query_map([], |r| Ok(CapacityPoint { day: r.get(0)?, full_wh: r.get(1)?, cycle_count: r.get(2)? }))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    BatteryCare { sleeps, capacity }
}

// ---- data budgets ----

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DataBudget {
    pub iface: String,
    pub monthly_gb: f64,
}

#[derive(Serialize)]
pub struct DataUsage {
    pub iface: String,
    pub used_bytes: i64,
    pub budget_bytes: Option<i64>,
}

/// Bytes each interface moved since the first of this month.
pub fn data_usage(db: &Db, budgets: &[DataBudget]) -> Vec<DataUsage> {
    let conn = db.conn.lock().unwrap();
    conn.prepare(
        "SELECT iface, SUM(rx_bytes + tx_bytes) FROM net_daily
         WHERE day >= date('now', 'localtime', 'start of month') GROUP BY iface ORDER BY 2 DESC",
    )
    .and_then(|mut s| {
        s.query_map([], |r| {
            let iface: String = r.get(0)?;
            Ok(DataUsage {
                budget_bytes: budgets.iter().find(|b| b.iface == iface).map(|b| (b.monthly_gb * 1e9) as i64),
                used_bytes: r.get(1)?,
                iface,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
    })
    .unwrap_or_default()
}

/// Warns once at 80% and once at 100% of a budget each month.
pub fn check_budgets(db: &Db, budgets: &[DataBudget]) {
    if budgets.is_empty() {
        return;
    }
    for usage in data_usage(db, budgets) {
        let Some(budget) = usage.budget_bytes.filter(|b| *b > 0) else {
            continue;
        };
        let share = usage.used_bytes as f64 / budget as f64;
        let level = if share >= 1.0 { 100 } else if share >= 0.8 { 80 } else { continue };
        let key = format!("budget:{}:{level}", usage.iface);
        let conn = db.conn.lock().unwrap();
        let month: String = conn.query_row("SELECT strftime('%Y-%m', 'now', 'localtime')", [], |r| r.get(0)).unwrap_or_default();
        let already: Option<String> = conn
            .query_row("SELECT value FROM logbook_state WHERE key = ?1", params![key], |r| r.get(0))
            .ok();
        if already.as_deref() == Some(month.as_str()) {
            continue;
        }
        conn.execute("INSERT OR REPLACE INTO logbook_state (key, value) VALUES (?1, ?2)", params![key, month]).ok();
        drop(conn);
        notify(
            &format!("Data budget: {level}% used on {}", usage.iface),
            &format!("{:.1} GB of {:.1} GB this month.", usage.used_bytes as f64 / 1e9, budget as f64 / 1e9),
            level == 100,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_cleaner_refuses_anything_outside_the_project_or_not_on_the_list() {
        let base = std::env::temp_dir().join(format!("trace-junk-{}", std::process::id()));
        let project = base.join("project");
        fs::create_dir_all(project.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(base.join("outside/node_modules")).unwrap();
        fs::write(project.join("node_modules/pkg/big"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();
        std::os::unix::fs::symlink(base.join("outside/node_modules"), project.join("dist")).unwrap();
        let p = project.to_string_lossy().to_string();

        let found = project_junk(&p);
        assert_eq!(found.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(), ["node_modules"]);

        // Source folders, paths outside, and links that lead outside are all refused.
        for bad in [project.join("src"), base.join("outside/node_modules"), project.join("dist"), project.clone()] {
            assert!(!clean_junk(&p, &[bad.to_string_lossy().to_string()]).ok, "{bad:?}");
        }
        assert!(base.join("outside/node_modules").exists() && project.join("src/main.rs").exists());

        assert!(clean_junk(&p, &[found[0].path.clone()]).ok);
        assert!(!project.join("node_modules").exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn a_long_gap_between_ticks_is_recorded_as_sleep() {
        let db = Db { conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        init(&db);
        db.conn.lock().unwrap().execute_batch("CREATE TABLE battery_daily (day TEXT, full_wh REAL, cycle_count INTEGER);").unwrap();
        let mut watch = SleepWatch::default();
        watch_sleep(&db, &mut watch, 1000, 80.0, true, 10);
        watch_sleep(&db, &mut watch, 1010, 80.0, true, 10);
        assert!(battery_care(&db).sleeps.is_empty());
        watch_sleep(&db, &mut watch, 1010 + 7 * 3600, 74.0, true, 10);
        let sleeps = battery_care(&db).sleeps;
        assert_eq!(sleeps.len(), 1);
        assert_eq!((sleeps[0].start_ts, sleeps[0].percent_before, sleeps[0].percent_after), (1010, 80.0, 74.0));
    }
}
