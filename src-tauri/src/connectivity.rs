use crate::control_process::ActionResult;
use crate::sys_util::{err, has_tool, ok, run, run_lossy};
use serde::Serialize;
use std::process::Command;

fn action(result: Result<String, String>, success: impl Into<String>) -> ActionResult {
    match result {
        Ok(_) => ok(success),
        Err(e) => err(e),
    }
}

// ---- firewall (firewalld) ----

#[derive(Serialize)]
pub struct FirewallStatus {
    pub available: bool,
    pub running: bool,
    pub zone: String,
    pub services: Vec<String>,
    pub ports: Vec<String>,
}

/// One `--list-all` call covers everything: `firewall-cmd` is a Python
/// client that takes seconds to start, so separate queries would add up.
pub fn firewall_status() -> FirewallStatus {
    let available = has_tool("firewall-cmd");
    let out = if available {
        run(Command::new("firewall-cmd").arg("--list-all")).ok()
    } else {
        None
    };
    let text = out.clone().unwrap_or_default();
    let list = |key: &str| -> Vec<String> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| v.split_whitespace().map(String::from).collect())
            .unwrap_or_default()
    };
    FirewallStatus {
        available,
        running: out.is_some(),
        // First line: "public (default, active)".
        zone: text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or_default()
            .to_string(),
        services: list("services:"),
        ports: list("ports:"),
    }
}

/// Opens or closes an inbound port in the default zone. firewalld asks for
/// authorization itself; the change lasts until the firewall reloads unless
/// `permanent` is set.
pub fn set_firewall_port(port: u16, protocol: &str, open: bool, permanent: bool) -> ActionResult {
    if !matches!(protocol, "tcp" | "udp") || port == 0 {
        return err("Port must be 1-65535 and protocol tcp or udp");
    }
    let flag = format!("--{}-port={port}/{protocol}", if open { "add" } else { "remove" });
    let verb = if open { "Opened" } else { "Closed" };
    let mut result = run(Command::new("firewall-cmd").arg(&flag));
    if permanent && result.is_ok() {
        result = run(Command::new("firewall-cmd").args(["--permanent", &flag]));
    }
    action(
        result,
        format!(
            "{verb} {port}/{protocol}{}",
            if permanent { " (permanent)" } else { " (until the firewall reloads)" }
        ),
    )
}

// ---- radios (Wi-Fi / Bluetooth kill switches) ----

#[derive(Serialize)]
pub struct Radio {
    /// "wlan" | "bluetooth" | ...
    pub kind: String,
    pub enabled: bool,
    /// Switched off by a physical key/switch; software can't turn it on.
    pub hard_blocked: bool,
}

pub fn list_radios() -> Vec<Radio> {
    let out = run_lossy(Command::new("rfkill").args(["-J", "-o", "TYPE,SOFT,HARD"]));
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) else {
        return Vec::new();
    };
    let mut radios: Vec<Radio> = Vec::new();
    // Field name changed from "" to "rfkilldevices" across util-linux versions.
    let devices = json.as_object().and_then(|o| o.values().next()).and_then(|v| v.as_array());
    for device in devices.into_iter().flatten() {
        let kind = device["type"].as_str().unwrap_or_default().to_string();
        let blocked = device["soft"].as_str() == Some("blocked");
        let hard = device["hard"].as_str() == Some("blocked");
        // A laptop often lists the same radio twice (driver + platform switch).
        if let Some(existing) = radios.iter_mut().find(|r| r.kind == kind) {
            existing.enabled &= !blocked;
            existing.hard_blocked |= hard;
        } else {
            radios.push(Radio { kind, enabled: !blocked, hard_blocked: hard });
        }
    }
    radios
}

pub fn set_radio(kind: &str, enabled: bool) -> ActionResult {
    if !matches!(kind, "wlan" | "bluetooth" | "wwan" | "nfc") {
        return err(format!("Unknown radio {kind}"));
    }
    let verb = if enabled { "unblock" } else { "block" };
    action(
        run(Command::new("rfkill").args([verb, kind])),
        format!("{} turned {}", if kind == "wlan" { "Wi-Fi" } else { kind }, if enabled { "on" } else { "off" }),
    )
}

// ---- Wi-Fi ----

#[derive(Serialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub connected: bool,
    /// A saved profile exists, so connecting needs no password.
    pub known: bool,
}

/// Splits one line of `nmcli -t` output, which escapes literal colons as `\:`.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut().unwrap().extend(chars.next()),
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

pub fn list_wifi(rescan: bool) -> Vec<WifiNetwork> {
    let known: Vec<String> = run_lossy(Command::new("nmcli").args(["-t", "-f", "NAME", "connection", "show"]))
        .lines()
        .map(|l| split_terse(l).remove(0))
        .collect();
    let out = run_lossy(Command::new("nmcli").args([
        "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list",
        "--rescan", if rescan { "yes" } else { "no" },
    ]));
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for line in out.lines() {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let network = WifiNetwork {
            connected: f[0] == "*",
            known: known.contains(&f[1]),
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            security: f[3].clone(),
        };
        // The same network shows once per access point; keep the best entry.
        match networks.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(existing) => {
                existing.connected |= network.connected;
                existing.signal = existing.signal.max(network.signal);
            }
            None => networks.push(network),
        }
    }
    networks.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)));
    networks
}

pub fn connect_wifi(ssid: &str, password: Option<&str>) -> ActionResult {
    let mut cmd = Command::new("nmcli");
    // Arguments are passed directly (no shell), so any SSID is safe here.
    cmd.args(["--wait", "20", "dev", "wifi", "connect", ssid]);
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        cmd.args(["password", password]);
    }
    action(run(&mut cmd), format!("Connected to {ssid}"))
}

// ---- Bluetooth ----

#[derive(Serialize)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub connected: bool,
}

fn is_mac(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6 && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}

pub fn list_bluetooth() -> Vec<BluetoothDevice> {
    let connected: Vec<String> = run_lossy(Command::new("bluetoothctl").args(["devices", "Connected"]))
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(String::from))
        .collect();
    run_lossy(Command::new("bluetoothctl").arg("devices"))
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, ' ');
            if parts.next()? != "Device" {
                return None;
            }
            let address = parts.next()?.to_string();
            Some(BluetoothDevice {
                connected: connected.contains(&address),
                name: parts.next().unwrap_or(&address).to_string(),
                address,
            })
        })
        .collect()
}

pub fn set_bluetooth_connected(address: &str, connect: bool) -> ActionResult {
    if !is_mac(address) {
        return err("Not a Bluetooth address");
    }
    let verb = if connect { "connect" } else { "disconnect" };
    action(
        run(Command::new("bluetoothctl").args(["--timeout", "15", verb, address])),
        format!("{}ed {address}", if connect { "Connect" } else { "Disconnect" }),
    )
}

// ---- display (Hyprland) ----

#[derive(Serialize)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub mode: String,
    pub scale: f64,
    pub available_modes: Vec<String>,
}

fn hyprctl_json(args: &[&str]) -> Option<serde_json::Value> {
    serde_json::from_str(&run(Command::new("hyprctl").args(args)).ok()?).ok()
}

pub fn list_monitors() -> Vec<Monitor> {
    let Some(json) = hyprctl_json(&["monitors", "-j"]) else {
        return Vec::new();
    };
    json.as_array()
        .into_iter()
        .flatten()
        .map(|m| Monitor {
            name: m["name"].as_str().unwrap_or_default().to_string(),
            description: m["description"].as_str().unwrap_or_default().to_string(),
            mode: format!(
                "{}x{}@{:.2}Hz",
                m["width"], m["height"], m["refreshRate"].as_f64().unwrap_or(0.0)
            ),
            scale: m["scale"].as_f64().unwrap_or(1.0),
            available_modes: {
                let mut modes: Vec<String> = m["availableModes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                // Some panels advertise the same mode more than once.
                let mut seen = std::collections::HashSet::new();
                modes.retain(|mode| seen.insert(mode.clone()));
                modes
            },
        })
        .collect()
}

/// Switches a monitor to one of the modes it advertises. Applies to the
/// running session only — your Hyprland config is untouched, so a reload or
/// re-login restores the configured mode.
pub fn set_monitor_mode(name: &str, mode: &str, scale: f64) -> ActionResult {
    let Some(monitor) = list_monitors().into_iter().find(|m| m.name == name) else {
        return err(format!("No monitor named {name}"));
    };
    if !monitor.available_modes.iter().any(|m| m == mode) {
        return err(format!("{name} doesn't support {mode}"));
    }
    if !(0.5..=3.0).contains(&scale) {
        return err("Scale must be between 0.5 and 3");
    }
    let spec = format!("{name},{},auto,{scale}", mode.trim_end_matches("Hz"));
    action(
        run(Command::new("hyprctl").args(["keyword", "monitor", &spec])),
        format!("{name} set to {mode} at {scale}× scale"),
    )
}

/// Current night-light colour temperature in Kelvin (6500 ≈ off), via hyprsunset.
pub fn night_light() -> Option<u32> {
    run(Command::new("hyprctl").args(["hyprsunset", "temperature"]))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn set_night_light(kelvin: Option<u32>) -> ActionResult {
    let (arg, message) = match kelvin {
        Some(k) => {
            let k = k.clamp(1000, 6500);
            (vec!["hyprsunset".to_string(), "temperature".to_string(), k.to_string()], format!("Night light set to {k} K"))
        }
        None => (vec!["hyprsunset".to_string(), "identity".to_string()], "Night light off".to_string()),
    };
    match run(Command::new("hyprctl").args(&arg)) {
        Ok(out) if !out.to_lowercase().contains("error") && !out.contains("not running") => ok(message),
        Ok(out) => err(format!("hyprsunset isn't running ({out})")),
        Err(e) => err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nmcli_terse_fields_with_escaped_colons() {
        assert_eq!(split_terse(r"*:peak bitch:80:WPA2"), ["*", "peak bitch", "80", "WPA2"]);
        assert_eq!(split_terse(r":my\:net:55:"), ["", "my:net", "55", ""]);
    }

    #[test]
    fn validates_bluetooth_addresses() {
        assert!(is_mac("90:A0:BE:E7:D1:D7"));
        assert!(!is_mac("90:A0:BE:E7:D1"));
        assert!(!is_mac("--help"));
    }
}
