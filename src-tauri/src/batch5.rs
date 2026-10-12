//! Things that act: maintenance run, project work mode, update briefing,
//! duplicate and forgotten files, backups, game mode, group budgets,
//! per-app sound, USB power, one-off full charge and monitor colour.

use crate::control_process::ActionResult;
use crate::db::Db;
use crate::sys_util::{data_dir, err, has_tool, notify, ok, read_num, read_trimmed, run, run_lossy, run_privileged};
use crate::tweaks::Change;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

fn load_json<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    fs::read_to_string(data_dir().join(name)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_json<T: Serialize>(name: &str, value: &T) {
    fs::write(data_dir().join(name), serde_json::to_string_pretty(value).unwrap_or_default()).ok();
}

/// Characters that are safe inside single quotes in a root shell script.
fn safe_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains("..") && path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@".contains(c))
}

fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn().map(|_| ())
}

fn du(path: &str) -> u64 {
    run_lossy(Command::new("du").args(["-sb", path])).split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

// ---- maintenance ----

#[derive(Serialize, Deserialize, Default)]
struct MaintenanceState {
    last_run: i64,
    last_reminder: i64,
    last_report: Vec<String>,
}

#[derive(Serialize)]
pub struct MaintenanceStatus {
    pub last_run: i64,
    pub last_report: Vec<String>,
    pub journal_bytes: u64,
    pub package_cache_bytes: u64,
    pub crash_dump_bytes: u64,
    pub docker: bool,
}

fn docker_usable() -> bool {
    has_tool("docker") && Command::new("docker").arg("info").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

pub fn maintenance_status() -> MaintenanceStatus {
    let state: MaintenanceState = load_json("maintenance.json");
    MaintenanceStatus {
        last_run: state.last_run,
        last_report: state.last_report,
        journal_bytes: crate::desktop::journal_bytes(),
        package_cache_bytes: du("/var/cache/pacman/pkg"),
        crash_dump_bytes: du("/var/lib/systemd/coredump"),
        docker: docker_usable(),
    }
}

/// Trim, old package versions, old logs, old crash dumps and Docker
/// leftovers, behind a single password prompt. Returns what each step did.
pub fn maintenance_run(now: i64) -> Result<Vec<String>, String> {
    let mut script = String::from("export LC_ALL=C\n");
    script.push_str("echo \"Drives trimmed: $(fstrim -av 2>/dev/null | wc -l) filesystem(s)\"\n");
    if has_tool("paccache") {
        script.push_str("echo \"Old package versions: $(paccache -rk2 2>&1 | tail -n 1)\"\n");
    }
    script.push_str("echo \"System log: $(journalctl --vacuum-size=500M --vacuum-time=8weeks 2>&1 | tail -n 1)\"\n");
    script.push_str("echo \"Crash dumps older than a week removed: $(find /var/lib/systemd/coredump -type f -mtime +7 -print -delete 2>/dev/null | wc -l)\"\n");
    let out = run_privileged(&script)?;
    let mut report: Vec<String> = out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if docker_usable() {
        // Only what nothing can be using: untagged image layers and the build cache.
        // Stopped containers are left alone; they may hold someone's work.
        let images = run_lossy(Command::new("docker").args(["image", "prune", "-f"]));
        let cache = run_lossy(Command::new("docker").args(["builder", "prune", "-f"]));
        report.push(format!(
            "Docker: untagged images — {}; build cache — {}",
            images.lines().last().unwrap_or("nothing to remove").to_lowercase(),
            cache.lines().last().unwrap_or("nothing to remove").to_lowercase()
        ));
    }
    let mut state: MaintenanceState = load_json("maintenance.json");
    state.last_run = now;
    state.last_report = report.clone();
    save_json("maintenance.json", &state);
    Ok(report)
}

/// A nudge once maintenance is a week overdue (at most one a day, and only
/// after it has been run once, so it never nags someone who doesn't use it).
pub fn maintenance_reminder(now: i64) {
    let mut state: MaintenanceState = load_json("maintenance.json");
    if state.last_run > 0 && now - state.last_run > 7 * 86_400 && now - state.last_reminder > 86_400 {
        notify("Weekly maintenance is due", "Open Trace → Control → Packages → Weekly Maintenance. It takes about a minute.", false);
        state.last_reminder = now;
        save_json("maintenance.json", &state);
    }
}

// ---- project work mode ----

#[derive(Serialize)]
pub struct WorkStatus {
    pub has_compose: bool,
    pub command: String,
    pub suggested: String,
    pub command_running: bool,
    pub containers_up: u32,
}

fn work_unit(path: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("trace-work-{:x}", h.finish())
}

fn project_dir(path: &str) -> Result<PathBuf, String> {
    let dir = fs::canonicalize(path).map_err(|_| "That folder no longer exists".to_string())?;
    let home = dirs::home_dir().ok_or("No home folder")?;
    if !dir.is_dir() || !dir.starts_with(&home) {
        return Err("Not a project folder".into());
    }
    Ok(dir)
}

fn has_compose(dir: &Path) -> bool {
    ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"].iter().any(|f| dir.join(f).is_file())
}

fn suggested_command(dir: &Path) -> String {
    if let Some(pkg) = fs::read_to_string(dir.join("package.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        let runner = if dir.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
            "bun"
        } else {
            "npm"
        };
        for script in ["dev", "start"] {
            if pkg["scripts"][script].is_string() {
                return format!("{runner} run {script}");
            }
        }
    }
    if dir.join("manage.py").exists() {
        return "python manage.py runserver".into();
    }
    String::new()
}

pub fn work_status(path: &str) -> Result<WorkStatus, String> {
    let dir = project_dir(path)?;
    let key = dir.to_string_lossy().to_string();
    let commands: HashMap<String, String> = load_json("project-work.json");
    let compose = has_compose(&dir);
    Ok(WorkStatus {
        has_compose: compose,
        command: commands.get(&key).cloned().unwrap_or_default(),
        suggested: suggested_command(&dir),
        command_running: run_lossy(Command::new("systemctl").args(["--user", "is-active", &format!("{}.service", work_unit(&key))])) == "active",
        containers_up: if compose && has_tool("docker") {
            run_lossy(Command::new("docker").args(["compose", "ps", "-q"]).current_dir(&dir)).lines().count() as u32
        } else {
            0
        },
    })
}

/// Starts the project's containers and its dev command. The command is
/// yours (typed or accepted in the UI) and runs as you, in the project folder.
pub fn work_start(path: &str, command: &str) -> ActionResult {
    let dir = match project_dir(path) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    let key = dir.to_string_lossy().to_string();
    let command = command.trim();
    let mut commands: HashMap<String, String> = load_json("project-work.json");
    if command.is_empty() {
        commands.remove(&key);
    } else {
        commands.insert(key.clone(), command.to_string());
    }
    save_json("project-work.json", &commands);

    let mut done = Vec::new();
    if has_compose(&dir) {
        match run(Command::new("docker").args(["compose", "up", "-d"]).current_dir(&dir)) {
            Ok(_) => done.push("containers started".to_string()),
            Err(e) => return err(format!("Couldn't start the containers: {}", e.lines().last().unwrap_or(""))),
        }
    }
    if !command.is_empty() {
        let unit = work_unit(&key);
        run(Command::new("systemctl").args(["--user", "reset-failed", &format!("{unit}.service")])).ok();
        let started = run(Command::new("systemd-run").args(["--user", "--quiet", "--collect", "--unit", &unit, "--working-directory", &key, "--", "sh", "-lc", command]));
        match started {
            Ok(_) => done.push(format!("“{command}” running")),
            Err(e) => return err(format!("Couldn't start “{command}”: {e}")),
        }
    }
    if done.is_empty() {
        return err("Nothing to start: this project has no compose file and no command set");
    }
    ok(format!("Work mode on: {}", done.join(", ")))
}

pub fn work_stop(path: &str) -> ActionResult {
    let dir = match project_dir(path) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    let key = dir.to_string_lossy().to_string();
    let mut done = Vec::new();
    if run(Command::new("systemctl").args(["--user", "stop", &format!("{}.service", work_unit(&key))])).is_ok() {
        done.push("command stopped");
    }
    if has_compose(&dir) && run(Command::new("docker").args(["compose", "stop"]).current_dir(&dir)).is_ok() {
        done.push("containers stopped");
    }
    ok(format!("Work mode off: {}", if done.is_empty() { "nothing was running".into() } else { done.join(", ") }))
}

/// The last lines the dev command printed.
pub fn work_log(path: &str) -> String {
    let Ok(dir) = project_dir(path) else {
        return String::new();
    };
    run_lossy(Command::new("journalctl").args(["--user", "-q", "--no-pager", "-n", "40", "-o", "cat", "-u", &format!("{}.service", work_unit(&dir.to_string_lossy()))]))
}

// ---- update briefing ----

#[derive(Serialize)]
pub struct News {
    pub title: String,
    pub date: String,
    pub link: String,
    /// The title says something must be done by hand.
    pub manual: bool,
}

#[derive(Serialize)]
pub struct UpdateBrief {
    pub count: usize,
    /// Pending packages that only take effect after a restart of the machine.
    pub needs_reboot: Vec<String>,
    /// Pending packages worth closing the app for (browsers, the desktop).
    pub big: Vec<String>,
    pub news: Vec<News>,
    /// Config files where the package shipped a newer version next to yours.
    pub pacnew: Vec<String>,
    /// Running apps still using files an update has replaced.
    pub stale_apps: Vec<String>,
    pub can_open_terminal: bool,
    /// "timeshift" | "snapper" | "" — what can take an undo point first.
    pub snapshot_tool: String,
}

const REBOOT_PREFIXES: &[&str] = &["linux", "nvidia", "systemd", "glibc", "amd-ucode", "intel-ucode", "mesa", "wayland", "hyprland", "pipewire", "dbus"];

fn tag<'a>(item: &'a str, name: &str) -> &'a str {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    item.split_once(&open).and_then(|(_, r)| r.split_once(&close)).map(|(v, _)| v.trim()).unwrap_or("")
}

fn parse_news(feed: &str) -> Vec<News> {
    feed.split("<item>")
        .skip(1)
        .take(4)
        .map(|item| {
            let title = tag(item, "title").replace("&amp;", "&").replace("&gt;", ">").replace("&lt;", "<").replace("&quot;", "\"");
            let lower = title.to_lowercase();
            News {
                manual: lower.contains("manual intervention") || lower.contains("requires") || lower.contains("action required"),
                date: tag(item, "pubDate").split(' ').skip(1).take(3).collect::<Vec<_>>().join(" "),
                link: tag(item, "link").to_string(),
                title,
            }
        })
        .collect()
}

fn stale_apps() -> Vec<String> {
    let mut apps = Vec::new();
    let Ok(dir) = fs::read_dir("/proc") else {
        return apps;
    };
    for entry in dir.flatten() {
        let pid = entry.file_name().to_string_lossy().to_string();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(maps) = fs::read_to_string(entry.path().join("maps")) else {
            continue;
        };
        // A library replaced on disk shows up as "(deleted)" in whoever still has the old one loaded.
        if maps.lines().any(|l| l.ends_with("(deleted)") && l.contains("/usr/") && l.contains(".so")) {
            if let Some(name) = crate::memory_map::app_name(&pid) {
                if !apps.contains(&name) {
                    apps.push(name);
                }
            }
        }
    }
    apps.sort();
    apps
}

fn snapshot_tool() -> &'static str {
    ["timeshift", "snapper"].into_iter().find(|t| has_tool(t)).unwrap_or("")
}

fn terminal() -> Option<&'static str> {
    ["kitty", "foot", "alacritty", "ghostty", "wezterm", "konsole"].into_iter().find(|t| has_tool(t))
}

/// What an update would change and what to know first. Talks to the
/// mirrors and the Arch news feed, so it takes a few seconds.
pub fn update_brief() -> UpdateBrief {
    let pending: Vec<String> = if has_tool("checkupdates") {
        run_lossy(&mut Command::new("checkupdates")).lines().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
    } else {
        Vec::new()
    };
    let needs_reboot: Vec<String> = pending.iter().filter(|p| REBOOT_PREFIXES.iter().any(|r| *p == r || p.starts_with(&format!("{r}-")))).cloned().collect();
    let big = pending.iter().filter(|p| ["brave", "firefox", "chromium", "zen", "code", "discord", "steam", "quickshell"].iter().any(|b| p.starts_with(b))).cloned().collect();
    let feed = run_lossy(Command::new("curl").args(["-s", "-m", "8", "https://archlinux.org/feeds/news/"]));
    let pacnew = run_lossy(Command::new("find").args(["/etc", "-xdev", "-name", "*.pacnew", "-type", "f"])).lines().filter(|l| safe_path(l)).map(String::from).collect();
    UpdateBrief { count: pending.len(), needs_reboot, big, news: parse_news(&feed), pacnew, stale_apps: stale_apps(), can_open_terminal: terminal().is_some(), snapshot_tool: snapshot_tool().to_string() }
}

fn pacnew_pair(path: &str) -> Result<(String, String), String> {
    let Some(original) = path.strip_suffix(".pacnew") else {
        return Err("Not a .pacnew file".into());
    };
    if !safe_path(path) || !path.starts_with("/etc/") || !Path::new(path).is_file() {
        return Err("Not a .pacnew file".into());
    }
    Ok((original.to_string(), path.to_string()))
}

/// The difference between your config file and the newer one the package shipped.
pub fn pacnew_diff(path: &str) -> Result<String, String> {
    let (original, new) = pacnew_pair(path)?;
    let out = Command::new("diff").args(["-u", &original, &new]).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        // Usually a file only root may read.
        return Ok("(The two files are identical, or one of them can only be read by root.)".into());
    }
    Ok(text.lines().take(300).collect::<Vec<_>>().join("\n"))
}

/// `use_new`: replace your file with the packaged one (yours is kept as
/// `.bak`). Otherwise keep yours and delete the `.pacnew`.
pub fn pacnew_resolve(path: &str, use_new: bool) -> ActionResult {
    let (original, new) = match pacnew_pair(path) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let script = if use_new { format!("cp -a '{original}' '{original}.bak' && mv -f '{new}' '{original}'") } else { format!("rm -f '{new}'") };
    match run_privileged(&script) {
        Ok(_) => ok(if use_new { format!("Now using the packaged version; your old file is {original}.bak") } else { "Kept your version".to_string() }),
        Err(e) => err(e),
    }
}

/// Opens a terminal running the system update, so you see and answer its questions yourself.
pub fn open_update_terminal(snapshot: bool) -> ActionResult {
    let Some(term) = terminal() else {
        return err("No terminal found to run the update in");
    };
    let before = match (snapshot, snapshot_tool()) {
        (true, "timeshift") => "sudo timeshift --create --comments 'before update (Trace)' && ",
        (true, "snapper") => "sudo snapper create --description 'before update (Trace)' && ",
        _ => "",
    };
    let script = format!("{before}sudo pacman -Syu; echo; echo 'Finished. Press Enter to close.'; read _");
    let script = script.as_str();
    let mut cmd = Command::new(term);
    if term == "wezterm" {
        cmd.args(["start", "--"]);
    } else {
        cmd.arg("-e");
    }
    match spawn_detached(cmd.args(["sh", "-c", script])) {
        Ok(()) => ok("Update opened in a terminal"),
        Err(e) => err(format!("Couldn't open the terminal: {e}")),
    }
}

// ---- duplicate and forgotten files ----

#[derive(Serialize, Clone)]
pub struct FileRow {
    pub path: String,
    pub bytes: u64,
    pub modified: i64,
}

#[derive(Serialize)]
pub struct DuplicateGroup {
    pub bytes: u64,
    pub paths: Vec<String>,
}

#[derive(Serialize)]
pub struct FileScan {
    pub folder: String,
    pub files: u64,
    pub truncated: bool,
    pub largest: Vec<FileRow>,
    /// Not changed in over a year.
    pub old: Vec<FileRow>,
    pub duplicates: Vec<DuplicateGroup>,
    pub duplicate_bytes: u64,
}

const SKIP_DIRS: &[&str] = &["node_modules", "target", "venv", "__pycache__", "dist", "build", "Steam", "steamapps"];
const MAX_FILES: usize = 300_000;

fn walk(root: &Path) -> (Vec<FileRow>, bool) {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // symlink_metadata: never follow links out of the folder.
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(entry.path());
                }
            } else if meta.is_file() {
                let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                files.push(FileRow { path: entry.path().to_string_lossy().to_string(), bytes: meta.len(), modified });
                if files.len() >= MAX_FILES {
                    return (files, true);
                }
            }
        }
    }
    (files, false)
}

fn hash_file(path: &str, limit: Option<u64>) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut read_total = 0u64;
    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        hasher.write(&buffer[..n]);
        read_total += n as u64;
        if limit.is_some_and(|l| read_total >= l) {
            break;
        }
    }
    Some(hasher.finish())
}

fn same_bytes(a: &str, b: &str) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let (Ok(na), Ok(nb)) = (fa.read(&mut ba), fb.read(&mut bb)) else {
            return false;
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

/// Files of at least `min_bytes` that have byte-for-byte identical twins.
fn find_duplicates(files: &[FileRow], min_bytes: u64) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<&FileRow>> = HashMap::new();
    for f in files.iter().filter(|f| f.bytes >= min_bytes) {
        by_size.entry(f.bytes).or_default().push(f);
    }
    let mut groups = Vec::new();
    for (bytes, same_size) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        // Cheap first: the first megabyte. Only matching ones are read in full.
        let mut by_head: HashMap<u64, Vec<&FileRow>> = HashMap::new();
        for f in same_size {
            if let Some(h) = hash_file(&f.path, Some(1 << 20)) {
                by_head.entry(h).or_default().push(f);
            }
        }
        for candidates in by_head.into_values().filter(|v| v.len() > 1) {
            let mut by_full: HashMap<u64, Vec<&FileRow>> = HashMap::new();
            for f in candidates {
                if let Some(h) = hash_file(&f.path, None) {
                    by_full.entry(h).or_default().push(f);
                }
            }
            for twins in by_full.into_values().filter(|v| v.len() > 1) {
                // A hash can collide; a byte comparison cannot.
                let paths: Vec<String> = twins.iter().filter(|f| same_bytes(&twins[0].path, &f.path)).map(|f| f.path.clone()).collect();
                if paths.len() > 1 {
                    groups.push(DuplicateGroup { bytes, paths });
                }
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.bytes * (g.paths.len() as u64 - 1)));
    groups
}

pub fn scan_files(folder: &str, now: i64) -> Result<FileScan, String> {
    let root = fs::canonicalize(folder).map_err(|_| "That folder doesn't exist".to_string())?;
    let home = dirs::home_dir().ok_or("No home folder")?;
    if !root.is_dir() || !root.starts_with(&home) {
        return Err("Pick a folder inside your home folder".into());
    }
    let (files, truncated) = walk(&root);
    let mut largest = files.clone();
    largest.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    largest.truncate(25);
    let mut old: Vec<FileRow> = files.iter().filter(|f| f.bytes >= 10 * 1024 * 1024 && now - f.modified > 365 * 86_400).cloned().collect();
    old.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    old.truncate(25);
    let mut duplicates = find_duplicates(&files, 1024 * 1024);
    let duplicate_bytes = duplicates.iter().map(|g| g.bytes * (g.paths.len() as u64 - 1)).sum();
    duplicates.truncate(40);
    Ok(FileScan { folder: root.to_string_lossy().to_string(), files: files.len() as u64, truncated, largest, old, duplicates, duplicate_bytes })
}

/// Moves files to the trash (never deletes outright), so a mistake can be undone.
pub fn trash_files(paths: &[String]) -> ActionResult {
    let Some(home) = dirs::home_dir() else {
        return err("No home folder");
    };
    let (mut moved, mut bytes) = (0, 0u64);
    for path in paths {
        let Ok(real) = fs::canonicalize(path) else {
            continue;
        };
        if !real.starts_with(&home) || !real.is_file() {
            continue;
        }
        let size = real.metadata().map(|m| m.len()).unwrap_or(0);
        if run(Command::new("gio").arg("trash").arg("--").arg(&real)).is_ok() {
            moved += 1;
            bytes += size;
        }
    }
    if moved == 0 {
        return err("Nothing could be moved to the trash");
    }
    ok(format!("Moved {moved} file(s) to the trash ({:.0} MB). Empty the trash to free the space.", bytes as f64 / 1048576.0))
}

// ---- backups ----

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct BackupConfig {
    pub sources: Vec<String>,
    pub destination: String,
    /// Run by itself once a week when the destination is connected.
    #[serde(default)]
    pub weekly: bool,
    #[serde(default)]
    pub last_ok: i64,
}

#[derive(Serialize, Default, Clone)]
pub struct BackupProgress {
    pub running: bool,
    pub message: String,
}

static BACKUP: Mutex<BackupProgress> = Mutex::new(BackupProgress { running: false, message: String::new() });

#[derive(Serialize)]
pub struct BackupView {
    pub config: BackupConfig,
    pub progress: BackupProgress,
    pub destination_connected: bool,
    /// The destination sits on the same drive as something it backs up.
    pub same_drive: bool,
    pub snapshots: Vec<String>,
    pub folder: String,
}

fn backup_root(config: &BackupConfig) -> PathBuf {
    let host = read_trimmed("/etc/hostname").unwrap_or_else(|| "machine".into());
    Path::new(&config.destination).join("trace-backups").join(host)
}

fn snapshots(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .map(|d| d.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| is_snapshot_name(n)).collect())
        .unwrap_or_default();
    names.sort_by(|a, b| b.cmp(a));
    names
}

/// "2026-10-09_231500"
fn is_snapshot_name(name: &str) -> bool {
    name.len() == 17 && name.chars().enumerate().all(|(i, c)| match i {
        4 | 7 => c == '-',
        10 => c == '_',
        _ => c.is_ascii_digit(),
    })
}

fn device_of(path: &str) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.dev())
}

pub fn backup_view() -> BackupView {
    let config: BackupConfig = load_json("backup.json");
    let root = backup_root(&config);
    let connected = !config.destination.is_empty() && Path::new(&config.destination).is_dir();
    let dest_dev = device_of(&config.destination);
    BackupView {
        same_drive: connected && config.sources.iter().any(|s| device_of(s) == dest_dev),
        destination_connected: connected,
        snapshots: if connected { snapshots(&root) } else { Vec::new() },
        folder: root.to_string_lossy().to_string(),
        progress: BACKUP.lock().unwrap().clone(),
        config,
    }
}

pub fn backup_save(mut config: BackupConfig) -> ActionResult {
    config.sources.retain(|s| Path::new(s).is_dir());
    config.sources.dedup();
    if !config.destination.is_empty() {
        if !Path::new(&config.destination).is_dir() {
            return err("The destination folder doesn't exist (is the drive plugged in?)");
        }
        if config.sources.iter().any(|s| Path::new(&config.destination).starts_with(s)) {
            return err("The destination is inside a folder being backed up — pick somewhere else");
        }
    }
    config.last_ok = load_json::<BackupConfig>("backup.json").last_ok;
    save_json("backup.json", &config);
    ok("Backup settings saved")
}

fn set_progress(running: bool, message: impl Into<String>) {
    *BACKUP.lock().unwrap() = BackupProgress { running, message: message.into() };
}

fn rsync_base() -> Command {
    let mut cmd = Command::new("rsync");
    cmd.args(["-a", "--delete", "--exclude=node_modules/", "--exclude=target/", "--exclude=.cache/", "--exclude=__pycache__/"]);
    cmd
}

fn make_snapshot(config: &BackupConfig) -> Result<String, String> {
    let root = backup_root(config);
    fs::create_dir_all(&root).map_err(|e| format!("Can't write to the destination: {e}"))?;
    let previous = snapshots(&root).into_iter().next();
    let name = run(Command::new("date").arg("+%Y-%m-%d_%H%M%S"))?;
    let partial = root.join(format!("{name}.partial"));
    for source in &config.sources {
        let base = Path::new(source).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "root".into());
        set_progress(true, format!("Copying {base}…"));
        let target = partial.join(&base);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        let mut cmd = rsync_base();
        if let Some(prev) = &previous {
            // Unchanged files become hard links to the previous snapshot: every
            // snapshot is complete, but only changes take new space.
            cmd.arg(format!("--link-dest={}", root.join(prev).join(&base).to_string_lossy()));
        }
        let out = cmd.arg(format!("{source}/")).arg(format!("{}/", target.to_string_lossy())).output().map_err(|e| e.to_string())?;
        // 24 = some files vanished while copying, which is normal on a live system.
        if !out.status.success() && out.status.code() != Some(24) {
            fs::remove_dir_all(&partial).ok();
            return Err(format!("Copying {base} failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
        }
    }
    fs::rename(&partial, root.join(&name)).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Starts a backup on its own thread; progress is read through `backup_view`.
pub fn backup_start(now: i64) -> ActionResult {
    let config: BackupConfig = load_json("backup.json");
    if config.sources.is_empty() || config.destination.is_empty() {
        return err("Choose what to back up and where to first");
    }
    if !Path::new(&config.destination).is_dir() {
        return err("The destination isn't connected");
    }
    if !has_tool("rsync") {
        return err("Backups need rsync installed");
    }
    {
        let mut progress = BACKUP.lock().unwrap();
        if progress.running {
            return err("A backup is already running");
        }
        *progress = BackupProgress { running: true, message: "Starting…".into() };
    }
    std::thread::spawn(move || match make_snapshot(&config) {
        Ok(name) => {
            let mut saved: BackupConfig = load_json("backup.json");
            saved.last_ok = now;
            save_json("backup.json", &saved);
            set_progress(false, format!("Backup {name} finished"));
            notify("Backup finished", &format!("Snapshot {name} is on {}.", config.destination), false);
        }
        Err(e) => {
            set_progress(false, format!("Backup failed: {e}"));
            notify("Backup failed", &e, true);
        }
    });
    ok("Backup started")
}

fn snapshot_path(name: &str) -> Result<(BackupConfig, PathBuf), String> {
    let config: BackupConfig = load_json("backup.json");
    let path = backup_root(&config).join(name);
    if !is_snapshot_name(name) || !path.is_dir() {
        return Err("No such snapshot".into());
    }
    Ok((config, path))
}

/// Reads every file in the snapshot back and compares it with the original.
pub fn backup_verify(name: &str) -> ActionResult {
    let (config, path) = match snapshot_path(name) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    {
        let mut progress = BACKUP.lock().unwrap();
        if progress.running {
            return err("Wait for the running backup to finish");
        }
        *progress = BackupProgress { running: true, message: format!("Checking {name}…") };
    }
    let name = name.to_string();
    std::thread::spawn(move || {
        let (mut differing, mut unreadable) = (0usize, 0usize);
        for source in &config.sources {
            let base = Path::new(source).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let out = rsync_base().args(["-n", "-c", "-i"]).arg(format!("{source}/")).arg(format!("{}/", path.join(&base).to_string_lossy())).output();
            match out {
                Ok(out) => {
                    differing += String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.starts_with(">f") || l.starts_with("*deleting")).count();
                    unreadable += String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.contains("failed") || l.contains("error")).count();
                }
                Err(_) => unreadable += 1,
            }
        }
        let message = if unreadable > 0 {
            format!("{name}: {unreadable} read error(s) — the backup drive may be failing")
        } else if differing == 0 {
            format!("{name} is readable and matches your files exactly")
        } else {
            format!("{name} is readable; {differing} file(s) have changed since it was taken")
        };
        set_progress(false, message.clone());
        notify("Backup check finished", &message, unreadable > 0);
    });
    ok("Checking — this reads the whole backup, so it takes a while")
}

pub fn backup_delete(name: &str) -> ActionResult {
    match snapshot_path(name) {
        Ok((_, path)) => match fs::remove_dir_all(&path) {
            Ok(()) => ok(format!("Snapshot {name} deleted")),
            Err(e) => err(format!("Couldn't delete it: {e}")),
        },
        Err(e) => err(e),
    }
}

/// Opens a snapshot in the file manager: restoring is copying files back out of it.
pub fn backup_open(name: &str) -> ActionResult {
    match snapshot_path(name) {
        Ok((_, path)) => match spawn_detached(Command::new("xdg-open").arg(&path)) {
            Ok(()) => ok("Opened — copy what you need back out of it"),
            Err(e) => err(e.to_string()),
        },
        Err(e) => err(e),
    }
}

/// The weekly automatic run, if it is switched on and the drive is there.
pub fn backup_tick(now: i64) {
    let config: BackupConfig = load_json("backup.json");
    if config.weekly && now - config.last_ok > 7 * 86_400 && Path::new(&config.destination).is_dir() && !BACKUP.lock().unwrap().running {
        backup_start(now);
    }
}

// ---- game mode ----

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct GameMode {
    pub enabled: bool,
    /// Preset id from Tweaks, applied while a game runs.
    pub preset: String,
    /// A different preset for particular games, by game name.
    #[serde(default)]
    pub per_game: HashMap<String, String>,
    /// Games noticed so far, so they can be given their own preset.
    #[serde(default)]
    pub seen: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct GameActive {
    game: String,
    restore: Vec<Change>,
}

pub fn game_mode() -> GameMode {
    load_json("game-mode.json")
}

pub fn set_game_mode(mode: &GameMode) {
    save_json("game-mode.json", mode);
}

/// The name of a game from one process's command line, if it is one.
fn game_from_cmdline(cmdline: &str) -> Option<String> {
    if let Some(rest) = cmdline.split("/steamapps/common/").nth(1) {
        let name = rest.split(['/', '\0']).next().unwrap_or("");
        // The runtime and Proton are how games are started, not games.
        if !name.is_empty() && !name.starts_with("SteamLinuxRuntime") && !name.starts_with("Proton") && !name.starts_with("Steamworks") {
            return Some(name.to_string());
        }
        return None;
    }
    let program = cmdline.split('\0').next().unwrap_or("").rsplit('/').next().unwrap_or("");
    match program {
        "gamescope" => Some("a game (gamescope)".into()),
        "lutris-wrapper" => Some("a Lutris game".into()),
        _ => None,
    }
}

pub fn running_game() -> Option<String> {
    let dir = fs::read_dir("/proc").ok()?;
    for entry in dir.flatten() {
        if !entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(cmdline) = fs::read_to_string(entry.path().join("cmdline")) {
            if let Some(game) = game_from_cmdline(&cmdline) {
                return Some(game);
            }
        }
    }
    None
}

/// Applies the chosen preset when a game starts and puts every setting back
/// when it ends. Never raises a password prompt. Returns a line for the log.
pub fn game_tick(db: &Db) -> Option<(bool, String)> {
    let mode = game_mode();
    let active: Option<GameActive> = fs::read_to_string(data_dir().join("game-active.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let game = if mode.enabled { running_game() } else { None };
    match (active, game) {
        (None, Some(game)) => {
            let mut mode = mode;
            if !mode.seen.contains(&game) {
                mode.seen.push(game.clone());
                set_game_mode(&mode);
            }
            let preset = mode.per_game.get(&game).filter(|p| !p.is_empty()).unwrap_or(&mode.preset).clone();
            let mode = GameMode { preset, ..mode };
            let changes = crate::tweak_presets::changes_for(&mode.preset, &crate::tweaks::list(db))?;
            let results = crate::tweaks::apply_with(db, &changes, &format!("game:{game}"), None, false);
            let restore: Vec<Change> = results.iter().filter(|r| r.ok).map(|r| Change { id: r.id.clone(), value: r.old_value.clone() }).collect();
            let applied = restore.len();
            save_json("game-active.json", &GameActive { game: game.clone(), restore });
            notify("Game mode on", &format!("{game} started: {applied} setting(s) changed until it closes."), false);
            Some((true, format!("{game} started — applied {applied} of {} setting(s) from “{}”", results.len(), mode.preset)))
        }
        (Some(active), None) => {
            let results = crate::tweaks::apply_with(db, &active.restore, "game ended", None, false);
            fs::remove_file(data_dir().join("game-active.json")).ok();
            let failed = results.iter().filter(|r| !r.ok).count();
            Some((failed == 0, format!("{} closed — {} setting(s) put back, {failed} failed", active.game, results.len() - failed)))
        }
        _ => None,
    }
}

// ---- budgets for groups of apps ----

#[derive(Serialize, Deserialize, Clone)]
pub struct GroupBudget {
    pub name: String,
    pub apps: Vec<String>,
    pub max_gb: f64,
    /// "notify" | "freeze" (freezes the heaviest member; for CPU, caps every member)
    pub action: String,
    /// Processor allowance for the whole group in percent of one core (200 = two cores); 0 = none.
    #[serde(default)]
    pub max_cpu_percent: f64,
}

#[derive(Serialize)]
pub struct GroupUse {
    pub budget: GroupBudget,
    pub used_bytes: u64,
    pub cpu_percent: f64,
}

pub fn group_budgets() -> Vec<GroupBudget> {
    load_json("group-budgets.json")
}

pub fn save_group_budgets(budgets: &[GroupBudget]) {
    let clean: Vec<&GroupBudget> = budgets.iter().filter(|b| !b.name.trim().is_empty() && !b.apps.is_empty() && (b.max_gb > 0.0 || b.max_cpu_percent > 0.0)).collect();
    save_json("group-budgets.json", &clean);
}

static CPU_PREV: Mutex<Option<(std::time::Instant, HashMap<String, u64>)>> = Mutex::new(None);

/// Processor use per app since the last call, in percent of one core.
/// Empty on the first call (there is nothing to compare with yet).
pub fn app_cpu() -> HashMap<String, f64> {
    let (ticks, _) = crate::batch4::cpu_ticks_by_app();
    let now = std::time::Instant::now();
    let mut prev = CPU_PREV.lock().unwrap();
    let usage = match prev.as_ref() {
        Some((then, before)) => {
            let secs = now.duration_since(*then).as_secs_f64().max(1.0);
            // The kernel counts processor time in hundredths of a second.
            ticks.iter().map(|(app, t)| (app.clone(), t.saturating_sub(*before.get(app).unwrap_or(t)) as f64 / secs)).collect()
        }
        None => HashMap::new(),
    };
    *prev = Some((now, ticks));
    usage
}

pub fn group_use(apps: &[(String, u64)], cpu: &HashMap<String, f64>) -> Vec<GroupUse> {
    group_budgets()
        .into_iter()
        .map(|budget| GroupUse {
            used_bytes: apps.iter().filter(|(name, _)| budget.apps.contains(name)).map(|(_, b)| *b).sum(),
            cpu_percent: budget.apps.iter().filter_map(|a| cpu.get(a)).sum(),
            budget,
        })
        .collect()
}

static GROUP_ALERTED: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

/// Checks every group against its allowance; `apps` is (name, bytes in use).
pub fn group_tick(apps: &[(String, u64)], now: i64) -> Vec<(bool, String)> {
    let mut log = Vec::new();
    let mut alerted = GROUP_ALERTED.lock().unwrap();
    let alerted = alerted.get_or_insert_with(HashMap::new);
    let cpu = if group_budgets().iter().any(|b| b.max_cpu_percent > 0.0) { app_cpu() } else { HashMap::new() };
    for usage in group_use(apps, &cpu) {
        let budget = &usage.budget;
        let members: Vec<&String> = apps.iter().map(|(n, _)| n).filter(|n| budget.apps.contains(n)).collect();

        // Processor allowance.
        let cpu_key = format!("{}:cpu", budget.name);
        if budget.max_cpu_percent > 0.0 && usage.cpu_percent > budget.max_cpu_percent {
            if !alerted.get(&cpu_key).is_some_and(|last| now - last < 900) {
                alerted.insert(cpu_key, now);
                let action = if budget.action == "freeze" && !members.is_empty() {
                    // An even share each, so the group as a whole can't exceed its allowance.
                    let share = ((budget.max_cpu_percent / members.len() as f64) as u32).max(5);
                    for app in &members {
                        let memory = crate::memory_map::app_limits(app).memory_mb;
                        crate::memory_map::set_app_limits(app, Some(share), memory);
                    }
                    format!("capped each of its apps at {share}% of a core")
                } else {
                    "told you".to_string()
                };
                notify(
                    &format!("“{}” is over its processor allowance", budget.name),
                    &format!("Using {:.0}% of a core; the allowance is {:.0}%. Trace {action}.", usage.cpu_percent, budget.max_cpu_percent),
                    false,
                );
                log.push((true, format!("group {} over its processor allowance ({:.0}% of {:.0}%): {action}", budget.name, usage.cpu_percent, budget.max_cpu_percent)));
            }
        } else {
            alerted.remove(&cpu_key);
        }

        // Memory allowance.
        let limit = (budget.max_gb * 1e9) as u64;
        if limit == 0 || usage.used_bytes <= limit {
            alerted.remove(&budget.name);
            continue;
        }
        // Once per quarter of an hour, not on every check.
        if alerted.get(&budget.name).is_some_and(|last| now - last < 900) {
            continue;
        }
        alerted.insert(budget.name.clone(), now);
        let used_gb = usage.used_bytes as f64 / 1e9;
        let heaviest = apps.iter().filter(|(n, _)| budget.apps.contains(n)).max_by_key(|(_, b)| *b).map(|(n, _)| n.clone());
        let (did_ok, action) = match (budget.action.as_str(), heaviest) {
            ("freeze", Some(app)) => {
                let r = crate::memory_map::app_action(&app, "freeze");
                (r.ok, format!("froze {app}"))
            }
            _ => (true, "told you".to_string()),
        };
        notify(
            &format!("“{}” is over its memory allowance", budget.name),
            &format!("Using {used_gb:.1} GB of {:.1} GB. Trace {action}.", budget.max_gb),
            false,
        );
        log.push((did_ok, format!("group {} over budget ({used_gb:.1} of {:.1} GB): {action}", budget.name, budget.max_gb)));
    }
    log
}

// ---- sound per app ----

#[derive(Serialize)]
pub struct Stream {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub volume: u32,
    pub muted: bool,
    pub sink: String,
}

#[derive(Serialize)]
pub struct Output {
    pub name: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct Mixer {
    pub streams: Vec<Stream>,
    pub outputs: Vec<Output>,
    pub noise_suppression: bool,
}

fn pactl_json(what: &str) -> Vec<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(&run_lossy(Command::new("pactl").args(["-f", "json", "list", what])))
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn noise_module() -> Option<String> {
    run_lossy(Command::new("pactl").args(["list", "short", "modules"]))
        .lines()
        .find(|l| l.contains("module-echo-cancel") && l.contains("trace_clean_mic"))
        .and_then(|l| l.split_whitespace().next().map(String::from))
}

pub fn mixer() -> Mixer {
    let sinks = pactl_json("sinks");
    let sink_name = |index: u64| sinks.iter().find(|s| s["index"].as_u64() == Some(index)).and_then(|s| s["name"].as_str()).unwrap_or("").to_string();
    let streams = pactl_json("sink-inputs")
        .iter()
        .filter_map(|s| {
            let volume = s["volume"].as_object()?.values().next()?["value_percent"].as_str()?.trim_end_matches('%').trim().parse().ok()?;
            let props = &s["properties"];
            Some(Stream {
                id: s["index"].as_u64()? as u32,
                app: props["application.name"].as_str().or(props["application.process.binary"].as_str()).unwrap_or("an app").to_string(),
                title: props["media.name"].as_str().unwrap_or("").to_string(),
                volume,
                muted: s["mute"].as_bool().unwrap_or(false),
                sink: sink_name(s["sink"].as_u64().unwrap_or(u64::MAX)),
            })
        })
        .collect();
    let outputs = sinks
        .iter()
        .filter_map(|s| Some(Output { name: s["name"].as_str()?.to_string(), label: s["description"].as_str().unwrap_or("").to_string() }))
        .collect();
    Mixer { streams, outputs, noise_suppression: noise_module().is_some() }
}

/// `action`: "volume" (value 0–150) | "mute" (value 0/1) | "move" (value = output name).
pub fn stream_set(id: u32, action: &str, value: &str) -> ActionResult {
    let id = id.to_string();
    let result = match action {
        "volume" => match value.parse::<u32>() {
            Ok(v) if v <= 150 => run(Command::new("pactl").args(["set-sink-input-volume", &id, &format!("{v}%")])),
            _ => Err("Volume must be 0–150".into()),
        },
        "mute" => run(Command::new("pactl").args(["set-sink-input-mute", &id, if value == "1" { "1" } else { "0" }])),
        "move" if value.chars().all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c)) => run(Command::new("pactl").args(["move-sink-input", &id, value])),
        _ => Err("Unknown action".into()),
    };
    match result {
        Ok(_) => ok("Done"),
        Err(e) => err(e),
    }
}

/// Adds (or removes) a "clean" microphone that filters out background
/// noise and echo, and makes it the default.
pub fn set_noise_suppression(on: bool) -> ActionResult {
    if let Some(module) = noise_module() {
        if on {
            return ok("Noise suppression is already on");
        }
        return match run(Command::new("pactl").args(["unload-module", &module])) {
            Ok(_) => ok("Noise suppression off — the plain microphone is back"),
            Err(e) => err(e),
        };
    }
    if !on {
        return ok("Noise suppression is already off");
    }
    let loaded = run(Command::new("pactl").args([
        "load-module",
        "module-echo-cancel",
        "aec_method=webrtc",
        "noise_suppression=true",
        "source_name=trace_clean_mic",
        "source_properties=device.description=Microphone\\ (noise\\ removed)",
    ]));
    match loaded {
        Ok(_) => {
            run(Command::new("pactl").args(["set-default-source", "trace_clean_mic"])).ok();
            ok("Noise suppression on: apps now record from “Microphone (noise removed)”. It lasts until you log out.")
        }
        Err(e) => err(format!("The sound system refused it: {e}")),
    }
}

// ---- USB power per device ----

fn is_usb_port(port: &str) -> bool {
    !port.is_empty() && port.len() <= 24 && port.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.') && port.contains('-')
}

/// `what`: "wake" (may wake the machine from sleep) | "saving" (may power down when idle).
pub fn set_usb_power(port: &str, what: &str, on: bool) -> ActionResult {
    if !is_usb_port(port) {
        return err("Not a USB port");
    }
    let (file, value) = match what {
        "wake" => ("wakeup", if on { "enabled" } else { "disabled" }),
        "saving" => ("control", if on { "auto" } else { "on" }),
        _ => return err("Unknown setting"),
    };
    let path = format!("/sys/bus/usb/devices/{port}/power/{file}");
    if !Path::new(&path).exists() {
        return err("That device doesn't offer this setting (or has been unplugged)");
    }
    match run_privileged(&format!("echo {value} > '{path}'")) {
        Ok(_) => ok(match (what, on) {
            ("wake", true) => "It can now wake the machine from sleep",
            ("wake", false) => "It can no longer wake the machine",
            (_, true) => "It may power down when idle",
            (_, false) => "It stays powered all the time",
        }
        .to_string() + ". This lasts until the device is unplugged or the machine restarts."),
        Err(e) => err(e),
    }
}

// ---- charging to full once ----

#[derive(Serialize)]
pub struct ChargeOnce {
    /// The usual limit, or None when the battery has no limit setting.
    pub limit: Option<u32>,
    /// Set while a one-off full charge is under way: the limit to go back to.
    pub restore_to: Option<u32>,
    /// A full charge planned for later: when the limit will be lifted, and when it should be full.
    pub lift_at: Option<i64>,
    pub ready_by: Option<i64>,
}

fn charge_limit() -> Option<u32> {
    let path = crate::tweaks::expand_globs(&["/sys/class/power_supply/BAT*/charge_control_end_threshold"]).into_iter().next()?;
    read_num(&path)
}

pub fn charge_once_status() -> ChargeOnce {
    let plan: Option<(i64, i64)> = load_json("charge-plan.json");
    ChargeOnce { limit: charge_limit(), restore_to: load_json::<Option<u32>>("charge-once.json"), lift_at: plan.map(|p| p.0), ready_by: plan.map(|p| p.1) }
}

/// Lifts the charge limit to 100% for one charge (asks for the password
/// unless the trusted helper is on). `charge_tick` puts it back.
pub fn charge_once_start(db: &Db) -> ActionResult {
    fs::remove_file(data_dir().join("charge-plan.json")).ok();
    charge_once_start_with(db, true)
}

/// How long before the deadline the limit is lifted: enough to fill the
/// last stretch at the slow rate batteries charge near full.
const CHARGE_LEAD_SECS: i64 = 150 * 60;

/// Plans a full charge to be ready at the next `hour:minute`.
pub fn charge_plan(hour: u32, minute: u32, now: i64) -> ActionResult {
    let Some(limit) = charge_limit() else {
        return err("This battery has no charge limit");
    };
    if limit >= 100 || hour > 23 || minute > 59 {
        return err("There is no charge limit set, so it already charges to full");
    }
    let offset = crate::batch6::utc_offset();
    let local = now + offset;
    let mut ready_local = local - local.rem_euclid(86_400) + hour as i64 * 3600 + minute as i64 * 60;
    if ready_local <= local + 600 {
        ready_local += 86_400;
    }
    let ready_by = ready_local - offset;
    save_json("charge-plan.json", &Some(((ready_by - CHARGE_LEAD_SECS).max(now), ready_by)));
    ok(format!("Planned: the {limit}% limit lifts about two and a half hours before {hour:02}:{minute:02}. Leave it plugged in. This needs the trusted helper (Tweaks) to happen without you; otherwise Trace will ask you at that moment."))
}

pub fn charge_plan_cancel() -> ActionResult {
    fs::remove_file(data_dir().join("charge-plan.json")).ok();
    ok("Planned full charge cancelled")
}

fn charge_once_start_with(db: &Db, allow_root: bool) -> ActionResult {
    let Some(limit) = charge_limit() else {
        return err("This battery has no charge limit");
    };
    if limit >= 100 {
        return err("There is no charge limit set, so it already charges to full");
    }
    let results = crate::tweaks::apply_with(db, &[Change { id: "battery.charge_limit".into(), value: "100".into() }], "full charge once", None, allow_root);
    match results.first() {
        Some(r) if r.ok => {
            save_json("charge-once.json", &Some(limit));
            ok(format!("Charging to 100% this once. The {limit}% limit comes back when it is full or unplugged."))
        }
        Some(r) => err(r.message.clone()),
        None => err("The charge limit setting isn't available here"),
    }
}

/// Puts the limit back once the battery is full or the charger was pulled.
pub fn charge_tick(db: &Db, now: i64) -> Option<(bool, String)> {
    if let Some((lift_at, _)) = load_json::<Option<(i64, i64)>>("charge-plan.json") {
        if now >= lift_at {
            fs::remove_file(data_dir().join("charge-plan.json")).ok();
            let r = charge_once_start_with(db, false);
            if !r.ok {
                notify("Time to charge to full", "Open Trace → Health and press “Charge to full this once” (the trusted helper is off, so Trace can't lift the limit by itself).", true);
            }
            return Some((r.ok, format!("planned full charge: {}", r.message)));
        }
    }
    let restore: u32 = load_json::<Option<u32>>("charge-once.json")?;
    let battery = crate::batch4::battery_now()?;
    if battery.percent < 100 && !battery.discharging {
        return None;
    }
    let results = crate::tweaks::apply_with(db, &[Change { id: "battery.charge_limit".into(), value: restore.to_string() }], "full charge done", None, false);
    fs::remove_file(data_dir().join("charge-once.json")).ok();
    let restored = results.first().is_some_and(|r| r.ok);
    if !restored {
        // Without the trusted helper Trace can't write it back unprompted.
        notify("Put the charge limit back", &format!("The one-off full charge is done. Set “Stop charging at” back to {restore}% in Tweaks."), false);
    }
    Some((restored, format!("one-off full charge finished; limit back to {restore}%: {}", if restored { "done" } else { "needs you (no trusted helper)" })))
}

// ---- monitor colour ----

#[derive(Serialize)]
pub struct MonitorColour {
    pub name: String,
    pub description: String,
    /// "srgb" | "wide" | "hdr" | …
    pub mode: String,
    pub saturation: f64,
    pub brightness: f64,
    pub ten_bit: bool,
}

pub fn monitor_colours() -> Vec<MonitorColour> {
    let out = run_lossy(Command::new("hyprctl").args(["monitors", "-j"]));
    serde_json::from_str::<serde_json::Value>(&out)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|m| MonitorColour {
            name: m["name"].as_str().unwrap_or_default().to_string(),
            description: m["description"].as_str().unwrap_or_default().to_string(),
            mode: m["colorManagementPreset"].as_str().unwrap_or("srgb").to_string(),
            saturation: m["sdrSaturation"].as_f64().unwrap_or(1.0),
            brightness: m["sdrBrightness"].as_f64().unwrap_or(1.0),
            ten_bit: m["currentFormat"].as_str().is_some_and(|f| f.contains("2101010")),
        })
        .collect()
}

pub const COLOUR_MODES: &[&str] = &["auto", "srgb", "wide", "hdr"];

/// The Lua call that changes one monitor's colour handling while keeping
/// its mode, place, scale and rotation exactly as they are.
fn lua_colour(setup: &crate::hardware::MonitorSetup, mode: &str, saturation: f64, brightness: f64, icc: &str) -> String {
    let profile = if icc.is_empty() { String::new() } else { format!(", icc = \"{icc}\"") };
    format!(
        "hl.monitor({{ output = \"{}\", mode = \"{}\", position = \"{}x{}\", scale = {}, transform = {}, vrr = {}, bitdepth = {}, cm = \"{mode}\", sdrsaturation = {saturation:.2}, sdrbrightness = {brightness:.2}{profile} }})",
        setup.name,
        setup.mode.trim_end_matches("Hz"),
        setup.x,
        setup.y,
        setup.scale,
        setup.transform,
        setup.vrr as u8,
        if matches!(mode, "wide" | "hdr") { 10 } else { 8 },
    )
}

/// Colour mode, saturation and SDR brightness for one monitor, for this
/// session (a Hyprland reload restores what its config says).
pub fn set_monitor_colour(name: &str, mode: &str, saturation: f64, brightness: f64, icc: &str) -> ActionResult {
    // A profile path goes into a Lua string, so only plain path characters are allowed.
    let icc_ok = icc.is_empty()
        || (icc.starts_with('/') && (icc.ends_with(".icc") || icc.ends_with(".icm")) && Path::new(icc).is_file() && icc.chars().all(|c| c.is_ascii_alphanumeric() || "/._- ".contains(c)));
    if !icc_ok {
        return err("Pick an .icc or .icm colour profile file (plain letters and digits in its path)");
    }
    if !COLOUR_MODES.contains(&mode) || !(0.5..=1.5).contains(&saturation) || !(0.5..=2.0).contains(&brightness) {
        return err("Pick a listed colour mode, saturation 0.5–1.5 and brightness 0.5–2");
    }
    if !crate::hypr::is_lua() {
        return err("Monitor colour needs a Hyprland with the Lua config");
    }
    let monitors = crate::hardware::monitors();
    let Some(info) = monitors.iter().find(|m| m.setup.name == name && m.setup.enabled) else {
        return err("No such monitor");
    };
    match run(Command::new("hyprctl").args(["eval", &lua_colour(&info.setup, mode, saturation, brightness, icc)])) {
        Ok(out) if out.trim() == "ok" || out.trim().is_empty() => ok(format!("{name}: {mode}, saturation {saturation:.2}, brightness {brightness:.2}")),
        Ok(out) | Err(out) => err(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("trace-b5-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_real_duplicates_and_not_lookalikes() {
        let dir = scratch("dups");
        let big = vec![7u8; 2 * 1024 * 1024];
        let mut other = big.clone();
        *other.last_mut().unwrap() = 8; // same size, same first megabyte, different end
        fs::write(dir.join("a.bin"), &big).unwrap();
        fs::write(dir.join("b.bin"), &big).unwrap();
        fs::write(dir.join("c.bin"), &other).unwrap();
        fs::write(dir.join("small1"), b"x").unwrap();
        fs::write(dir.join("small2"), b"x").unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules/a.bin"), &big).unwrap();
        let (files, truncated) = walk(&dir);
        assert!(!truncated);
        assert_eq!(files.len(), 5, "node_modules is skipped");
        let groups = find_duplicates(&files, 1024 * 1024);
        assert_eq!(groups.len(), 1);
        let mut names: Vec<&str> = groups[0].paths.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["a.bin", "b.bin"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn trash_and_scan_refuse_anything_outside_home() {
        assert!(!trash_files(&["/etc/hostname".to_string()]).ok);
        assert!(scan_files("/etc", 0).is_err());
        assert!(project_dir("/etc").is_err());
    }

    #[test]
    fn snapshots_hard_link_unchanged_files() {
        if !has_tool("rsync") {
            return;
        }
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("backup");
        let (src, dest) = (dir.join("docs"), dir.join("drive"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("keep.txt"), "same").unwrap();
        fs::write(src.join("edit.txt"), "one").unwrap();
        let config = BackupConfig { sources: vec![src.to_string_lossy().to_string()], destination: dest.to_string_lossy().to_string(), weekly: false, last_ok: 0 };
        let first = make_snapshot(&config).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100)); // snapshot names are to the second
        fs::write(src.join("edit.txt"), "two!").unwrap();
        fs::remove_file(src.join("keep.txt")).ok();
        fs::write(src.join("keep.txt"), "same").ok();
        let second = make_snapshot(&config).unwrap();
        assert_ne!(first, second);
        let root = backup_root(&config);
        assert_eq!(snapshots(&root), vec![second.clone(), first.clone()]);
        // The first snapshot still holds the old content: snapshots are independent.
        assert_eq!(fs::read_to_string(root.join(&first).join("docs/edit.txt")).unwrap(), "one");
        assert_eq!(fs::read_to_string(root.join(&second).join("docs/edit.txt")).unwrap(), "two!");
        assert!(root.join(&second).join("docs/keep.txt").metadata().unwrap().nlink() >= 1);
        assert!(is_snapshot_name(&first) && !is_snapshot_name("../../etc") && !is_snapshot_name("2026-10-09_2315.partial"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recognises_games_but_not_the_launcher() {
        assert_eq!(game_from_cmdline("/home/u/.local/share/Steam/steamapps/common/Hades/Hades\0-fullscreen").as_deref(), Some("Hades"));
        assert_eq!(game_from_cmdline("/home/u/.local/share/Steam/steamapps/common/SteamLinuxRuntime_sniper/_v2-entry-point\0"), None);
        assert_eq!(game_from_cmdline("/home/u/.local/share/Steam/ubuntu12_64/steamwebhelper\0"), None);
        assert!(game_from_cmdline("/usr/bin/gamescope\0-f\0").is_some());
        assert_eq!(game_from_cmdline("/usr/bin/brave\0"), None);
    }

    #[test]
    fn reads_the_news_feed() {
        let feed = "<rss><channel><title>Arch</title><item><title>foo &gt;= 2 requires manual intervention</title><link>https://archlinux.org/news/foo/</link><pubDate>Tue, 06 Oct 2026 10:00:00 +0000</pubDate></item><item><title>Plain news</title><link>https://x</link><pubDate>Mon, 01 Sep 2026 10:00:00 +0000</pubDate></item></channel></rss>";
        let news = parse_news(feed);
        assert_eq!(news.len(), 2);
        assert!(news[0].manual && !news[1].manual);
        assert_eq!(news[0].title, "foo >= 2 requires manual intervention");
        assert_eq!(news[0].date, "06 Oct 2026");
    }

    #[test]
    fn root_script_inputs_are_validated() {
        assert!(pacnew_pair("/etc/pacman.conf.pacnew'; rm -rf /; '.pacnew").is_err());
        assert!(pacnew_pair("/etc/../home/x.pacnew").is_err());
        assert!(pacnew_pair("/home/x/a.pacnew").is_err());
        assert!(is_usb_port("1-2.3") && !is_usb_port("1-2/../../x") && !is_usb_port("usb1; reboot"));
        assert!(!set_usb_power("1-2; reboot", "wake", true).ok);
    }

    #[test]
    fn colour_change_keeps_the_monitor_where_it_is() {
        let setup = crate::hardware::MonitorSetup { name: "HDMI-A-1".into(), mode: "1920x1080@60.00Hz".into(), x: 1920, y: 0, scale: 1.0, transform: 0, vrr: false, mirror_of: String::new(), enabled: true };
        let lua = lua_colour(&setup, "hdr", 1.1, 1.0, "");
        assert!(lua_colour(&setup, "srgb", 1.0, 1.0, "/x/a.icc").contains(r#"icc = "/x/a.icc""#));
        assert!(lua.contains(r#"mode = "1920x1080@60.00""#) && lua.contains(r#"position = "1920x0""#));
        assert!(lua.contains("bitdepth = 10") && lua.contains(r#"cm = "hdr""#) && lua.contains("sdrsaturation = 1.10"));
        assert!(!set_monitor_colour("HDMI-A-1", "rainbow", 1.0, 1.0, "").ok);
        assert!(!set_monitor_colour("HDMI-A-1", "srgb", 9.0, 1.0, "").ok);
        assert!(!set_monitor_colour("HDMI-A-1", "srgb", 1.0, 1.0, "/etc/passwd\" }) os.exit() --.icc").ok);
    }
}
