//! Process details, putting every setting back, frame-rate history per
//! game, a health report to share, and alerts through ntfy.

use crate::control_process::ActionResult;
use crate::db::Db;
use crate::sys_util::{err, ok, read_trimmed, run_lossy};
use rusqlite::params;
use serde::Serialize;
use std::fs;
use std::process::Command;

pub fn init(db: &Db) {
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS game_sessions (file TEXT PRIMARY KEY, ts INTEGER NOT NULL, game TEXT NOT NULL, minutes REAL NOT NULL, avg_fps REAL NOT NULL, low_fps REAL NOT NULL);",
        )
        .ok();
}

// ---- process details ----

#[derive(Serialize)]
pub struct ProcessDetail {
    pub pid: u32,
    pub name: String,
    pub command: String,
    pub exe: String,
    pub folder: String,
    pub user: String,
    pub state: String,
    pub threads: u32,
    pub started_secs_ago: u64,
    pub ram_bytes: u64,
    /// systemd unit or scope that owns it.
    pub unit: String,
    /// From this process up to the first one: why it is running.
    pub parents: Vec<(u32, String)>,
    pub children: Vec<(u32, String)>,
    pub open_files: Vec<String>,
    pub open_file_count: usize,
    pub connections: Vec<String>,
    /// Only readable for your own processes.
    pub environment: Vec<String>,
}

fn status_field(pid: u32, key: &str) -> String {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_string()))
        .unwrap_or_default()
}

fn proc_name(pid: u32) -> String {
    read_trimmed(&format!("/proc/{pid}/comm")).unwrap_or_default()
}

/// Everything worth knowing about one process, read from /proc.
pub fn process_detail(pid: u32) -> Result<ProcessDetail, String> {
    let base = format!("/proc/{pid}");
    if !std::path::Path::new(&base).exists() {
        return Err("That process has finished".into());
    }
    let command = fs::read(format!("{base}/cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ").trim().to_string()).unwrap_or_default();
    let uid: u32 = status_field(pid, "Uid:").split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
    let user = run_lossy(Command::new("id").args(["-nu", &uid.to_string()]));

    let mut parents = Vec::new();
    let mut current = pid;
    for _ in 0..30 {
        let ppid: u32 = status_field(current, "PPid:").parse().unwrap_or(0);
        if ppid == 0 {
            break;
        }
        parents.push((ppid, proc_name(ppid)));
        current = ppid;
    }
    let children: Vec<(u32, String)> = fs::read_to_string(format!("{base}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|c| c.parse::<u32>().ok())
        .map(|c| (c, proc_name(c)))
        .collect();

    let mut open_files: Vec<String> = fs::read_dir(format!("{base}/fd"))
        .map(|d| d.flatten().filter_map(|e| fs::read_link(e.path()).ok()).map(|t| t.to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let open_file_count = open_files.len();
    open_files.retain(|f| f.starts_with('/') && !f.starts_with("/dev/") && !f.starts_with("/proc/"));
    open_files.sort();
    open_files.dedup();
    open_files.truncate(60);

    let pid_tag = format!("pid={pid},");
    let connections: Vec<String> = run_lossy(Command::new("ss").args(["-tunapH"]))
        .lines()
        .filter(|l| l.contains(&pid_tag))
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            format!("{} {} {} → {}", f.first().unwrap_or(&""), f.get(1).unwrap_or(&""), f.get(4).unwrap_or(&""), f.get(5).unwrap_or(&""))
        })
        .take(40)
        .collect();

    let environment: Vec<String> = fs::read(format!("{base}/environ"))
        .map(|b| String::from_utf8_lossy(&b).split('\0').filter(|v| !v.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    // Values that look like secrets are not shown, only that they are set.
    let environment = environment
        .into_iter()
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if ["TOKEN", "SECRET", "PASSWORD", "PASS", "KEY", "CREDENTIAL"].iter().any(|w| k.to_uppercase().contains(w)) => format!("{k}=(hidden)"),
            _ => kv,
        })
        .collect();

    let stat = fs::read_to_string(format!("{base}/stat")).unwrap_or_default();
    let fields: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
    let start_ticks: u64 = fields.get(19).and_then(|v| v.parse().ok()).unwrap_or(0);
    let uptime: f64 = fs::read_to_string("/proc/uptime").ok().and_then(|t| t.split_whitespace().next()?.parse().ok()).unwrap_or(0.0);
    let unit = fs::read_to_string(format!("{base}/cgroup")).unwrap_or_default().lines().next().and_then(|l| l.rsplit('/').next()).unwrap_or("").to_string();

    Ok(ProcessDetail {
        pid,
        name: proc_name(pid),
        command,
        exe: fs::read_link(format!("{base}/exe")).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        folder: fs::read_link(format!("{base}/cwd")).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        user,
        state: status_field(pid, "State:"),
        threads: status_field(pid, "Threads:").parse().unwrap_or(0),
        started_secs_ago: (uptime - start_ticks as f64 / 100.0).max(0.0) as u64,
        ram_bytes: status_field(pid, "VmRSS:").split_whitespace().next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 1024,
        unit,
        parents,
        children,
        open_files,
        open_file_count,
        connections,
        environment,
    })
}

// ---- putting everything back ----

#[derive(Serialize)]
pub struct ChangedSetting {
    pub id: String,
    pub title: String,
    pub now: String,
    pub original: String,
}

/// Every catalogue setting whose value differs from what Trace first saw.
pub fn changed_settings(db: &Db) -> Vec<ChangedSetting> {
    crate::tweaks::list(db)
        .into_iter()
        .filter(|t| t.changed && !t.original.is_empty())
        .map(|t| ChangedSetting { id: t.id, title: t.title, now: t.value, original: t.original })
        .collect()
}

/// Puts every changed setting back to its original value in one go (one password prompt).
pub fn put_everything_back(db: &Db) -> Vec<crate::tweaks::ChangeResult> {
    let changes: Vec<crate::tweaks::Change> = changed_settings(db).into_iter().map(|c| crate::tweaks::Change { id: c.id, value: c.original }).collect();
    if changes.is_empty() {
        return Vec::new();
    }
    // Not keeping them any more either: the point is to be back where things started.
    crate::tweaks::apply(db, &changes, "put everything back", Some(false))
}

// ---- frame rate per game ----

#[derive(Serialize)]
pub struct GameSession {
    pub ts: i64,
    pub game: String,
    pub minutes: f64,
    pub avg_fps: f64,
    pub low_fps: f64,
}

/// (minutes played, average fps, 1%-low fps) from one MangoHud log.
fn summarise(text: &str) -> Option<(f64, f64, f64)> {
    let mut lines = text.lines();
    lines.position(|l| l.starts_with("fps,"))?;
    let rows: Vec<(f64, f64)> = lines
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            Some((f.first()?.parse().ok()?, f.last()?.parse().ok()?))
        })
        .filter(|(fps, _): &(f64, f64)| fps.is_finite() && *fps > 0.0)
        .collect();
    if rows.len() < 4 {
        return None;
    }
    let mut fps: Vec<f64> = rows.iter().map(|r| r.0).collect();
    let avg = fps.iter().sum::<f64>() / fps.len() as f64;
    fps.sort_by(f64::total_cmp);
    // "1% low": the average of the slowest hundredth of the samples.
    let slowest = (fps.len() / 100).max(1);
    let low = fps[..slowest].iter().sum::<f64>() / slowest as f64;
    // The last column is the time since logging started, in nanoseconds.
    let minutes = rows.last()?.1 / 1e9 / 60.0;
    Some((minutes, avg, low))
}

/// Records games whose log hasn't grown for two minutes (they have ended).
pub fn ingest_games(db: &Db) {
    let Ok(dir) = fs::read_dir(crate::overlay::fps_folder()) else {
        return;
    };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".csv") || name.ends_with("_summary.csv") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.modified().ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age.as_secs() < 120) {
            continue;
        }
        let known: bool = db.conn.lock().unwrap().query_row("SELECT 1 FROM game_sessions WHERE file = ?1", params![name], |_| Ok(true)).unwrap_or(false);
        if known {
            continue;
        }
        let Some((minutes, avg, low)) = fs::read_to_string(entry.path()).ok().and_then(|t| summarise(&t)) else {
            continue;
        };
        // "Game Name_2026-10-19_19-58-21.csv"
        let game = name.rsplitn(3, '_').nth(2).unwrap_or(&name).trim_end_matches(".exe").to_string();
        let ts = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        db.conn
            .lock()
            .unwrap()
            .execute("INSERT OR IGNORE INTO game_sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![name, ts, game, minutes, avg, low])
            .ok();
    }
}

pub fn game_sessions(db: &Db) -> Vec<GameSession> {
    let conn = db.conn.lock().unwrap();
    conn.prepare("SELECT ts, game, minutes, avg_fps, low_fps FROM game_sessions ORDER BY ts DESC LIMIT 40")
        .and_then(|mut s| {
            s.query_map([], |r| Ok(GameSession { ts: r.get(0)?, game: r.get(1)?, minutes: r.get(2)?, avg_fps: r.get(3)?, low_fps: r.get(4)? }))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default()
}

// ---- a health report to send someone ----

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub struct ReportParts {
    pub score: u32,
    pub grade: String,
    pub issues: Vec<(String, String, String)>,
    pub rows: Vec<(String, String)>,
    pub failed: Vec<String>,
    pub errors: Vec<String>,
}

/// Writes a self-contained HTML page (no scripts, nothing fetched) to Downloads.
pub fn write_report(parts: &ReportParts) -> Result<String, String> {
    let platform = crate::platform::get();
    let mut html = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><title>Machine health report</title><style>body{font:14px/1.5 system-ui,sans-serif;max-width:860px;margin:32px auto;padding:0 16px;color:#1d2230}h1{margin-bottom:0}h2{margin-top:28px;border-bottom:1px solid #ddd}table{border-collapse:collapse;width:100%}td{padding:4px 8px;border-bottom:1px solid #eee;vertical-align:top}td:first-child{color:#555;width:35%}.critical{color:#b3261e}.warn{color:#8a5a00}.muted{color:#666}code{font-size:12px}</style></head><body>");
    html += &format!("<h1>Machine health report</h1><p class=\"muted\">{} · {} · made by Trace on {}</p>", esc(&platform.distro), esc(&platform.desktop), esc(&run_lossy(Command::new("date").arg("+%Y-%m-%d %H:%M"))));
    html += &format!("<h2>Score: {} / 100 ({})</h2>", parts.score, esc(&parts.grade));
    if parts.issues.is_empty() {
        html += "<p>No issues found.</p>";
    }
    for (severity, title, detail) in &parts.issues {
        html += &format!("<p class=\"{}\"><b>{}</b><br><span class=\"muted\">{}</span></p>", esc(severity), esc(title), esc(detail));
    }
    html += "<h2>This machine</h2><table>";
    for (k, v) in &parts.rows {
        html += &format!("<tr><td>{}</td><td>{}</td></tr>", esc(k), esc(v));
    }
    html += "</table>";
    if !parts.failed.is_empty() {
        html += "<h2>Failed services</h2><ul>";
        for f in &parts.failed {
            html += &format!("<li><code>{}</code></li>", esc(f));
        }
        html += "</ul>";
    }
    if !parts.errors.is_empty() {
        html += "<h2>Recent errors (this boot)</h2><ul>";
        for e in parts.errors.iter().take(40) {
            html += &format!("<li><code>{}</code></li>", esc(e));
        }
        html += "</ul>";
    }
    html += "<p class=\"muted\">Contains no passwords, file contents or browsing history. Check it before sending: error messages can mention file or device names.</p></body></html>";
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("No Downloads folder")?;
    let path = dir.join(format!("machine-health-{}.html", run_lossy(Command::new("date").arg("+%Y-%m-%d"))));
    fs::write(&path, html).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

// ---- ntfy ----

/// Only https addresses on a topic path, so nothing odd reaches curl.
pub fn valid_ntfy(url: &str) -> bool {
    url.starts_with("https://") && url.len() < 200 && url[8..].contains('/') && url.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
}

/// Sends an alert to the ntfy topic you set, in the background.
pub fn send_ntfy(url: &str, title: &str, body: &str, critical: bool) {
    if !valid_ntfy(url) {
        return;
    }
    let (url, title, body) = (url.to_string(), title.chars().filter(|c| !c.is_control()).collect::<String>(), body.to_string());
    std::thread::spawn(move || {
        Command::new("curl")
            .args(["-s", "-m", "10", "-H", &format!("Title: {title}"), "-H", if critical { "Priority: high" } else { "Priority: default" }, "-d", &body, &url])
            .output()
            .ok();
    });
}

pub fn test_ntfy(url: &str) -> ActionResult {
    if !valid_ntfy(url) {
        return err("Use an https address with a topic, e.g. https://ntfy.sh/your-private-topic");
    }
    let out = Command::new("curl").args(["-s", "-m", "10", "-o", "/dev/null", "-w", "%{http_code}", "-H", "Title: Trace", "-d", "Test alert from Trace", url]).output();
    match out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()) {
        Ok(code) if code.starts_with('2') => ok("Sent. It should arrive on every device subscribed to that topic."),
        Ok(code) => err(format!("The server answered {code}")),
        Err(e) => err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_a_mangohud_log() {
        let mut log = String::from("os,cpu\nArch,AMD\nfps,frametime,cpu_load,elapsed\n");
        for i in 0..200 {
            let fps = if i == 7 { 20.0 } else { 60.0 };
            log += &format!("{fps},16.6,10,{}\n", (i + 1) as u64 * 500_000_000);
        }
        let (minutes, avg, low) = summarise(&log).unwrap();
        assert!((minutes - 200.0 * 0.5 / 60.0).abs() < 0.01);
        assert!((avg - 59.8).abs() < 0.01);
        assert_eq!(low, 40.0, "the slowest 1% (two samples here) catches the stutter");
        assert!(summarise("fps,frametime\n").is_none());
    }

    #[test]
    fn looks_at_this_process_and_hides_secret_looking_values() {
        // A process's environment is fixed when it starts, so look at a child started with one.
        let mut child = Command::new("sleep").arg("5").env("TRACE_TEST_API_TOKEN", "shh").env("TRACE_TEST_COLOUR", "blue").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let detail = process_detail(child.id()).unwrap();
        child.kill().ok();
        child.wait().ok();
        assert!(detail.threads >= 1 && detail.command.starts_with("sleep") && !detail.parents.is_empty());
        assert!(detail.environment.iter().any(|v| v == "TRACE_TEST_API_TOKEN=(hidden)"), "secret-looking values are hidden");
        assert!(detail.environment.iter().any(|v| v == "TRACE_TEST_COLOUR=blue"), "ordinary values are shown");
        assert_eq!(detail.parents[0].0, std::process::id(), "its parent is this test");
        assert!(process_detail(u32::MAX - 1).is_err());
    }

    #[test]
    fn ntfy_addresses_are_checked() {
        assert!(valid_ntfy("https://ntfy.sh/my-private-topic-123"));
        assert!(!valid_ntfy("http://ntfy.sh/topic"));
        assert!(!valid_ntfy("https://ntfy.sh"));
        assert!(!valid_ntfy("https://ntfy.sh/a b"));
        assert!(!valid_ntfy("https://ntfy.sh/a;rm"));
    }

    #[test]
    fn the_report_escapes_what_it_is_given() {
        let parts = ReportParts { score: 80, grade: "Good".into(), issues: vec![("warn".into(), "<script>x</script>".into(), "d".into())], rows: vec![("k".into(), "v & w".into())], failed: vec![], errors: vec![] };
        let path = write_report(&parts).unwrap();
        let html = fs::read_to_string(&path).unwrap();
        assert!(html.contains("&lt;script&gt;") && !html.contains("<script>x") && html.contains("v &amp; w"));
        fs::remove_file(path).ok();
    }
}
