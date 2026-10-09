use crate::control_process::ActionResult;
use crate::sys_util::{err, has_tool, ok, privileged_action, run, run_lossy};
use serde::Serialize;
use std::process::Command;

// ---- opening things ----

/// Opens a folder in the editor, file manager or a terminal. `path` must be
/// an existing directory; it is passed as an argument, never through a shell.
pub fn open_path(path: &str, with: &str) -> ActionResult {
    if !std::path::Path::new(path).is_dir() {
        return err(format!("{path} is not a folder"));
    }
    let mut cmd = match with {
        "editor" => {
            let Some(editor) = ["code", "codium", "zed"].into_iter().find(|e| has_tool(e)) else {
                return err("No supported editor found (code, codium, zed)");
            };
            let mut c = Command::new(editor);
            c.arg(path);
            c
        }
        "files" => {
            let mut c = Command::new("xdg-open");
            c.arg(path);
            c
        }
        "terminal" => {
            let preferred = std::env::var("TERMINAL").unwrap_or_default();
            let Some(terminal) = [preferred.as_str(), "kitty", "foot", "alacritty", "ghostty", "wezterm", "konsole"]
                .into_iter()
                .find(|t| !t.is_empty() && has_tool(t))
            else {
                return err("No terminal emulator found");
            };
            let mut c = Command::new(terminal);
            c.current_dir(path);
            c
        }
        _ => return err(format!("Unknown opener {with}")),
    };
    use std::os::unix::process::CommandExt;
    match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(_) => ok(format!("Opened {path}")),
        Err(e) => err(format!("Couldn't open {path}: {e}")),
    }
}

// ---- session & power ----

pub fn session_action(action: &str) -> ActionResult {
    let session = std::env::var("XDG_SESSION_ID").unwrap_or_default();
    let (mut cmd, done) = match action {
        "lock" => {
            let mut c = Command::new("loginctl");
            c.arg("lock-session");
            (c, "Screen locked")
        }
        "suspend" => {
            let mut c = Command::new("systemctl");
            c.arg("suspend");
            (c, "Suspending")
        }
        "reboot" => {
            let mut c = Command::new("systemctl");
            c.arg("reboot");
            (c, "Rebooting")
        }
        "poweroff" => {
            let mut c = Command::new("systemctl");
            c.arg("poweroff");
            (c, "Shutting down")
        }
        "logout" => {
            let mut c = Command::new("loginctl");
            c.arg("terminate-session").arg(&session);
            (c, "Logging out")
        }
        _ => return err(format!("Unknown session action {action}")),
    };
    match run(&mut cmd) {
        Ok(_) => ok(done),
        Err(e) => err(format!("{action} failed: {e}")),
    }
}

// ---- audio devices ----

#[derive(Serialize)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub muted: bool,
}

#[derive(Serialize)]
pub struct AudioDevices {
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
}

fn audio_list(kind: &str) -> Vec<AudioDevice> {
    let default = run_lossy(Command::new("pactl").arg(format!("get-default-{kind}")));
    let out = run_lossy(Command::new("pactl").args(["-f", "json", "list", &format!("{kind}s")]));
    let Ok(serde_json::Value::Array(devices)) = serde_json::from_str(&out) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|d| {
            let name = d["name"].as_str()?.to_string();
            // Every output also exposes a loopback "monitor" source; hide those.
            if name.ends_with(".monitor") {
                return None;
            }
            Some(AudioDevice {
                is_default: name == default,
                description: d["description"].as_str().unwrap_or(&name).to_string(),
                muted: d["mute"].as_bool().unwrap_or(false),
                name,
            })
        })
        .collect()
}

pub fn audio_devices() -> AudioDevices {
    AudioDevices {
        outputs: audio_list("sink"),
        inputs: audio_list("source"),
    }
}

pub fn set_default_audio(kind: &str, name: &str) -> ActionResult {
    if !matches!(kind, "sink" | "source") {
        return err("Unknown audio device kind");
    }
    match run(Command::new("pactl").args([&format!("set-default-{kind}"), name])) {
        Ok(_) => ok(format!(
            "{} switched",
            if kind == "sink" { "Output" } else { "Input" }
        )),
        Err(e) => err(e),
    }
}

// ---- cleanup ----

#[derive(Serialize)]
pub struct CleanupStatus {
    pub trash_bytes: u64,
    pub thumbnails_bytes: u64,
    pub journal_bytes: u64,
    pub coredump_bytes: u64,
    pub user_cache_bytes: u64,
}

fn du(path: &std::path::Path) -> u64 {
    run_lossy(Command::new("du").arg("-sb").arg(path))
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn trash_dir() -> Option<std::path::PathBuf> {
    Some(dirs::data_local_dir()?.join("Trash"))
}

fn thumbnails_dir() -> Option<std::path::PathBuf> {
    Some(dirs::cache_dir()?.join("thumbnails"))
}

/// "…take up 3.9G in the file system." -> bytes
fn parse_journal_usage(text: &str) -> u64 {
    let Some(token) = text
        .split_whitespace()
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
    else {
        return 0;
    };
    let split = token.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(token.len());
    let value: f64 = token[..split].parse().unwrap_or(0.0);
    let scale = match token[split..].chars().next() {
        Some('K') => 1024.0,
        Some('M') => 1024.0 * 1024.0,
        Some('G') => 1024.0 * 1024.0 * 1024.0,
        Some('T') => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (value * scale) as u64
}

pub fn cleanup_status() -> CleanupStatus {
    CleanupStatus {
        trash_bytes: trash_dir().map(|d| du(&d)).unwrap_or(0),
        thumbnails_bytes: thumbnails_dir().map(|d| du(&d)).unwrap_or(0),
        journal_bytes: parse_journal_usage(&run_lossy(Command::new("journalctl").arg("--disk-usage"))),
        coredump_bytes: du(std::path::Path::new("/var/lib/systemd/coredump")),
        user_cache_bytes: dirs::cache_dir().map(|d| du(&d)).unwrap_or(0),
    }
}

fn empty_dir(dir: &std::path::Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        // symlink_metadata: never follow a link out of the folder being emptied.
        let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or(false)
            && !path.is_symlink();
        let result = if is_dir {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if result.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// `target`: "trash" | "thumbnails" | "journal" | "coredumps".
pub fn clean(target: &str) -> ActionResult {
    match target {
        "trash" => {
            let Some(trash) = trash_dir() else {
                return err("No trash folder");
            };
            let files = empty_dir(&trash.join("files")).unwrap_or(0);
            empty_dir(&trash.join("info")).ok();
            ok(format!("Emptied the trash ({files} item(s))"))
        }
        "thumbnails" => match thumbnails_dir().map(|d| empty_dir(&d)) {
            Some(Ok(_)) => ok("Cleared cached thumbnails — they regenerate as you browse"),
            _ => err("No thumbnail cache to clear"),
        },
        "journal" => privileged_action(
            "journalctl --vacuum-size=500M",
            "Trimmed the system journal to 500 MB (oldest entries removed)",
        ),
        "coredumps" => privileged_action(
            "find /var/lib/systemd/coredump -type f -delete",
            "Deleted saved crash dumps (the crash list in Logs is kept)",
        ),
        _ => err(format!("Unknown cleanup target {target}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_journal_disk_usage() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 3.9G in the file system."),
            (3.9 * gib) as u64
        );
        assert_eq!(parse_journal_usage("nothing here"), 0);
    }

    #[test]
    fn empty_dir_removes_contents_but_not_link_targets() {
        let base = std::env::temp_dir().join(format!("trace-empty-test-{}", std::process::id()));
        let (target, keep) = (base.join("target"), base.join("keep"));
        std::fs::create_dir_all(target.join("sub")).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("precious"), "x").unwrap();
        std::fs::write(target.join("file"), "x").unwrap();
        std::os::unix::fs::symlink(&keep, target.join("link")).unwrap();

        assert_eq!(empty_dir(&target).unwrap(), 3);
        assert!(std::fs::read_dir(&target).unwrap().next().is_none());
        assert!(keep.join("precious").exists(), "symlink target must survive");
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn open_path_rejects_non_directories() {
        assert!(!open_path("/definitely/not/here", "files").ok);
        assert!(!open_path("/tmp", "nonsense").ok);
    }
}
