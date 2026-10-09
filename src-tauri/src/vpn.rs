//! VPN connections, managed through NetworkManager so they show up in (and
//! behave like) every other VPN on the system. Includes a browser for VPN
//! Gate's public server list that turns a server into a saved connection.

use crate::control_process::ActionResult;
use crate::sys_util::{data_dir, err, ok, run, run_lossy};
use base64::Engine;
use serde::Serialize;
use std::process::Command;
use std::sync::Mutex;

#[derive(Serialize)]
pub struct VpnConnection {
    pub name: String,
    pub uuid: String,
    /// "OpenVPN" | "WireGuard" | other plugin name
    pub kind: String,
    pub active: bool,
    pub autoconnect: bool,
    pub remote: String,
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

/// Pulls one `key = value` out of NetworkManager's comma-separated `vpn.data`.
fn vpn_data_value(data: &str, key: &str) -> Option<String> {
    data.split(", ").find_map(|pair| {
        let (k, v) = pair.split_once(" = ")?;
        (k.trim() == key).then(|| v.replace('\\', ""))
    })
}

pub fn list() -> Vec<VpnConnection> {
    let out = run_lossy(Command::new("nmcli").args([
        "-t", "-f", "NAME,UUID,TYPE,ACTIVE,AUTOCONNECT", "connection", "show",
    ]));
    let mut connections: Vec<VpnConnection> = out
        .lines()
        .filter_map(|line| {
            let f = split_terse(line);
            if f.len() < 5 || !matches!(f[2].as_str(), "vpn" | "wireguard") {
                return None;
            }
            let wireguard = f[2] == "wireguard";
            let field = if wireguard { "wireguard.peers" } else { "vpn.data,vpn.service-type" };
            let detail = run_lossy(Command::new("nmcli").args(["-g", field, "connection", "show", &f[1]]));
            let kind = if wireguard {
                "WireGuard".to_string()
            } else {
                // "org.freedesktop.NetworkManager.openvpn" -> "OpenVPN"
                match detail.lines().last().and_then(|l| l.rsplit('.').next()) {
                    Some("openvpn") => "OpenVPN".to_string(),
                    Some(other) if !other.is_empty() => other.to_string(),
                    _ => "VPN".to_string(),
                }
            };
            Some(VpnConnection {
                remote: if wireguard {
                    String::new()
                } else {
                    // `detail` is two lines: vpn.data, then the service type.
                    vpn_data_value(detail.lines().next().unwrap_or(""), "remote").unwrap_or_default()
                },
                name: f[0].clone(),
                uuid: f[1].clone(),
                kind,
                active: f[3] == "yes",
                autoconnect: f[4] == "yes",
            })
        })
        .collect();
    connections.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    connections
}

fn nmcli(args: &[&str], success: impl Into<String>) -> ActionResult {
    match run(Command::new("nmcli").args(args)) {
        Ok(_) => ok(success),
        Err(e) => err(e),
    }
}

pub fn set_active(uuid: &str, up: bool) -> ActionResult {
    let verb = if up { "up" } else { "down" };
    nmcli(
        &["--wait", "40", "connection", verb, "uuid", uuid],
        if up { "VPN connected" } else { "VPN disconnected" },
    )
}

pub fn set_autoconnect(uuid: &str, enabled: bool) -> ActionResult {
    nmcli(
        &["connection", "modify", "uuid", uuid, "connection.autoconnect", if enabled { "yes" } else { "no" }],
        if enabled { "Will connect automatically" } else { "Won't connect automatically" },
    )
}

pub fn rename(uuid: &str, name: &str) -> ActionResult {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return err("Give the VPN a name of 1–80 characters");
    }
    nmcli(&["connection", "modify", "uuid", uuid, "connection.id", name], format!("Renamed to {name}"))
}

pub fn delete(uuid: &str) -> ActionResult {
    nmcli(&["connection", "delete", "uuid", uuid], "VPN removed")
}

/// "Connection 'x' (0b1c…) successfully added." -> the UUID
fn uuid_from_import(output: &str) -> Option<String> {
    let start = output.rfind('(')? + 1;
    let end = output[start..].find(')')? + start;
    let uuid = &output[start..end];
    (uuid.len() == 36 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| uuid.to_string())
}

/// Imports an OpenVPN (`.ovpn`) or WireGuard (`.conf`) file as a saved
/// connection and returns its UUID. Nothing is connected yet.
pub fn import_file(path: &str, name: Option<&str>) -> Result<String, String> {
    let file = std::path::Path::new(path);
    if !file.is_file() {
        return Err(format!("{path} is not a file"));
    }
    let kind = match file.extension().and_then(|e| e.to_str()) {
        Some("conf") => "wireguard",
        Some("ovpn") => "openvpn",
        _ => return Err("Expected an OpenVPN .ovpn file or a WireGuard .conf file".to_string()),
    };
    let out = run(Command::new("nmcli").args(["connection", "import", "type", kind, "file", path]))
        .map_err(|e| {
            if e.contains("plugin") {
                format!("{e} — install the NetworkManager {kind} plugin (networkmanager-{kind})")
            } else {
                e
            }
        })?;
    let uuid = uuid_from_import(&out).ok_or_else(|| format!("Imported, but couldn't read the result: {out}"))?;

    // A VPN shouldn't come up by itself just because it was added.
    run(Command::new("nmcli").args(["connection", "modify", "uuid", &uuid, "connection.autoconnect", "no"])).ok();
    if kind == "openvpn" {
        // Current OpenVPN refuses a server's older `cipher` unless it is also
        // offered as a data cipher, which the importer doesn't carry over.
        let data = run_lossy(Command::new("nmcli").args(["-g", "vpn.data", "connection", "show", &uuid]));
        if let (Some(cipher), None) = (vpn_data_value(&data, "cipher"), vpn_data_value(&data, "data-ciphers")) {
            run(Command::new("nmcli").args([
                "connection", "modify", "uuid", &uuid, "+vpn.data", &format!("data-ciphers={cipher}"),
            ]))
            .ok();
        }
    }
    if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
        run(Command::new("nmcli").args(["connection", "modify", "uuid", &uuid, "connection.id", name])).ok();
    }
    Ok(uuid)
}

/// Config files sitting in Downloads, offered as one-click imports.
pub fn find_configs() -> Vec<String> {
    let Some(dir) = dirs::download_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("ovpn" | "conf")))
        // A .conf in Downloads is only a WireGuard config if it says so.
        .filter(|p| {
            p.extension().is_some_and(|e| e == "ovpn")
                || std::fs::read_to_string(p).is_ok_and(|t| t.contains("[Interface]"))
        })
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    found.sort();
    found.truncate(30);
    found
}

// ---- VPN Gate ----

#[derive(Serialize, Clone)]
pub struct GateServer {
    pub host: String,
    pub ip: String,
    pub country: String,
    pub country_code: String,
    pub ping_ms: u32,
    pub speed_mbps: f32,
    pub sessions: u32,
    pub uptime_days: f32,
    pub operator: String,
    /// "udp" | "tcp", read from the server's config
    pub protocol: String,
    pub port: u16,
    #[serde(skip)]
    config: String,
}

/// The last list fetched; the configs stay here so the UI only has to name a
/// host when adding one.
static GATE_SERVERS: Mutex<Vec<GateServer>> = Mutex::new(Vec::new());

const GATE_API: &str = "https://www.vpngate.net/api/iphone/";

fn parse_gate_list(csv: &str) -> Vec<GateServer> {
    csv.lines()
        .filter(|l| !l.starts_with('*') && !l.starts_with('#'))
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 15 {
                return None;
            }
            let config = base64::engine::general_purpose::STANDARD
                .decode(f[14].trim())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())?;
            let directive = |name: &str| {
                config
                    .lines()
                    .map(str::trim)
                    .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ').map(str::to_string))
            };
            Some(GateServer {
                host: f[0].to_string(),
                ip: f[1].to_string(),
                ping_ms: f[3].parse().unwrap_or(0),
                speed_mbps: f[4].parse::<f64>().unwrap_or(0.0) as f32 / 1e6,
                country: f[5].to_string(),
                country_code: f[6].to_string(),
                sessions: f[7].parse().unwrap_or(0),
                uptime_days: f[8].parse::<f64>().unwrap_or(0.0) as f32 / 86_400_000.0,
                operator: f[12].replace('_', ","),
                protocol: directive("proto").unwrap_or_else(|| "udp".to_string()),
                port: directive("remote")
                    .and_then(|r| r.split_whitespace().nth(1)?.parse().ok())
                    .unwrap_or(0),
                config,
            })
        })
        .collect()
}

/// Fetched with the system `curl`: the site's old TLS setup is rejected by
/// the HTTP library the update check uses, and falling back to plain HTTP
/// would let anyone on the network swap in their own VPN configs.
pub fn fetch_gate_servers() -> Result<Vec<GateServer>, String> {
    let csv = run(Command::new("curl").args(["-fsS", "--max-time", "45", GATE_API]))
        .map_err(|e| format!("Couldn't reach VPN Gate: {e}"))?;
    let servers = parse_gate_list(&csv);
    if servers.is_empty() {
        return Err("VPN Gate returned no servers — try again in a minute".to_string());
    }
    *GATE_SERVERS.lock().unwrap() = servers.clone();
    Ok(servers)
}

/// Saves a VPN Gate server as a NetworkManager connection, optionally
/// connecting straight away.
pub fn add_gate_server(host: &str, connect: bool) -> ActionResult {
    let Some(server) = GATE_SERVERS.lock().unwrap().iter().find(|s| s.host == host).cloned() else {
        return err("That server isn't in the list any more — reload the list");
    };
    let safe_host: String = server
        .host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let dir = data_dir().join("vpn");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err(format!("Couldn't create {}: {e}", dir.display()));
    }
    let path = dir.join(format!("vpngate-{}-{safe_host}.ovpn", server.country_code));
    if let Err(e) = std::fs::write(&path, &server.config) {
        return err(format!("Couldn't save the config: {e}"));
    }
    let name = format!("VPN Gate {} ({})", server.country_code, server.host);
    // Re-adding the same server replaces the old entry instead of piling up copies.
    if let Some(existing) = list().into_iter().find(|c| c.name == name) {
        run(Command::new("nmcli").args(["connection", "delete", "uuid", &existing.uuid])).ok();
    }
    let uuid = match import_file(&path.to_string_lossy(), Some(&name)) {
        Ok(uuid) => uuid,
        Err(e) => return err(e),
    };
    if !connect {
        return ok(format!("Added “{name}” to your VPNs"));
    }
    let r = set_active(&uuid, true);
    if r.ok {
        ok(format!("Connected through {} ({})", server.country, server.host))
    } else {
        err(format!(
            "Added “{name}”, but connecting failed: {} — volunteer servers come and go, try another",
            r.message
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_values_out_of_vpn_data() {
        let data = "auth = SHA1, cipher = AES-128-CBC, dev = tun, remote = 126.206.11.243\\:1568";
        assert_eq!(vpn_data_value(data, "cipher").as_deref(), Some("AES-128-CBC"));
        assert_eq!(vpn_data_value(data, "remote").as_deref(), Some("126.206.11.243:1568"));
        assert_eq!(vpn_data_value(data, "data-ciphers"), None);
    }

    #[test]
    fn reads_uuid_from_import_output() {
        let out = "Connection 'x (y)' (7ca6ed8f-c504-45f6-b2ee-cdb01db7ac8d) successfully added.";
        assert_eq!(uuid_from_import(out).as_deref(), Some("7ca6ed8f-c504-45f6-b2ee-cdb01db7ac8d"));
        assert_eq!(uuid_from_import("Error: failed"), None);
    }

    #[test]
    fn parses_the_vpn_gate_list() {
        let config = "dev tun\r\nproto tcp\r\nremote 219.100.37.30 443\r\ncipher AES-128-CBC\r\n";
        let b64 = base64::engine::general_purpose::STANDARD.encode(config);
        let csv = format!(
            "*vpn_servers\n#HostName,IP,...\npublic-vpn-88,219.100.37.30,2936602,12,168900653,Japan,JP,94,575626696,1,2,2weeks,Some Operator_ Japan,,{b64}\n*\n"
        );
        let servers = parse_gate_list(&csv);
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!((s.host.as_str(), s.country_code.as_str(), s.ping_ms), ("public-vpn-88", "JP", 12));
        assert_eq!((s.protocol.as_str(), s.port), ("tcp", 443));
        assert!((s.speed_mbps - 168.9).abs() < 0.1);
        assert_eq!(s.operator, "Some Operator, Japan");
    }

    /// Touches the real NetworkManager: imports a downloaded config, checks
    /// the result, and deletes it again. Run with `--ignored`.
    #[test]
    #[ignore]
    fn imports_and_deletes_a_real_config() {
        let Some(path) = find_configs().into_iter().find(|p| p.ends_with(".ovpn")) else {
            return;
        };
        let name = format!("trace-import-test-{}", std::process::id());
        let uuid = import_file(&path, Some(&name)).expect("import");
        let added = list().into_iter().find(|c| c.uuid == uuid).expect("listed");
        assert_eq!(added.name, name);
        assert_eq!(added.kind, "OpenVPN");
        assert!(!added.autoconnect && !added.active);
        assert!(!added.remote.is_empty());
        let data = run_lossy(Command::new("nmcli").args(["-g", "vpn.data", "connection", "show", &uuid]));
        assert!(vpn_data_value(&data, "data-ciphers").is_some(), "{data}");
        assert!(delete(&uuid).ok);
        assert!(list().iter().all(|c| c.uuid != uuid));
    }
}

#[cfg(test)]
mod live {
    /// Hits the real VPN Gate API. Run with `--ignored`.
    #[test]
    #[ignore]
    fn fetches_the_live_server_list() {
        let servers = super::fetch_gate_servers().expect("fetch");
        assert!(servers.len() > 5);
        assert!(servers.iter().all(|s| s.port > 0 && !s.country_code.is_empty()));
    }
}
