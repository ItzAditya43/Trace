//! What kind of machine Trace is running on, and which of its features can
//! work here. Pages use this to hide what can't, and the "What works on
//! this machine" card lists the rest with what to install.

use crate::sys_util::{has_tool, read_trimmed, run_lossy};
use serde::Serialize;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Serialize, Clone)]
pub struct Platform {
    /// "hyprland" | "kde" | "gnome" | "sway" | "xfce" | "cinnamon" | "other"
    pub desktop: String,
    /// "wayland" | "x11" | "tty"
    pub session: String,
    /// "pacman" | "apt" | "dnf" | "zypper" | "none"
    pub packages: String,
    /// "firewalld" | "ufw" | "none"
    pub firewall: String,
    pub network_manager: bool,
    pub pipewire: bool,
    pub laptop: bool,
    pub distro: String,
    /// Window, monitor, colour and overlay control need Hyprland's IPC.
    pub hyprland: bool,
    pub features: Vec<Feature>,
}

#[derive(Serialize, Clone)]
pub struct Feature {
    pub name: String,
    pub available: bool,
    /// What it needs, shown when it isn't available.
    pub needs: String,
}

fn desktop() -> String {
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        return "hyprland".into();
    }
    if std::env::var("SWAYSOCK").is_ok() {
        return "sway".into();
    }
    let current = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    for (key, name) in [("kde", "kde"), ("gnome", "gnome"), ("xfce", "xfce"), ("x-cinnamon", "cinnamon"), ("cinnamon", "cinnamon"), ("hyprland", "hyprland"), ("sway", "sway")] {
        if current.contains(key) {
            return name.into();
        }
    }
    "other".into()
}

fn session() -> String {
    match std::env::var("XDG_SESSION_TYPE").unwrap_or_default().as_str() {
        "wayland" => "wayland".into(),
        "x11" => "x11".into(),
        _ if std::env::var("WAYLAND_DISPLAY").is_ok() => "wayland".into(),
        _ if std::env::var("DISPLAY").is_ok() => "x11".into(),
        _ => "tty".into(),
    }
}

fn package_manager() -> String {
    for tool in ["pacman", "apt-get", "dnf", "zypper"] {
        if has_tool(tool) {
            return tool.trim_end_matches("-get").into();
        }
    }
    "none".into()
}

fn active(unit: &str) -> bool {
    run_lossy(Command::new("systemctl").args(["is-active", unit])) == "active"
}

fn firewall() -> String {
    if has_tool("firewall-cmd") && active("firewalld") {
        "firewalld".into()
    } else if has_tool("ufw") && active("ufw") {
        "ufw".into()
    } else if has_tool("firewall-cmd") {
        "firewalld".into()
    } else if has_tool("ufw") {
        "ufw".into()
    } else {
        "none".into()
    }
}

fn is_laptop() -> bool {
    // Chassis types 8–14 and 30–32 are portables in the SMBIOS spec; a battery settles it either way.
    let chassis: u32 = read_trimmed("/sys/class/dmi/id/chassis_type").and_then(|v| v.parse().ok()).unwrap_or(0);
    matches!(chassis, 8..=14 | 30..=32) || crate::tweaks::expand_globs(&["/sys/class/power_supply/BAT*"]).into_iter().next().is_some()
}

fn feature(name: &str, available: bool, needs: &str) -> Feature {
    Feature { name: name.into(), available, needs: if available { String::new() } else { needs.into() } }
}

fn detect() -> Platform {
    let desktop = desktop();
    let hyprland = desktop == "hyprland" && crate::hypr::available();
    let packages = package_manager();
    let firewall = firewall();
    let network_manager = has_tool("nmcli") && active("NetworkManager");
    let pipewire = has_tool("pactl") && run_lossy(Command::new("pactl").arg("info")).contains("PipeWire");
    let laptop = is_laptop();
    let distro = std::fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
        .unwrap_or_else(|| "Linux".into());

    let features = vec![
        feature("Live monitoring, memory, history and logbook", true, ""),
        feature("Tweaks catalogue", true, ""),
        feature("Processes, services and timers", has_tool("systemctl"), "systemd"),
        feature("Window and workspace control", hyprland, "Hyprland"),
        feature(
            "Screen time per app and sleeping unused apps",
            hyprland || desktop == "sway" || session() == "x11" && has_tool("xprop") || desktop == "kde" && has_tool("kdotool"),
            if desktop == "gnome" { "a desktop that says which window is in front (GNOME on Wayland doesn't; X11 sessions do)" } else if desktop == "kde" { "kdotool" } else { "xprop (X11)" },
        ),
        feature("Monitor arrangement and colour", hyprland, "Hyprland"),
        feature("Click-through stats overlay", hyprland, "Hyprland (elsewhere it opens as an ordinary window)"),
        feature("Wi-Fi, VPNs and connection doctor", network_manager, "NetworkManager"),
        feature("Firewall ports", firewall != "none", "firewalld or ufw"),
        feature("Per-app volume, equaliser and mic filter", pipewire, "PipeWire with pipewire-pulse"),
        feature("Package updates and cleanup", packages != "none", "pacman, apt, dnf or zypper"),
        feature("Update briefing with distribution news", packages == "pacman", "Arch-based system"),
        feature("Known-vulnerable package check", packages == "pacman" && has_tool("vercmp"), "Arch-based system"),
        feature("Battery history, drain per app and charge limit", laptop, "a laptop"),
        feature("Drive health (SMART)", has_tool("smartctl"), "smartmontools"),
        feature("External monitor brightness", has_tool("ddcutil"), "ddcutil"),
        feature("NVIDIA card details", has_tool("nvidia-smi"), "nvidia-smi (NVIDIA driver)"),
        feature("Docker containers", has_tool("docker"), "Docker"),
        feature("Backups", has_tool("rsync"), "rsync"),
        feature("Frame rate in games", has_tool("mangohud"), "MangoHud"),
        feature("Run programs without file access", has_tool("bwrap"), "bubblewrap"),
        feature("Keyboard light timeout", crate::batch6::has_kbd_light(), "a keyboard backlight and brightnessctl"),
        feature("Phone alerts", has_tool("kdeconnect-cli"), "KDE Connect"),
        feature("Wi-Fi QR codes", has_tool("qrencode"), "qrencode"),
        feature("Password prompts for system changes", has_tool("pkexec"), "polkit (pkexec)"),
    ];
    Platform { desktop, session: session(), packages, firewall, network_manager, pipewire, laptop, distro, hyprland, features }
}

/// Detected once per run; none of this changes while Trace is open.
pub fn get() -> Platform {
    static PLATFORM: OnceLock<Platform> = OnceLock::new();
    PLATFORM.get_or_init(detect).clone()
}

// ---- the focused window, on whichever desktop ----

/// Finds the focused window's process in Sway's tree.
fn sway_focused(node: &serde_json::Value) -> Option<u32> {
    if node["focused"].as_bool() == Some(true) {
        return node["pid"].as_u64().map(|p| p as u32);
    }
    for key in ["nodes", "floating_nodes"] {
        for child in node[key].as_array().into_iter().flatten() {
            if let Some(pid) = sway_focused(child) {
                return Some(pid);
            }
        }
    }
    None
}

/// The process owning the window in front, where the desktop lets other
/// programs ask. GNOME on Wayland doesn't, so there it is unknown.
pub fn focused_pid() -> Option<u32> {
    let p = get();
    match p.desktop.as_str() {
        "hyprland" => {
            let json: serde_json::Value = serde_json::from_str(&run_lossy(Command::new("hyprctl").args(["activewindow", "-j"]))).ok()?;
            json["pid"].as_u64().map(|v| v as u32)
        }
        "sway" => sway_focused(&serde_json::from_str(&run_lossy(Command::new("swaymsg").args(["-t", "get_tree"]))).ok()?),
        "kde" if p.session == "wayland" && has_tool("kdotool") => run_lossy(Command::new("kdotool").args(["getactivewindow", "getwindowpid"])).trim().parse().ok(),
        _ if p.session == "x11" && has_tool("xprop") => {
            // "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007"
            let root = run_lossy(Command::new("xprop").args(["-root", "_NET_ACTIVE_WINDOW"]));
            let id = root.rsplit(' ').next()?.trim();
            let out = run_lossy(Command::new("xprop").args(["-id", id, "_NET_WM_PID"]));
            out.rsplit(' ').next()?.trim().parse().ok()
        }
        _ => None,
    }
}

/// Changes whenever the pointer moves, where that can be asked; empty otherwise.
pub fn pointer_position() -> String {
    let p = get();
    if p.desktop == "hyprland" {
        run_lossy(Command::new("hyprctl").arg("cursorpos"))
    } else if p.session == "x11" && has_tool("xdotool") {
        run_lossy(Command::new("xdotool").arg("getmouselocation"))
    } else {
        String::new()
    }
}

/// Counts of keyboard and touchpad interrupts: they move with every key
/// press and touch, on any desktop, without asking it anything.
pub fn input_interrupts() -> String {
    std::fs::read_to_string("/proc/interrupts")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("i8042") || l.contains("i2c") || l.to_lowercase().contains("touchpad"))
        .map(|l| l.split_whitespace().take(17).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_something_sensible_here() {
        let p = detect();
        assert!(["hyprland", "kde", "gnome", "sway", "xfce", "cinnamon", "other"].contains(&p.desktop.as_str()));
        assert!(["pacman", "apt", "dnf", "zypper", "none"].contains(&p.packages.as_str()));
        assert!(p.features.iter().all(|f| f.available || !f.needs.is_empty()), "every missing feature says what it needs");
        assert!(p.features.iter().any(|f| f.available));
    }
}
