//! Starting Trace at login.
//!
//! Desktops disagree about how this is done. Hyprland only starts what its
//! own config tells it to, so there Trace adds a clearly marked block to the
//! config; everywhere else the standard XDG autostart entry is used.

use crate::control_process::ActionResult;
use crate::sys_util::{err, ok};
use serde::Serialize;
use std::path::{Path, PathBuf};

const BEGIN: &str = ">>> trace autostart (added by Trace; remove this block to undo)";
const END: &str = "<<< trace autostart";

#[derive(Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    /// The file Trace edits (or would edit) for this.
    pub file: String,
    pub method: String,
}

enum Method {
    HyprLua(PathBuf),
    HyprConf(PathBuf),
    Xdg(PathBuf),
}

fn method() -> Option<Method> {
    let config = dirs::config_dir()?;
    if crate::hypr::available() {
        let hypr = config.join("hypr");
        // A "custom" file that the main config already loads is the least
        // intrusive place; fall back to the main config itself.
        for candidate in ["custom/execs.lua", "hyprland.lua"] {
            if crate::hypr::is_lua() && hypr.join(candidate).is_file() {
                return Some(Method::HyprLua(hypr.join(candidate)));
            }
        }
        if hypr.join("hyprland.conf").is_file() {
            return Some(Method::HyprConf(hypr.join("hyprland.conf")));
        }
    }
    Some(Method::Xdg(config.join("autostart/trace.desktop")))
}

/// Removes any existing managed block from `text`, then appends `block`
/// (already wrapped in its marker comments) if one is given.
fn with_block(text: &str, comment: &str, body: Option<&str>) -> String {
    let begin = format!("{comment} {BEGIN}");
    let end = format!("{comment} {END}");
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == begin {
            inside = true;
        } else if line.trim() == end {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    let mut out = out.trim_end().to_string();
    if !out.is_empty() {
        out.push('\n');
    }
    if let Some(body) = body {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{begin}\n{body}\n{end}\n"));
    }
    out
}

fn has_block(path: &Path, comment: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.lines().any(|l| l.trim() == format!("{comment} {BEGIN}")))
}

pub fn status() -> AutostartStatus {
    match method() {
        Some(Method::HyprLua(path)) => AutostartStatus {
            enabled: has_block(&path, "--"),
            file: path.to_string_lossy().to_string(),
            method: "Hyprland config".to_string(),
        },
        Some(Method::HyprConf(path)) => AutostartStatus {
            enabled: has_block(&path, "#"),
            file: path.to_string_lossy().to_string(),
            method: "Hyprland config".to_string(),
        },
        Some(Method::Xdg(path)) => AutostartStatus {
            enabled: path.is_file(),
            file: path.to_string_lossy().to_string(),
            method: "desktop autostart entry".to_string(),
        },
        None => AutostartStatus { enabled: false, file: String::new(), method: "unavailable".to_string() },
    }
}

pub fn set(enabled: bool) -> ActionResult {
    let Ok(exe) = std::env::current_exe() else {
        return err("Couldn't work out where Trace is installed");
    };
    let exe = exe.to_string_lossy().to_string();
    // The path is written into a config file inside quotes.
    if exe.contains(['"', '\\', '\n', '\'']) {
        return err("Trace's install path contains characters that can't be written into a config file");
    }
    let Some(method) = method() else {
        return err("No config folder found");
    };
    let done = |path: &Path| {
        ok(if enabled {
            format!("Trace will start hidden at login (added to {})", path.display())
        } else {
            format!("Trace no longer starts at login (removed from {})", path.display())
        })
    };
    let rewrite = |path: &Path, comment: &str, body: String| {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let updated = with_block(&text, comment, enabled.then_some(body.as_str()));
        match std::fs::write(path, updated) {
            Ok(()) => done(path),
            Err(e) => err(format!("Couldn't write {}: {e}", path.display())),
        }
    };
    match method {
        Method::HyprLua(path) => rewrite(
            &path,
            "--",
            format!("hl.on(\"hyprland.start\", function()\n    hl.exec_cmd(\"{exe} --hidden\")\nend)"),
        ),
        Method::HyprConf(path) => rewrite(&path, "#", format!("exec-once = {exe} --hidden")),
        Method::Xdg(path) => {
            if !enabled {
                return match std::fs::remove_file(&path) {
                    Ok(()) => done(&path),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => done(&path),
                    Err(e) => err(format!("Couldn't remove {}: {e}", path.display())),
                };
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName=Trace\nComment=Machine observatory\nExec=\"{exe}\" --hidden\nX-GNOME-Autostart-enabled=true\n"
            );
            match std::fs::write(&path, entry) {
                Ok(()) => done(&path),
                Err(e) => err(format!("Couldn't write {}: {e}", path.display())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_replaces_and_removes_only_its_own_block() {
        let original = "-- my config\nhl.exec_cmd(\"something\")\n";
        let added = with_block(original, "--", Some("hl.exec_cmd(\"/usr/bin/trace --hidden\")"));
        assert!(added.starts_with(original));
        assert!(added.contains("trace --hidden"));

        // Adding again replaces rather than duplicates.
        let again = with_block(&added, "--", Some("hl.exec_cmd(\"/opt/trace --hidden\")"));
        assert_eq!(again.matches(BEGIN).count(), 1);
        assert!(again.contains("/opt/trace") && !again.contains("/usr/bin/trace"));

        // Removing leaves the user's own lines exactly as they were.
        assert_eq!(with_block(&again, "--", None), original);
        assert_eq!(with_block("", "#", None), "");
    }
}
