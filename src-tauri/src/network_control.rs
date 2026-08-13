use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

fn ok(msg: impl Into<String>) -> ActionResult {
    ActionResult { ok: true, message: msg.into() }
}

fn err(msg: impl Into<String>) -> ActionResult {
    ActionResult { ok: false, message: msg.into() }
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---- per-process connection list ----

#[derive(Serialize, Clone)]
pub struct ConnectionInfo {
    pub pid: u32,
    pub process_name: String,
    pub local_addr: String,
    pub remote_addr: String,
    pub state: String,
}

fn hex_to_ipv4(hex: &str) -> String {
    let Ok(bytes) = u32::from_str_radix(hex, 16) else {
        return hex.to_string();
    };
    let b = bytes.to_le_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn decode_addr(hex_addr: &str) -> Option<String> {
    let (ip_hex, port_hex) = hex_addr.split_once(':')?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    let ip = if ip_hex.len() == 8 {
        hex_to_ipv4(ip_hex)
    } else {
        "[ipv6]".to_string()
    };
    Some(format!("{ip}:{port}"))
}

fn tcp_state_name(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "UNKNOWN",
    }
}

/// Maps socket inode -> (local, remote, state) from /proc/net/tcp[6].
fn read_socket_table() -> std::collections::HashMap<String, (String, String, String)> {
    let mut map = std::collections::HashMap::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = fs::read_to_string(path) else { continue };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            let (Some(local), Some(remote)) = (decode_addr(fields[1]), decode_addr(fields[2]))
            else {
                continue;
            };
            let state = tcp_state_name(fields[3]).to_string();
            map.insert(fields[9].to_string(), (local, remote, state));
        }
    }
    map
}

pub fn list_connections(sys: &sysinfo::System, only_pid: Option<u32>) -> Vec<ConnectionInfo> {
    let socket_table = read_socket_table();
    let mut connections = Vec::new();

    let Ok(proc_entries) = fs::read_dir("/proc") else {
        return connections;
    };
    for entry in proc_entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(pid_num) = name.parse::<u32>() else { continue };
        if let Some(want) = only_pid {
            if pid_num != want {
                continue;
            }
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else { continue };
        let process_name = sys
            .process(sysinfo::Pid::from_u32(pid_num))
            .map(|p| p.name().to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else { continue };
            let link_str = link.to_string_lossy();
            let Some(inode) = link_str.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']'))
            else {
                continue;
            };
            if let Some((local, remote, state)) = socket_table.get(inode) {
                connections.push(ConnectionInfo {
                    pid: pid_num,
                    process_name: process_name.clone(),
                    local_addr: local.clone(),
                    remote_addr: remote.clone(),
                    state: state.clone(),
                });
            }
        }
    }
    connections
}

// ---- interface bandwidth limiting (tc tbf) ----
//
// This limits a whole network interface, not a single process. True
// per-process shaping needs cgroup-classified traffic (net_cls is
// deprecated under cgroup v2; the real replacement is an eBPF classifier),
// which is a much bigger, riskier undertaking — see nft-based process
// blocking below for a coarser per-process control that IS safe to ship.

pub fn limit_interface(iface: &str, rate_kbit: u32) -> ActionResult {
    // Clear any existing shaping first so limits don't stack.
    run(Command::new("tc").args(["qdisc", "del", "dev", iface, "root"])).ok();
    match run(Command::new("tc").args([
        "qdisc", "add", "dev", iface, "root", "tbf",
        "rate", &format!("{rate_kbit}kbit"),
        "burst", "32kbit",
        "latency", "400ms",
    ])) {
        Ok(_) => ok(format!("Limited {iface} to {rate_kbit} kbit/s (tc tbf)")),
        Err(e) => err(format!(
            "Failed to shape {iface}: {e} (tc qdisc changes need CAP_NET_ADMIN — usually root)"
        )),
    }
}

pub fn clear_interface_limit(iface: &str) -> ActionResult {
    match run(Command::new("tc").args(["qdisc", "del", "dev", iface, "root"])) {
        Ok(_) => ok(format!("Removed bandwidth limit on {iface}")),
        Err(e) => err(format!("Failed to clear shaping on {iface}: {e}")),
    }
}

pub fn list_interfaces() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "lo")
        .collect()
}

// ---- per-process network block (nftables + cgroup v2 path) ----

fn process_cgroup_path(pid: u32) -> Result<String, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|e| format!("Cannot read /proc/{pid}/cgroup: {e}"))?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("0::") {
            return Ok(rest.to_string());
        }
    }
    Err("Process is not on the unified (v2) cgroup hierarchy".to_string())
}

fn nft_ensure_table_and_chain() -> Result<(), String> {
    run(Command::new("nft").args(["add", "table", "inet", "trace_block"]))?;
    run(Command::new("nft").args([
        "add", "chain", "inet", "trace_block", "output",
        "{", "type", "filter", "hook", "output", "priority", "0", ";", "}",
    ]))?;
    Ok(())
}

/// Blocks outbound network traffic from every process in `pid`'s cgroup by
/// matching the cgroup's path. This is coarse — it affects the whole
/// cgroup, not just the one PID, which for most desktop apps (one cgroup
/// per app under systemd --user) is exactly the process tree you'd expect
/// to block. The exact `nft` command run is included in the result so you
/// can verify it with `nft list ruleset` yourself.
pub fn block_process_network(pid: u32) -> ActionResult {
    let path = match process_cgroup_path(pid) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let level = path.split('/').filter(|s| !s.is_empty()).count();
    if let Err(e) = nft_ensure_table_and_chain() {
        return err(format!("Failed to prepare nft table: {e} (needs root/CAP_NET_ADMIN)"));
    }
    let rule = format!("socket cgroupv2 level {level} \"{path}\" drop");
    match run(Command::new("nft").args([
        "add", "rule", "inet", "trace_block", "output",
        "socket", "cgroupv2", "level", &level.to_string(), &path, "drop",
    ])) {
        Ok(_) => ok(format!(
            "Blocked outbound traffic for cgroup {path} (nft rule: {rule})"
        )),
        Err(e) => err(format!("Failed to add nft rule: {e} (needs root/CAP_NET_ADMIN)")),
    }
}

pub fn unblock_all_network() -> ActionResult {
    match run(Command::new("nft").args(["delete", "table", "inet", "trace_block"])) {
        Ok(_) => ok("Removed all Trace network blocks"),
        Err(e) => err(format!("Failed to clear nft table: {e}")),
    }
}
