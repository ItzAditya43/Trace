use crate::memory_map::app_name;
use crate::sys_util::run_lossy;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::time::Instant;

#[derive(Default, Clone, Copy)]
struct Counters {
    read_bytes: u64,
    write_bytes: u64,
    wakeups: u64,
    net_sent: u64,
    net_received: u64,
}

/// Previous cumulative counters, so each call can report a rate.
#[derive(Default)]
pub struct ActivityState {
    at: Option<Instant>,
    per_pid: HashMap<u32, Counters>,
    drives: HashMap<String, (u64, u64)>,
}

#[derive(Serialize, Clone, Default)]
pub struct AppActivity {
    pub name: String,
    pub process_count: usize,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
    pub wakeups_per_sec: f32,
}

#[derive(Serialize, Clone)]
pub struct DriveActivity {
    pub device: String,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
}

#[derive(Serialize, Clone)]
pub struct Activity {
    /// False on the very first sample, when there is nothing to diff against.
    pub ready: bool,
    pub apps: Vec<AppActivity>,
    pub drives: Vec<DriveActivity>,
}

fn field_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let start = text.find(key)? + key.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Cumulative TCP bytes per PID from `ss`, which prints each socket as a
/// header line (with its owning processes) followed by an indented stats line.
/// Only sockets of processes we own carry a PID, and bytes of sockets that
/// have since closed drop out — so this is a good "who is busy right now"
/// signal, not an exact meter.
fn tcp_bytes_by_pid() -> HashMap<u32, (u64, u64)> {
    let out = run_lossy(Command::new("ss").args(["-tinpH"]));
    let mut totals: HashMap<u32, (u64, u64)> = HashMap::new();
    let mut owners: Vec<u32> = Vec::new();
    for line in out.lines() {
        if !line.starts_with(char::is_whitespace) {
            owners = line
                .match_indices("pid=")
                .filter_map(|(i, _)| field_after(&line[i..], "pid=")?.parse().ok())
                .collect();
            owners.dedup();
            continue;
        }
        let sent: u64 = field_after(line, "bytes_sent:").and_then(|v| v.parse().ok()).unwrap_or(0);
        let received: u64 = field_after(line, "bytes_received:")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // A socket shared by several processes (browser helpers) is charged
        // to the first so it isn't counted twice.
        if let Some(pid) = owners.first() {
            let entry = totals.entry(*pid).or_default();
            entry.0 += sent;
            entry.1 += received;
        }
    }
    totals
}

fn read_counters(pid: u32, net: &HashMap<u32, (u64, u64)>) -> Counters {
    let mut c = Counters::default();
    if let Ok(io) = fs::read_to_string(format!("/proc/{pid}/io")) {
        for line in io.lines() {
            if let Some(v) = line.strip_prefix("read_bytes: ") {
                c.read_bytes = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("write_bytes: ") {
                c.write_bytes = v.parse().unwrap_or(0);
            }
        }
    }
    if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
        c.wakeups = status
            .lines()
            .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
    }
    if let Some((sent, received)) = net.get(&pid) {
        c.net_sent = *sent;
        c.net_received = *received;
    }
    c
}

fn drive_sectors() -> HashMap<String, (u64, u64)> {
    crate::health::list_drives()
        .into_iter()
        .filter_map(|d| {
            let stat = fs::read_to_string(format!("/sys/block/{}/stat", d.device)).ok()?;
            let f: Vec<u64> = stat.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some((d.device, (*f.get(2)? * 512, *f.get(6)? * 512)))
        })
        .collect()
}

pub fn sample(state: &mut ActivityState) -> Activity {
    let now = Instant::now();
    let dt = state
        .at
        .map(|prev| now.duration_since(prev).as_secs_f64().max(0.2));

    let net = tcp_bytes_by_pid();
    let mut current: HashMap<u32, Counters> = HashMap::new();
    let mut apps: HashMap<String, AppActivity> = HashMap::new();

    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let pid_str = entry.file_name().to_string_lossy().to_string();
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };
            let counters = read_counters(pid, &net);
            current.insert(pid, counters);
            let (Some(dt), Some(prev)) = (dt, state.per_pid.get(&pid)) else {
                continue;
            };
            let rate = |now: u64, before: u64| (now.saturating_sub(before) as f64 / dt) as u64;
            let read = rate(counters.read_bytes, prev.read_bytes);
            let write = rate(counters.write_bytes, prev.write_bytes);
            let rx = rate(counters.net_received, prev.net_received);
            let tx = rate(counters.net_sent, prev.net_sent);
            let wakeups = counters.wakeups.saturating_sub(prev.wakeups) as f64 / dt;
            if read + write + rx + tx == 0 && wakeups < 1.0 {
                continue;
            }
            // Kernel threads have no command line; they aren't apps.
            if fs::read(format!("/proc/{pid}/cmdline")).map_or(true, |c| c.is_empty()) {
                continue;
            }
            let Some(name) = app_name(&pid_str) else {
                continue;
            };
            let app = apps.entry(name.clone()).or_insert_with(|| AppActivity {
                name,
                ..Default::default()
            });
            app.process_count += 1;
            app.read_bytes_per_sec += read;
            app.write_bytes_per_sec += write;
            app.net_rx_bytes_per_sec += rx;
            app.net_tx_bytes_per_sec += tx;
            app.wakeups_per_sec += wakeups as f32;
        }
    }

    let sectors = drive_sectors();
    let mut drives: Vec<DriveActivity> = sectors
        .iter()
        .map(|(device, (read, written))| {
            let (prev_read, prev_written) = state.drives.get(device).copied().unwrap_or((*read, *written));
            let per_sec = |now: u64, before: u64| {
                dt.map(|dt| (now.saturating_sub(before) as f64 / dt) as u64).unwrap_or(0)
            };
            DriveActivity {
                device: device.clone(),
                read_bytes_per_sec: per_sec(*read, prev_read),
                write_bytes_per_sec: per_sec(*written, prev_written),
            }
        })
        .collect();
    drives.sort_by(|a, b| a.device.cmp(&b.device));

    let ready = state.at.is_some();
    state.at = Some(now);
    state.per_pid = current;
    state.drives = sectors;

    Activity {
        ready,
        apps: apps.into_values().collect(),
        drives,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_numeric_fields() {
        let line = "\t cubic rto:219 bytes_sent:16682 bytes_acked:16683 bytes_received:1148376 segs_out:582";
        assert_eq!(field_after(line, "bytes_sent:"), Some("16682"));
        assert_eq!(field_after(line, "bytes_received:"), Some("1148376"));
        assert_eq!(field_after(r#"users:(("brave",pid=2506,fd=183))"#, "pid="), Some("2506"));
    }
}
