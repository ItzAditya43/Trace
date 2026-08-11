use serde::Serialize;
use std::fs;

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

fn send_signal(pid: u32, sig: i32) -> Result<(), String> {
    let ret = unsafe { libc::kill(pid as i32, sig) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

pub fn freeze(pid: u32) -> ActionResult {
    match send_signal(pid, libc::SIGSTOP) {
        Ok(()) => ok(format!("Froze PID {pid} (SIGSTOP)")),
        Err(e) => err(format!("Failed to freeze PID {pid}: {e}")),
    }
}

pub fn resume(pid: u32) -> ActionResult {
    match send_signal(pid, libc::SIGCONT) {
        Ok(()) => ok(format!("Resumed PID {pid} (SIGCONT)")),
        Err(e) => err(format!("Failed to resume PID {pid}: {e}")),
    }
}

pub fn kill(pid: u32, force: bool) -> ActionResult {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    let label = if force { "SIGKILL" } else { "SIGTERM" };
    match send_signal(pid, sig) {
        Ok(()) => ok(format!("Sent {label} to PID {pid}")),
        Err(e) => err(format!("Failed to signal PID {pid}: {e}")),
    }
}

pub fn set_priority(pid: u32, nice: i32) -> ActionResult {
    let clamped = nice.clamp(-20, 19);
    let ret = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, clamped) };
    if ret == 0 {
        ok(format!("Set PID {pid} priority (nice) to {clamped}"))
    } else {
        err(format!(
            "Failed to set priority for PID {pid}: {} (raising priority usually needs root)",
            std::io::Error::last_os_error()
        ))
    }
}

pub fn set_affinity(pid: u32, cores: Vec<usize>) -> ActionResult {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        for core in &cores {
            libc::CPU_SET(*core, &mut set);
        }
        let ret = libc::sched_setaffinity(
            pid as i32,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set,
        );
        if ret == 0 {
            ok(format!(
                "Pinned PID {pid} to cores {:?}",
                cores
            ))
        } else {
            err(format!(
                "Failed to set affinity for PID {pid}: {}",
                std::io::Error::last_os_error()
            ))
        }
    }
}

/// Find the cgroup v2 path a process currently belongs to, under /sys/fs/cgroup.
fn process_cgroup_path(pid: u32) -> Result<String, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|e| format!("Cannot read /proc/{pid}/cgroup: {e}"))?;
    // cgroup v2 unified hierarchy: a single line "0::/path"
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("0::") {
            return Ok(format!("/sys/fs/cgroup{rest}"));
        }
    }
    Err("Process is not on the unified (v2) cgroup hierarchy".to_string())
}

pub fn set_cpu_limit(pid: u32, percent: u32) -> ActionResult {
    let path = match process_cgroup_path(pid) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    // cpu.max format: "<quota> <period>", quota/period ≈ fraction of a core.
    let period = 100_000u32;
    let quota = (period as u64 * percent as u64 / 100).max(1000) as u32;
    let value = format!("{quota} {period}");
    match fs::write(format!("{path}/cpu.max"), &value) {
        Ok(()) => ok(format!(
            "Limited PID {pid}'s cgroup to {percent}% CPU ({path}/cpu.max)"
        )),
        Err(e) => err(format!(
            "Failed to write cpu.max at {path}: {e} (this cgroup may not be user-writable — often needs root or systemd delegation)"
        )),
    }
}

pub fn set_memory_limit(pid: u32, mb: u64) -> ActionResult {
    let path = match process_cgroup_path(pid) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let bytes = mb * 1024 * 1024;
    match fs::write(format!("{path}/memory.max"), bytes.to_string()) {
        Ok(()) => ok(format!(
            "Limited PID {pid}'s cgroup to {mb} MB ({path}/memory.max)"
        )),
        Err(e) => err(format!(
            "Failed to write memory.max at {path}: {e} (this cgroup may not be user-writable — often needs root or systemd delegation)"
        )),
    }
}

pub fn set_oom_score_adj(pid: u32, score: i32) -> ActionResult {
    let clamped = score.clamp(-1000, 1000);
    match fs::write(format!("/proc/{pid}/oom_score_adj"), clamped.to_string()) {
        Ok(()) => ok(format!("Set PID {pid} oom_score_adj to {clamped}")),
        Err(e) => err(format!("Failed to set oom_score_adj for PID {pid}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::{thread, time::Duration};

    fn proc_state(pid: u32) -> char {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        // Format: "pid (comm) state ...". comm may contain spaces/parens, so
        // split after the last ')'.
        let after_comm = stat.rsplit_once(')').unwrap().1;
        after_comm.trim_start().chars().next().unwrap()
    }

    #[test]
    fn freeze_resume_and_kill_a_throwaway_process() {
        let mut child = Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn throwaway sleep process");
        let pid = child.id();
        thread::sleep(Duration::from_millis(50));

        let r = freeze(pid);
        assert!(r.ok, "{}", r.message);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(proc_state(pid), 'T', "process should be stopped");

        let r = resume(pid);
        assert!(r.ok, "{}", r.message);
        thread::sleep(Duration::from_millis(50));
        assert_ne!(proc_state(pid), 'T', "process should no longer be stopped");

        let r = kill(pid, false);
        assert!(r.ok, "{}", r.message);
        let status = child.wait().expect("wait on killed process");
        assert!(!status.success());
    }

    #[test]
    fn set_priority_on_throwaway_process_reports_result() {
        let mut child = Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn throwaway sleep process");
        let pid = child.id();
        thread::sleep(Duration::from_millis(50));

        // Lowering our own priority (positive nice) never needs privilege.
        let r = set_priority(pid, 10);
        assert!(r.ok, "{}", r.message);

        kill(pid, true);
        child.wait().ok();
    }
}
