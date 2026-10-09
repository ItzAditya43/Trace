//! VPN connections, managed through NetworkManager so they show up in (and
//! behave like) every other VPN on the system. Includes a browser for VPN
//! Gate's public server list that turns a server into a saved connection.

use crate::control_process::ActionResult;
use crate::sys_util::{data_dir, err, ok, privileged_action, run, run_lossy};
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

/// Rewrites `remote <hostname> <port>` to use the server's IP address. A VPN
/// that needs a DNS lookup to connect can't reconnect once the kill switch
/// is blocking everything outside the tunnel.
fn pin_remote_ip(config: &str, ip: &str) -> String {
    let is_ip = |s: &str| s.parse::<std::net::IpAddr>().is_ok();
    if !is_ip(ip) {
        return config.to_string();
    }
    config
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                ["remote", host, rest @ ..] if !is_ip(host) => {
                    format!("remote {ip} {}", rest.join(" ")).trim_end().to_string()
                }
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
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
    if let Err(e) = std::fs::write(&path, pin_remote_ip(&server.config, &server.ip)) {
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

/// Tries the fastest VPN Gate servers in a country one after another until
/// one connects. `country_code` "any" searches every country.
pub fn connect_best_gate(country_code: &str) -> ActionResult {
    if GATE_SERVERS.lock().unwrap().is_empty() {
        if let Err(e) = fetch_gate_servers() {
            return err(e);
        }
    }
    let mut candidates: Vec<GateServer> = GATE_SERVERS
        .lock()
        .unwrap()
        .iter()
        .filter(|s| country_code == "any" || s.country_code == country_code)
        .cloned()
        .collect();
    if candidates.is_empty() {
        return err("No servers listed for that country right now");
    }
    candidates.sort_by(|a, b| b.speed_mbps.total_cmp(&a.speed_mbps));
    let mut tried = Vec::new();
    for server in candidates.iter().take(5) {
        let result = add_gate_server(&server.host, true);
        if result.ok {
            return ok(if tried.is_empty() {
                result.message
            } else {
                format!("{} (after {} server(s) didn't answer)", result.message, tried.len())
            });
        }
        // Don't leave the dead server behind in the saved list.
        let name = format!("VPN Gate {} ({})", server.country_code, server.host);
        if let Some(saved) = list().into_iter().find(|c| c.name == name) {
            delete(&saved.uuid);
        }
        tried.push(server.host.clone());
    }
    err(format!(
        "None of the {} fastest servers answered ({}). Volunteer servers come and go — try again or pick another country.",
        tried.len(),
        tried.join(", ")
    ))
}

// ---- where the internet sees you ----

#[derive(Serialize)]
pub struct PublicAddress {
    pub ip: String,
    pub country_code: String,
}

/// Asks Cloudflare's trace endpoint which address this machine's traffic
/// appears to come from. Only called when you ask or when connecting a VPN.
pub fn public_address() -> Result<PublicAddress, String> {
    let out = run(Command::new("curl").args(["-fsS", "--max-time", "8", "https://1.1.1.1/cdn-cgi/trace"]))
        .map_err(|e| format!("Couldn't check: {e}"))?;
    let field = |key: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
            .to_string()
    };
    let ip = field("ip");
    if ip.is_empty() {
        return Err("Couldn't check: unexpected answer".to_string());
    }
    Ok(PublicAddress { ip, country_code: field("loc") })
}

// ---- kill switch ----

const KILL_MARKER: &str = "/run/trace-killswitch";

/// The firewall rules behind the kill switch. Outbound traffic is dropped
/// unless it goes into a VPN tunnel, to a VPN server (so the tunnel itself
/// can be built), or stays on this machine and — optionally — the local
/// network. DNS outside the tunnel is always dropped, so lookups can't leak.
fn kill_switch_rules(server_ips: &[String], allow_lan: bool) -> String {
    let servers: Vec<&String> = server_ips
        .iter()
        .filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
        .collect();
    let mut rules = String::from(
        "table inet trace_killswitch {\n  chain output {\n    type filter hook output priority filter; policy drop;\n    oifname \"lo\" accept\n    oifname \"tun*\" accept\n    oifname \"tap*\" accept\n    oifname \"wg*\" accept\n    oifname \"ppp*\" accept\n",
    );
    if !servers.is_empty() {
        let list: Vec<&str> = servers.iter().map(|s| s.as_str()).collect();
        rules.push_str(&format!("    ip daddr {{ {} }} accept\n", list.join(", ")));
    }
    rules.push_str(
        "    udp dport 67 accept\n    oifname \"docker0\" accept\n    oifname \"br-*\" accept\n    oifname \"veth*\" accept\n    oifname \"virbr*\" accept\n    meta l4proto { tcp, udp } th dport 53 drop\n",
    );
    if allow_lan {
        rules.push_str(
            "    ip daddr { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16 } accept\n    ip6 daddr fe80::/10 accept\n",
        );
    }
    rules.push_str(
        "    icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert, nd-router-solicit } accept\n  }\n  chain forward {\n    type filter hook forward priority filter; policy drop;\n    oifname \"tun*\" accept\n    oifname \"wg*\" accept\n    oifname \"docker0\" accept\n    oifname \"br-*\" accept\n    oifname \"veth*\" accept\n    oifname \"virbr*\" accept\n  }\n}\n",
    );
    rules
}

#[derive(Serialize)]
pub struct KillSwitchStatus {
    pub active: bool,
    /// Saved VPNs whose server is a hostname: they can't reconnect while
    /// the kill switch is on, because the lookup is blocked.
    pub hostname_vpns: Vec<String>,
}

fn remote_host(remote: &str) -> &str {
    remote.rsplit_once(':').map(|(host, _)| host).unwrap_or(remote)
}

pub fn kill_switch_status() -> KillSwitchStatus {
    KillSwitchStatus {
        active: std::path::Path::new(KILL_MARKER).exists(),
        hostname_vpns: list()
            .into_iter()
            .filter(|c| !c.remote.is_empty() && remote_host(&c.remote).parse::<std::net::IpAddr>().is_err())
            .map(|c| c.name)
            .collect(),
    }
}

pub fn set_kill_switch(enabled: bool, allow_lan: bool) -> ActionResult {
    if !enabled {
        return privileged_action(
            &format!("nft delete table inet trace_killswitch 2>/dev/null; rm -f {KILL_MARKER}"),
            "Kill switch off — traffic flows normally again",
        );
    }
    let ips: Vec<String> = list().iter().map(|c| remote_host(&c.remote).to_string()).collect();
    let rules = kill_switch_rules(&ips, allow_lan);
    privileged_action(
        &format!(
            "nft delete table inet trace_killswitch 2>/dev/null; nft -f - <<'TRACE_EOF'\n{rules}TRACE_EOF\n[ $? = 0 ] && touch {KILL_MARKER}"
        ),
        "Kill switch on — nothing leaves this machine except through a VPN",
    )
}

// ---- split tunnel: only chosen apps use the VPN ----

const SPLIT_MARKER: &str = "/run/trace-split";
const SPLIT_SLICE: &str = "tracevpn.slice";
const SPLIT_MARK: &str = "0x7472";
const SPLIT_TABLE: &str = "7472";

#[derive(Serialize)]
pub struct SplitStatus {
    pub active: bool,
    pub interface: String,
    pub vpn_name: String,
    /// Programs currently running inside the VPN-only group.
    pub apps: Vec<String>,
}

fn split_slice_cgroup() -> Option<String> {
    let path = run(Command::new("systemctl").args(["--user", "show", "-p", "ControlGroup", "--value", SPLIT_SLICE])).ok()?;
    let path = path.trim().trim_start_matches('/').to_string();
    (!path.is_empty()).then_some(path)
}

fn tunnel_interfaces() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| ["tun", "tap", "wg", "ppp"].iter().any(|p| n.starts_with(p)))
        .collect();
    names.sort();
    names
}

pub fn split_status() -> SplitStatus {
    let marker = std::fs::read_to_string(SPLIT_MARKER).unwrap_or_default();
    let mut parts = marker.trim().splitn(2, ' ');
    let interface = parts.next().unwrap_or("").to_string();
    let uuid = parts.next().unwrap_or("");
    let apps = run_lossy(Command::new("systemctl").args([
        "--user", "list-units", "--type=scope", "--no-legend", "--plain", "--state=running", "tracevpn-*",
    ]))
    .lines()
    .filter_map(|l| {
        let unit = l.split_whitespace().next()?;
        // tracevpn-<name>-<number>.scope
        let inner = unit.strip_prefix("tracevpn-")?.strip_suffix(".scope")?;
        Some(inner.rsplit_once('-').map(|(name, _)| name).unwrap_or(inner).to_string())
    })
    .collect();
    SplitStatus {
        active: !marker.is_empty(),
        vpn_name: list().into_iter().find(|c| c.uuid == uuid).map(|c| c.name).unwrap_or_default(),
        interface,
        apps,
    }
}

fn split_rules(cgroup: &str, interface: &str) -> String {
    let level = cgroup.split('/').filter(|p| !p.is_empty()).count();
    format!(
        "table inet trace_split {{\n  chain mark_out {{\n    type route hook output priority mangle; policy accept;\n    socket cgroupv2 level {level} \"{cgroup}\" meta nfproto ipv4 ip daddr != {{ 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 127.0.0.0/8 }} meta mark set {SPLIT_MARK}\n  }}\n  chain block_v6 {{\n    type filter hook output priority filter; policy accept;\n    socket cgroupv2 level {level} \"{cgroup}\" meta nfproto ipv6 ip6 daddr != {{ ::1, fe80::/10, fc00::/7 }} reject\n  }}\n  chain nat_out {{\n    type nat hook postrouting priority srcnat; policy accept;\n    meta mark {SPLIT_MARK} oifname \"{interface}\" masquerade\n  }}\n}}\n"
    )
}

fn split_cleanup_script() -> String {
    format!(
        "nft delete table inet trace_split 2>/dev/null; ip rule del fwmark {SPLIT_MARK} table {SPLIT_TABLE} 2>/dev/null; ip route flush table {SPLIT_TABLE} 2>/dev/null; rm -f {SPLIT_MARKER}"
    )
}

/// Connects `uuid` without making it the default route, then routes only
/// the programs in the VPN-only group through it.
pub fn split_start(uuid: &str) -> ActionResult {
    if !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') || uuid.len() != 36 {
        return err("Not a VPN id");
    }
    let Some(vpn) = list().into_iter().find(|c| c.uuid == uuid) else {
        return err("That VPN is no longer saved");
    };
    for setting in ["ipv4.never-default", "ipv6.never-default"] {
        if let Err(e) = run(Command::new("nmcli").args(["connection", "modify", "uuid", uuid, setting, "yes"])) {
            return err(format!("Couldn't prepare the VPN: {e}"));
        }
    }
    // Reconnect so the "not the default route" setting takes hold.
    let before = tunnel_interfaces();
    if vpn.active {
        set_active(uuid, false);
    }
    let connected = set_active(uuid, true);
    if !connected.ok {
        return err(format!("The VPN didn't connect: {}", connected.message));
    }
    let after = tunnel_interfaces();
    let Some(interface) = after.iter().find(|i| !before.contains(i)).or(after.first()).cloned() else {
        return err("The VPN connected but no tunnel interface appeared");
    };
    if let Err(e) = run(Command::new("systemctl").args(["--user", "start", SPLIT_SLICE])) {
        return err(format!("Couldn't create the app group: {e}"));
    }
    let Some(cgroup) = split_slice_cgroup() else {
        return err("Couldn't find the app group");
    };
    let rules = split_rules(&cgroup, &interface);
    let script = format!(
        "{cleanup}\nnft -f - <<'TRACE_EOF'\n{rules}TRACE_EOF\n[ $? = 0 ] || exit 1\nip rule add fwmark {SPLIT_MARK} table {SPLIT_TABLE} && ip route replace default dev {interface} table {SPLIT_TABLE} && sysctl -q -w net.ipv4.conf.all.src_valid_mark=1 net.ipv4.conf.{interface}.rp_filter=2 && echo '{interface} {uuid}' > {SPLIT_MARKER}",
        cleanup = split_cleanup_script()
    );
    match crate::sys_util::run_privileged(&script) {
        Ok(_) => ok(format!(
            "“{}” is up on {interface}. Programs you launch through it below use the VPN; everything else doesn't.",
            vpn.name
        )),
        Err(e) => {
            // Don't leave a half-configured VPN behind.
            set_active(uuid, false);
            for setting in ["ipv4.never-default", "ipv6.never-default"] {
                run(Command::new("nmcli").args(["connection", "modify", "uuid", uuid, setting, "no"])).ok();
            }
            err(format!("Couldn't set up the routing: {e}"))
        }
    }
}

pub fn split_stop() -> ActionResult {
    let marker = std::fs::read_to_string(SPLIT_MARKER).unwrap_or_default();
    let uuid = marker.trim().split(' ').nth(1).unwrap_or("").to_string();
    let result = privileged_action(&split_cleanup_script(), "Split tunnel off");
    if result.ok && uuid.len() == 36 {
        set_active(&uuid, false);
        for setting in ["ipv4.never-default", "ipv6.never-default"] {
            run(Command::new("nmcli").args(["connection", "modify", "uuid", &uuid, setting, "no"])).ok();
        }
    }
    result
}

/// Starts a command inside the VPN-only group. The command line is yours
/// and runs as you, exactly as if typed in a terminal.
pub fn launch_in_vpn(command: &str) -> ActionResult {
    let command = command.trim();
    if command.is_empty() {
        return err("Type the command to run, e.g. firefox");
    }
    let name: String = command
        .split_whitespace()
        .next()
        .and_then(|first| first.rsplit('/').next())
        .unwrap_or("app")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    use std::os::unix::process::CommandExt;
    let spawned = Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            &format!("--slice={SPLIT_SLICE}"),
            &format!("--unit=tracevpn-{}-{}", if name.is_empty() { "app" } else { &name }, std::process::id() as u64 * 100_000 + (now_millis() % 100_000)),
            "--",
            "sh",
            "-c",
            command,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    match spawned {
        Ok(_) => ok(format!("Started {command} — its traffic goes through the VPN")),
        Err(e) => err(format!("Couldn't start it: {e}")),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    fn pins_hostname_remotes_to_the_server_ip() {
        let config = "dev tun\nremote vpn123.opengw.net 1776\ncipher AES-128-CBC\n";
        assert_eq!(pin_remote_ip(config, "1.2.3.4"), "dev tun\nremote 1.2.3.4 1776\ncipher AES-128-CBC\n");
        let already = "remote 9.9.9.9 443\n";
        assert_eq!(pin_remote_ip(already, "1.2.3.4"), already);
        assert_eq!(pin_remote_ip(config, "not-an-ip"), config);
    }

    #[test]
    fn kill_switch_rules_only_list_real_addresses() {
        let rules = kill_switch_rules(&["1.2.3.4".to_string(), "evil; flush ruleset".to_string(), "host.example".to_string()], true);
        assert!(rules.contains("ip daddr { 1.2.3.4 } accept"));
        assert!(!rules.contains("evil") && !rules.contains("host.example"));
        assert!(rules.contains("192.168.0.0/16"));
        assert!(!kill_switch_rules(&[], false).contains("192.168.0.0/16"));
    }

    /// Loads the kill-switch rules for real, inside a throwaway user+network
    /// namespace, so a syntax error can never reach the host firewall.
    #[test]
    fn kill_switch_rules_load_in_a_sandbox() {
        let rules = kill_switch_rules(&["1.2.3.4".to_string(), "5.6.7.8".to_string()], true);
        let path = std::env::temp_dir().join(format!("trace-kill-{}.nft", std::process::id()));
        std::fs::write(&path, rules).unwrap();
        let out = Command::new("unshare")
            .args(["-Urn", "sh", "-c"])
            .arg(format!("nft -f {0} && nft list table inet trace_killswitch >/dev/null && nft delete table inet trace_killswitch", path.display()))
            .output();
        std::fs::remove_file(&path).ok();
        let Ok(out) = out else {
            return; // no unshare on this machine
        };
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Unprivileged namespaces may be disabled; that isn't a rules problem.
        if stderr.contains("Operation not permitted") || stderr.contains("unshare failed") {
            return;
        }
        assert!(out.status.success(), "{stderr}");
    }

    #[test]
    fn split_rules_use_the_cgroup_depth() {
        let rules = split_rules("user.slice/user-1000.slice/user@1000.service/tracevpn.slice", "tun0");
        assert!(rules.contains("socket cgroupv2 level 4 \"user.slice/user-1000.slice/user@1000.service/tracevpn.slice\""));
        assert!(rules.contains("oifname \"tun0\" masquerade"));
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
