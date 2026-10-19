use crate::control_process::ActionResult;
use std::fs;
use std::process::Command;

pub fn ok(msg: impl Into<String>) -> ActionResult {
    ActionResult {
        ok: true,
        message: msg.into(),
    }
}

pub fn err(msg: impl Into<String>) -> ActionResult {
    ActionResult {
        ok: false,
        message: msg.into(),
    }
}

pub fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("exited with {}", out.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Like `run`, but returns stdout even when the command exits non-zero —
/// for tools such as `checkupdates` or `systemctl --failed` whose exit code
/// encodes "nothing found" rather than a real failure.
pub fn run_lossy(cmd: &mut Command) -> String {
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Runs a shell script as root through polkit (`pkexec`), which shows the
/// desktop's own password prompt. Callers must build `script` only from
/// validated values (numbers, whitelisted names) — never raw UI input.
pub fn run_privileged(script: &str) -> Result<String, String> {
    run(Command::new("pkexec").args(["sh", "-c", script])).map_err(|e| {
        if e.contains("dismissed") || e.contains("Not authorized") {
            "Authorization was cancelled".to_string()
        } else {
            e
        }
    })
}

pub fn privileged_action(script: &str, success: impl Into<String>) -> ActionResult {
    match run_privileged(script) {
        Ok(_) => ok(success),
        Err(e) => err(e),
    }
}

pub fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_num<T: std::str::FromStr>(path: &str) -> Option<T> {
    read_trimmed(path)?.parse().ok()
}

pub fn has_tool(name: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join(name).exists())
}

/// True for names safe to splice into a privileged shell script.
pub fn is_safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':' | '/'))
}

pub struct SentNotification {
    pub ts: i64,
    pub title: String,
    pub body: String,
    pub critical: bool,
}

/// Notifications sent but not yet written to the logbook.
static SENT: std::sync::Mutex<Vec<SentNotification>> = std::sync::Mutex::new(Vec::new());

pub fn take_notifications() -> Vec<SentNotification> {
    std::mem::take(&mut *SENT.lock().unwrap())
}

/// While set, only critical notifications are shown (focus time).
pub static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn notify(title: &str, body: &str, critical: bool) {
    // Unit tests exercise rules and alerts; they shouldn't pop real notifications.
    if cfg!(test) {
        return;
    }
    if !critical && QUIET.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    SENT.lock().unwrap().push(SentNotification {
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        title: title.to_string(),
        body: body.to_string(),
        critical,
    });
    let ntfy = crate::batch6::extras().ntfy_url;
    if !ntfy.is_empty() {
        crate::batch7::send_ntfy(&ntfy, title, body, critical);
    }
    Command::new("notify-send")
        .args([
            "--app-name=Trace",
            "--urgency",
            if critical { "critical" } else { "normal" },
            title,
            body,
        ])
        .spawn()
        .map(|mut child| {
            // Collect it when it exits, or it stays behind as a dead entry for as long as Trace runs.
            std::thread::spawn(move || child.wait().ok());
        })
        .ok();
}

pub struct LocalTime {
    pub hour: u32,
    pub minute: u32,
    /// 0 = Sunday.
    pub weekday: u32,
    /// Day of the year, used to make sure a schedule fires once per day.
    pub yday: u32,
}

pub fn local_time() -> LocalTime {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        LocalTime {
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            weekday: tm.tm_wday as u32,
            yday: tm.tm_yday as u32,
        }
    }
}

pub fn data_dir() -> std::path::PathBuf {
    let mut dir = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("trace");
    fs::create_dir_all(&dir).ok();
    dir
}
