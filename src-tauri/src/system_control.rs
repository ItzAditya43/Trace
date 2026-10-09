use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

fn ok(msg: impl Into<String>) -> ActionResult {
    ActionResult {
        ok: true,
        message: msg.into(),
    }
}

fn err(msg: impl Into<String>) -> ActionResult {
    ActionResult {
        ok: false,
        message: msg.into(),
    }
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---- ionice ----

pub fn set_ionice(pid: u32, class: u32, level: u32) -> ActionResult {
    let class_name = match class {
        1 => "realtime",
        2 => "best-effort",
        3 => "idle",
        _ => "best-effort",
    };
    match run(Command::new("ionice").args([
        "-c",
        &class.to_string(),
        "-n",
        &level.to_string(),
        "-p",
        &pid.to_string(),
    ])) {
        Ok(_) => ok(format!(
            "Set PID {pid} IO scheduling to {class_name} (level {level})"
        )),
        Err(e) => err(format!("Failed to set ionice for PID {pid}: {e}")),
    }
}

// ---- brightness ----

#[derive(Serialize)]
pub struct BrightnessInfo {
    pub device: String,
    pub percent: u32,
}

pub fn get_brightness() -> Option<BrightnessInfo> {
    let out = run(Command::new("brightnessctl").arg("-m")).ok()?;
    // format: device,class,current,percent%,max
    let fields: Vec<&str> = out.lines().next()?.split(',').collect();
    let device = fields.first()?.to_string();
    let percent = fields.get(3)?.trim_end_matches('%').parse().ok()?;
    Some(BrightnessInfo { device, percent })
}

pub fn set_brightness(percent: u32) -> ActionResult {
    let clamped = percent.clamp(1, 100);
    match run(Command::new("brightnessctl").args(["set", &format!("{clamped}%")])) {
        Ok(_) => ok(format!("Set brightness to {clamped}%")),
        Err(e) => err(format!("Failed to set brightness: {e}")),
    }
}

// ---- volume ----

#[derive(Serialize)]
pub struct VolumeInfo {
    pub percent: u32,
    pub muted: bool,
}

pub fn get_volume() -> Option<VolumeInfo> {
    let out = run(Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"])).ok()?;
    // format: "Volume: 0.49" or "Volume: 0.49 [MUTED]"
    let muted = out.contains("MUTED");
    let value: f32 = out
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(VolumeInfo {
        percent: (value * 100.0).round() as u32,
        muted,
    })
}

pub fn set_volume(percent: u32) -> ActionResult {
    let clamped = percent.clamp(0, 150);
    match run(Command::new("wpctl").args([
        "set-volume",
        "@DEFAULT_AUDIO_SINK@",
        &format!("{}%", clamped),
    ])) {
        Ok(_) => ok(format!("Set volume to {clamped}%")),
        Err(e) => err(format!("Failed to set volume: {e}")),
    }
}

pub fn toggle_mute() -> ActionResult {
    match run(Command::new("wpctl").args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])) {
        Ok(_) => ok("Toggled mute"),
        Err(e) => err(format!("Failed to toggle mute: {e}")),
    }
}

// ---- startup impact ----

#[derive(Serialize)]
pub struct StartupEntry {
    pub unit: String,
    pub time_ms: u64,
}

pub fn startup_impact() -> Vec<StartupEntry> {
    let Ok(out) = run(Command::new("systemd-analyze").args(["blame", "--no-pager"])) else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|line| {
            // "6min 50.113s some.service": the duration can be several
            // tokens, so the unit is whatever follows the last one.
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let unit_at = tokens.iter().position(|t| !is_duration_token(t))?;
            if unit_at == 0 {
                return None;
            }
            Some(StartupEntry {
                unit: tokens[unit_at..].join(" "),
                time_ms: crate::health::parse_duration_ms(&tokens[..unit_at].join(" ")),
            })
        })
        .take(30)
        .collect()
}

fn is_duration_token(token: &str) -> bool {
    let digits = token.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &token[digits.len()..];
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && matches!(suffix, "h" | "min" | "s" | "ms")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_multi_token_durations() {
        assert!(is_duration_token("6min"));
        assert!(is_duration_token("50.113s"));
        assert!(is_duration_token("234ms"));
        assert!(!is_duration_token("archlinux-keyring-wkd-sync.service"));
        assert!(!is_duration_token("3s.service"));
    }
}

// ---- autostart ----

#[derive(Serialize)]
pub struct AutostartEntry {
    pub filename: String,
    pub name: String,
    pub enabled: bool,
    pub system_wide: bool,
}

fn autostart_dirs() -> (String, String) {
    let home = std::env::var("HOME").unwrap_or_default();
    (format!("{home}/.config/autostart"), "/etc/xdg/autostart".to_string())
}

fn parse_desktop_name(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("Name="))
        .map(|l| l.trim_start_matches("Name=").to_string())
}

fn is_disabled(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false"
    })
}

pub fn list_autostart() -> Vec<AutostartEntry> {
    let (user_dir, system_dir) = autostart_dirs();
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();

    // User overrides take priority since they're read first.
    for dir in [user_dir.as_str(), system_dir.as_str()] {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        for entry in rd.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".desktop") || !seen.insert(filename.clone()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(entry.path()) else {
                continue;
            };
            entries.push(AutostartEntry {
                name: parse_desktop_name(&text).unwrap_or_else(|| filename.clone()),
                enabled: !is_disabled(&text),
                system_wide: dir == system_dir,
                filename,
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

pub fn set_autostart_enabled(filename: &str, enabled: bool) -> ActionResult {
    let (user_dir, system_dir) = autostart_dirs();
    fs::create_dir_all(&user_dir).ok();
    let user_path = format!("{user_dir}/{filename}");

    let base_text = fs::read_to_string(&user_path)
        .or_else(|_| fs::read_to_string(format!("{system_dir}/{filename}")))
        .unwrap_or_default();

    if base_text.is_empty() {
        return err(format!("Could not find autostart entry {filename}"));
    }

    let mut lines: Vec<String> = base_text
        .lines()
        .filter(|l| {
            let l = l.trim();
            l != "Hidden=true" && l != "X-GNOME-Autostart-enabled=false"
        })
        .map(String::from)
        .collect();

    if !enabled {
        lines.push("Hidden=true".to_string());
    }

    match fs::write(&user_path, lines.join("\n") + "\n") {
        Ok(()) => ok(format!(
            "{} {filename} at startup",
            if enabled { "Enabled" } else { "Disabled" }
        )),
        Err(e) => err(format!("Failed to write {user_path}: {e}")),
    }
}
