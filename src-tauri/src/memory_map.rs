use crate::control_process::ActionResult;
use crate::sys_util::{err, ok};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;

#[derive(Serialize, Clone)]
pub struct MemoryCategory {
    pub key: String,
    pub label: String,
    pub bytes: u64,
    pub note: String,
}

#[derive(Serialize, Clone)]
pub struct SwapDevice {
    pub name: String,
    pub kind: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub priority: i32,
}

#[derive(Serialize, Clone, Default)]
pub struct CompressedSwap {
    /// Size of the data before compression (zram + zswap).
    pub original_bytes: u64,
    /// RAM the compressed copies actually occupy.
    pub ram_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub ram_bytes: u64,
    pub swap_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct AppMemory {
    pub name: String,
    pub process_count: usize,
    pub ram_bytes: u64,
    pub anon_bytes: u64,
    pub file_bytes: u64,
    pub shmem_bytes: u64,
    pub swap_bytes: u64,
    /// True when at least one process could only be read via the coarser
    /// `/proc/<pid>/status` numbers (other users' processes, e.g. root).
    pub approximate: bool,
    pub processes: Vec<ProcessMemory>,
}

#[derive(Serialize, Clone)]
pub struct MemoryBreakdown {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub categories: Vec<MemoryCategory>,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_devices: Vec<SwapDevice>,
    pub compressed: CompressedSwap,
    /// Share of the last 10s / 60s in which some task stalled waiting for memory.
    pub pressure_some_avg10: Option<f32>,
    pub pressure_some_avg60: Option<f32>,
    pub apps: Vec<AppMemory>,
}

/// Parses `Key:   123 kB` style files (`/proc/meminfo`, `smaps_rollup`,
/// `status`) into bytes. Lines without a `kB` unit are kept as raw numbers.
fn parse_kb_file(text: &str) -> HashMap<&str, u64> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let Some(value) = parts.next().and_then(|v| v.parse::<u64>().ok()) else {
            continue;
        };
        let bytes = if parts.next() == Some("kB") { value * 1024 } else { value };
        map.insert(key.trim(), bytes);
    }
    map
}

/// Sums zram's `mm_stat` across devices: (original data size, RAM used).
fn read_zram() -> (u64, u64) {
    let mut original = 0;
    let mut ram = 0;
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("zram") {
            continue;
        }
        let Ok(stat) = fs::read_to_string(entry.path().join("mm_stat")) else {
            continue;
        };
        let fields: Vec<u64> = stat.split_whitespace().filter_map(|f| f.parse().ok()).collect();
        if fields.len() >= 3 {
            original += fields[0];
            ram += fields[2];
        }
    }
    (original, ram)
}

/// System RAM the AMD driver has handed to the GPU (GTT), for kernels that
/// don't report GPU memory in `/proc/meminfo`.
fn read_amd_gtt() -> u64 {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("card") && !name.contains('-')
        })
        .filter_map(|e| fs::read_to_string(e.path().join("device/mem_info_gtt_used")).ok())
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .sum()
}

fn read_swap_devices() -> Vec<SwapDevice> {
    let Ok(text) = fs::read_to_string("/proc/swaps") else {
        return Vec::new();
    };
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 5 {
                return None;
            }
            let kind = if f[0].contains("zram") {
                "compressed RAM"
            } else if f[1] == "file" {
                "swap file"
            } else {
                "disk"
            };
            Some(SwapDevice {
                name: f[0].to_string(),
                kind: kind.to_string(),
                total_bytes: f[2].parse::<u64>().ok()? * 1024,
                used_bytes: f[3].parse::<u64>().ok()? * 1024,
                priority: f[4].parse().unwrap_or(0),
            })
        })
        .collect()
}

fn read_pressure() -> (Option<f32>, Option<f32>) {
    let Ok(text) = fs::read_to_string("/proc/pressure/memory") else {
        return (None, None);
    };
    let Some(line) = text.lines().find(|l| l.starts_with("some")) else {
        return (None, None);
    };
    let field = |name: &str| {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(name))
            .and_then(|v| v.parse::<f32>().ok())
    };
    (field("avg10="), field("avg60="))
}

const INTERPRETERS: [&str; 12] = [
    "node", "python", "python3", "bash", "sh", "fish", "zsh", "java", "bun", "deno", "ruby", "perl",
];

/// Display/grouping name for a process: the executable's file name, or for
/// interpreters the script they're running (`python: uvicorn`), so unrelated
/// scripts don't all collapse into one "python" row.
pub fn app_name(pid: &str) -> Option<String> {
    let base = format!("/proc/{pid}");
    let exe = fs::read_link(format!("{base}/exe")).ok().map(|p| {
        let s = p.to_string_lossy().to_string();
        let s = s.trim_end_matches(" (deleted)");
        s.rsplit('/').next().unwrap_or(s).to_string()
    });
    let Some(mut name) = exe.filter(|n| !n.is_empty()) else {
        // Not our process (or a kernel thread): fall back to the short comm name.
        return fs::read_to_string(format!("{base}/comm"))
            .ok()
            .map(|s| s.trim().to_string());
    };
    // python3.14 -> python
    if name.starts_with("python") {
        name = "python".to_string();
    }
    if INTERPRETERS.contains(&name.as_str()) {
        if let Ok(raw) = fs::read(format!("{base}/cmdline")) {
            let args: Vec<String> = raw
                .split(|b| *b == 0)
                .skip(1)
                .map(|a| String::from_utf8_lossy(a).to_string())
                .collect();
            let script = args
                .iter()
                .position(|a| a == "-m")
                .and_then(|i| args.get(i + 1))
                .or_else(|| args.iter().find(|a| !a.is_empty() && !a.starts_with('-')));
            if let Some(script) = script {
                let short = script.rsplit('/').next().unwrap_or(script);
                let short: String = short.chars().take(32).collect();
                return Some(format!("{name}: {short}"));
            }
        }
    }
    Some(name)
}

struct ProcSample {
    anon: u64,
    file: u64,
    shmem: u64,
    swap: u64,
    approximate: bool,
}

/// Proportional memory for one process. `smaps_rollup` splits shared pages
/// fairly between the processes mapping them (PSS), so summing it across an
/// app's processes doesn't double count; `status` (RSS) is the fallback for
/// processes we aren't allowed to inspect that closely.
fn sample_process(pid: &str) -> Option<ProcSample> {
    if let Ok(text) = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        let m = parse_kb_file(&text);
        if m.contains_key("Pss") {
            let get = |k: &str| m.get(k).copied().unwrap_or(0);
            return Some(ProcSample {
                anon: get("Pss_Anon"),
                file: get("Pss_File"),
                shmem: get("Pss_Shmem"),
                swap: get("SwapPss"),
                approximate: false,
            });
        }
    }
    let text = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let m = parse_kb_file(&text);
    // Kernel threads have no VmRSS line at all.
    m.get("VmRSS")?;
    let get = |k: &str| m.get(k).copied().unwrap_or(0);
    Some(ProcSample {
        anon: get("RssAnon"),
        file: get("RssFile"),
        shmem: get("RssShmem"),
        swap: get("VmSwap"),
        approximate: true,
    })
}

fn read_apps() -> Vec<AppMemory> {
    let mut groups: HashMap<String, AppMemory> = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let pid_str = entry.file_name().to_string_lossy().to_string();
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        let Some(sample) = sample_process(&pid_str) else {
            continue;
        };
        let Some(name) = app_name(&pid_str) else {
            continue;
        };
        let ram = sample.anon + sample.file + sample.shmem;
        let group = groups.entry(name.clone()).or_insert_with(|| AppMemory {
            name: name.clone(),
            process_count: 0,
            ram_bytes: 0,
            anon_bytes: 0,
            file_bytes: 0,
            shmem_bytes: 0,
            swap_bytes: 0,
            approximate: false,
            processes: Vec::new(),
        });
        group.process_count += 1;
        group.ram_bytes += ram;
        group.anon_bytes += sample.anon;
        group.file_bytes += sample.file;
        group.shmem_bytes += sample.shmem;
        group.swap_bytes += sample.swap;
        group.approximate |= sample.approximate;
        group.processes.push(ProcessMemory {
            pid,
            name,
            ram_bytes: ram,
            swap_bytes: sample.swap,
        });
    }
    let mut apps: Vec<AppMemory> = groups.into_values().collect();
    for app in &mut apps {
        app.processes.sort_by(|a, b| b.ram_bytes.cmp(&a.ram_bytes));
        app.processes.truncate(12);
    }
    apps.sort_by(|a, b| (b.ram_bytes + b.swap_bytes).cmp(&(a.ram_bytes + a.swap_bytes)));
    apps
}

/// Accounts for every byte of physical RAM by purpose, plus where swapped-out
/// data went and which apps own it. Reading `smaps_rollup` for every process
/// takes a few hundred ms on a busy desktop, so this is fetched on demand
/// rather than folded into the 2s snapshot.
pub fn breakdown() -> MemoryBreakdown {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let m = parse_kb_file(&text);
    let get = |k: &str| m.get(k).copied().unwrap_or(0);

    let total = get("MemTotal");
    let free = get("MemFree");
    let shmem = get("Shmem");
    let apps_anon = get("AnonPages");
    let cache = (get("Cached") + get("Buffers")).saturating_sub(shmem);
    let kernel = get("Slab")
        + get("KernelStack")
        + get("PageTables")
        + get("SecPageTables")
        + get("Percpu");
    // Newer kernels report driver-held GPU memory directly; older ones only
    // expose it per-card for AMD.
    let graphics = if m.contains_key("GPUActive") {
        get("GPUActive") + get("GPUReclaim")
    } else {
        read_amd_gtt()
    };
    let (zram_original, zram_ram) = read_zram();
    let compressed = CompressedSwap {
        original_bytes: zram_original + get("Zswapped"),
        ram_bytes: zram_ram + get("Zswap"),
    };

    let accounted = apps_anon + cache + shmem + kernel + graphics + compressed.ram_bytes + free;
    let other = total.saturating_sub(accounted);

    let cat = |key: &str, label: &str, bytes: u64, note: &str| MemoryCategory {
        key: key.to_string(),
        label: label.to_string(),
        bytes,
        note: note.to_string(),
    };
    let categories = vec![
        cat("apps", "Apps", apps_anon, "private data of running programs — only freed by closing them or swapping"),
        cat("compressed", "Compressed swap", compressed.ram_bytes, "old app data squeezed and kept in RAM (zram/zswap)"),
        cat("cache", "File cache", cache, "recently used files — handed back instantly when apps need room"),
        cat("kernel", "Kernel", kernel, "the kernel's own bookkeeping (slab, page tables, stacks)"),
        cat("graphics", "Graphics", graphics, "system RAM lent to the GPU driver for windows and rendering"),
        cat("shared", "Shared / tmpfs", shmem, "shared memory and files living in RAM (/tmp, /dev/shm)"),
        cat("other", "Other", other, "swap cache, firmware/driver reservations and anything unattributable"),
        cat("free", "Free", free, "completely unused"),
    ];

    let swap_total = get("SwapTotal");
    let (pressure_some_avg10, pressure_some_avg60) = read_pressure();

    MemoryBreakdown {
        total_bytes: total,
        available_bytes: get("MemAvailable"),
        categories,
        swap_total_bytes: swap_total,
        swap_used_bytes: swap_total.saturating_sub(get("SwapFree")),
        swap_devices: read_swap_devices(),
        compressed,
        pressure_some_avg10,
        pressure_some_avg60,
        apps: read_apps(),
    }
}

// ---- acting on a whole app ----

/// Apps whose death takes the whole session (or machine) with them.
const PROTECTED_APPS: [&str; 9] = [
    "systemd", "Hyprland", "gnome-shell", "kwin_wayland", "sway", "Xorg", "sddm", "gdm", "trace",
];

pub fn pids_of_app(name: &str) -> Vec<u32> {
    let own = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = entries
        .flatten()
        .filter_map(|e| {
            let pid_str = e.file_name().to_string_lossy().to_string();
            let pid: u32 = pid_str.parse().ok()?;
            (pid != own && app_name(&pid_str).as_deref() == Some(name)).then_some(pid)
        })
        .collect();
    pids.sort();
    pids
}

fn parent_pid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

fn nul_separated(path: String) -> Vec<String> {
    fs::read(path)
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .collect()
}

fn signal_all(pids: &[u32], signal: i32) -> usize {
    pids.iter()
        .filter(|pid| unsafe { libc::kill(**pid as i32, signal) } == 0)
        .count()
}

fn restart_app(name: &str, pids: &[u32]) -> ActionResult {
    // The process the rest of the app descends from is the one to relaunch.
    let Some(leader) = pids
        .iter()
        .copied()
        .find(|pid| parent_pid(*pid).is_none_or(|parent| !pids.contains(&parent)))
    else {
        return err(format!("Couldn't work out how {name} was started"));
    };
    let args = nul_separated(format!("/proc/{leader}/cmdline"));
    let env = nul_separated(format!("/proc/{leader}/environ"));
    let cwd = fs::read_link(format!("/proc/{leader}/cwd")).ok();
    if args.is_empty() || env.is_empty() {
        return err(format!(
            "Can't read how {name} was started (it belongs to another user), so it can't be restarted from here"
        ));
    }

    signal_all(pids, libc::SIGTERM);
    let alive = |pid: &u32| std::path::Path::new(&format!("/proc/{pid}")).exists();
    for _ in 0..50 {
        if !pids.iter().any(alive) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let stubborn: Vec<u32> = pids.iter().copied().filter(|p| alive(p)).collect();
    signal_all(&stubborn, libc::SIGKILL);

    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(&args[0]);
    cmd.args(&args[1..])
        .env_clear()
        .envs(env.iter().filter_map(|kv| kv.split_once('=')))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Own process group, so the app outlives Trace.
        .process_group(0);
    if let Some(cwd) = cwd.filter(|c| c.exists()) {
        cmd.current_dir(cwd);
    }
    match cmd.spawn() {
        Ok(_) => ok(format!("Restarted {name} ({} processes replaced)", pids.len())),
        Err(e) => err(format!("Stopped {name} but couldn't start it again: {e}")),
    }
}

/// `action`: "kill" | "force_kill" | "freeze" | "resume" | "restart".
pub fn app_action(name: &str, action: &str) -> ActionResult {
    if PROTECTED_APPS.contains(&name) && action != "resume" {
        return err(format!(
            "{name} keeps your session running — Trace won't {} it",
            action.replace('_', " ")
        ));
    }
    let pids = pids_of_app(name);
    if pids.is_empty() {
        return err(format!("{name} is no longer running"));
    }
    let (signal, verb) = match action {
        "kill" => (libc::SIGTERM, "Asked"),
        "force_kill" => (libc::SIGKILL, "Force-killed"),
        "freeze" => (libc::SIGSTOP, "Froze"),
        "resume" => (libc::SIGCONT, "Resumed"),
        "restart" => return restart_app(name, &pids),
        _ => return err(format!("Unknown action {action}")),
    };
    let done = signal_all(&pids, signal);
    if done == 0 {
        return err(format!(
            "Couldn't signal any of {name}'s {} processes (they belong to another user)",
            pids.len()
        ));
    }
    ok(format!(
        "{verb} {done} of {} {name} processes{}",
        pids.len(),
        if action == "kill" { " to quit" } else { "" }
    ))
}

/// Caps an app's memory through its systemd scope. Refuses when the app
/// isn't in a scope of its own — the cap would then land on the whole
/// session and could take the desktop down with it.
pub fn limit_app_memory(name: &str, mb: u64) -> ActionResult {
    let Some(pid) = pids_of_app(name).first().copied() else {
        return err(format!("{name} is no longer running"));
    };
    let cgroup = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
    let leaf = cgroup.trim().rsplit('/').next().unwrap_or("").to_string();
    if !(leaf.starts_with("app-") || cgroup.contains("/app.slice/")) || leaf.starts_with("session-") {
        return err(format!(
            "{name} shares its control group ({leaf}) with the rest of your session, so a cap would hit everything in it"
        ));
    }
    let r = crate::control_process::set_memory_limit(pid, mb);
    ActionResult {
        ok: r.ok,
        message: if r.ok {
            format!("Capped {name} at {mb} MB — it will be slowed or killed if it goes over")
        } else {
            r.message
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the app-level actions against a uniquely named copy of
    /// `sleep`, so nothing else on the machine can match the app name.
    #[test]
    fn app_actions_freeze_restart_and_kill_a_throwaway_app() {
        let name = format!("trace-test-sleeper-{}", std::process::id());
        let exe = std::env::temp_dir().join(&name);
        // Copied by `cp` rather than `fs::copy`: a write handle held in this
        // process can leak into a parallel test's fork and make exec fail
        // with "text file busy".
        assert!(std::process::Command::new("cp").arg("/usr/bin/sleep").arg(&exe).status().unwrap().success());
        let mut child = std::process::Command::new(&exe).arg("300").spawn().unwrap();
        let original = child.id();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(pids_of_app(&name), vec![original]);

        let state = |pid: u32| {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            stat.rsplit_once(')').unwrap().1.trim_start().chars().next().unwrap()
        };
        assert!(app_action(&name, "freeze").ok);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(state(original), 'T');
        assert!(app_action(&name, "resume").ok);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_ne!(state(original), 'T');

        let r = app_action(&name, "restart");
        assert!(r.ok, "{}", r.message);
        child.wait().ok();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let relaunched = pids_of_app(&name);
        assert_eq!(relaunched.len(), 1, "exactly one relaunched process");
        assert_ne!(relaunched[0], original);

        assert!(app_action(&name, "force_kill").ok);
        std::thread::sleep(std::time::Duration::from_millis(300));
        // The relaunched process is our child, so it lingers as a zombie
        // until reaped; a zombie has no executable and no longer matches.
        assert!(pids_of_app(&name).is_empty());
        assert!(!app_action(&name, "kill").ok, "nothing left to act on");
        assert!(!app_action("systemd", "kill").ok, "protected apps are refused");
        fs::remove_file(exe).ok();
    }

    #[test]
    fn categories_account_for_all_ram() {
        let b = breakdown();
        let sum: u64 = b.categories.iter().map(|c| c.bytes).sum();
        assert_eq!(sum, b.total_bytes);
        assert!(!b.apps.is_empty());
        if let Ok(path) = std::env::var("TRACE_DUMP_MEMORY") {
            fs::write(path, serde_json::to_string(&b).unwrap()).unwrap();
        }
    }
}
