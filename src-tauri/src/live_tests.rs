//! Tests that act on the real machine. All are `#[ignore]`d.
//!
//! `user_live` runs as you: `cargo test user_live -- --ignored --test-threads=1`.
//! `root_live` needs administrator rights for everything it does, so run the
//! test binary itself as root (one password prompt instead of a dozen):
//! `pkexec env HOME=<scratch dir> <test binary> root_live --ignored --test-threads=1 --nocapture`.
//! Everything is put back afterwards, except where the point of the action
//! is to remove something (maintenance clears old logs and cached packages).

use crate::db::Db;
use crate::sys_util::{read_trimmed, run, run_lossy};
use crate::tweaks::Change;
use std::fs;
use std::process::Command;
use std::sync::Mutex;

fn memory_db() -> Db {
    let db = Db { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
    crate::tweaks::init(&db);
    db
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

mod user_live {
    use super::*;
    use crate::{batch3, batch5, batch6};

    #[test]
    #[ignore]
    fn privacy_cleaner_clears_for_real_and_the_originals_are_put_back() {
        let scratch = std::env::temp_dir().join("trace-privacy-backup");
        fs::create_dir_all(&scratch).unwrap();
        let items = batch3::privacy_items();
        println!("found: {:?}", items.iter().map(|i| (&i.id, i.bytes)).collect::<Vec<_>>());

        // Recent files list.
        let recent = dirs::data_local_dir().unwrap().join("recently-used.xbel");
        if recent.exists() {
            fs::copy(&recent, scratch.join("recent")).unwrap();
            let r = batch3::privacy_clean("recent");
            assert!(r.ok, "{}", r.message);
            assert_eq!(fs::metadata(&recent).unwrap().len(), 0);
            fs::copy(scratch.join("recent"), &recent).unwrap();
            println!("recent list: cleared, then restored ({} bytes)", fs::metadata(&recent).unwrap().len());
        }

        // Clipboard history.
        let db = dirs::cache_dir().unwrap().join("cliphist/db");
        if db.exists() && crate::sys_util::has_tool("cliphist") {
            let before = run_lossy(Command::new("cliphist").arg("list")).lines().count();
            fs::copy(&db, scratch.join("cliphist-db")).unwrap();
            let r = batch3::privacy_clean("clipboard");
            assert!(r.ok, "{}", r.message);
            assert_eq!(run_lossy(Command::new("cliphist").arg("list")).lines().count(), 0);
            fs::copy(scratch.join("cliphist-db"), &db).unwrap();
            let after = run_lossy(Command::new("cliphist").arg("list")).lines().count();
            println!("clipboard: {before} entries wiped, then restored ({after})");
            assert_eq!(after, before);
        }

        // A browser cache: the smallest one, which is only ever a cache.
        if let Some(item) = items.iter().filter(|i| i.id.starts_with("browser:")).min_by_key(|i| i.bytes) {
            let dir = dirs::cache_dir().unwrap().join(item.id.trim_start_matches("browser:"));
            let r = batch3::privacy_clean(&item.id);
            println!("{}: {}", item.title, r.message);
            assert!(r.ok && !dir.exists());
        }
        assert!(!batch3::privacy_clean("browser:../../Documents").ok, "only known browser folders");
        fs::remove_dir_all(scratch).ok();
    }

    #[test]
    #[ignore]
    fn game_mode_applies_a_preset_for_a_game_and_puts_everything_back() {
        use std::collections::HashMap;
        use std::os::unix::process::CommandExt;
        let db = memory_db();
        let values = |db: &Db| -> HashMap<String, String> { crate::tweaks::list(db).into_iter().map(|t| (t.id, t.value)).collect() };
        let differing = |a: &HashMap<String, String>, b: &HashMap<String, String>| -> Vec<String> {
            let mut ids: Vec<String> = a.iter().filter(|(k, v)| b.get(*k).is_some_and(|other| other != *v)).map(|(k, _)| k.clone()).collect();
            ids.sort();
            ids
        };
        assert!(batch5::running_game().is_none(), "a real game is running; not interfering");
        // A session left over from an interrupted run is finished off first.
        if let Some((_, line)) = batch5::game_tick(&db) {
            println!("leftover: {line}");
        }
        let saved_mode = batch5::game_mode();
        let before = values(&db);

        batch5::set_game_mode(&batch5::GameMode { enabled: true, preset: "gaming".into(), per_game: Default::default(), seen: saved_mode.seen.clone() });
        let mut game = Command::new("sleep").arg0("/tmp/steamapps/common/TraceTestGame/game").arg("120").spawn().unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert_eq!(batch5::running_game().as_deref(), Some("TraceTestGame"));
            let started = batch5::game_tick(&db).expect("the game is noticed");
            println!("start: {}", started.1);
            let changed = differing(&before, &values(&db));
            println!("changed while playing: {changed:?}");
            assert!(!changed.is_empty(), "the preset changed something");
            assert!(batch5::game_tick(&db).is_none(), "nothing more to do while it runs");
        }));
        game.kill().ok();
        game.wait().ok();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let ended = batch5::game_tick(&db);
        let mut restored = saved_mode;
        restored.seen.retain(|g| g != "TraceTestGame");
        batch5::set_game_mode(&restored);
        outcome.unwrap();
        println!("end: {}", ended.expect("the game ending is noticed").1);
        let left = differing(&before, &values(&db));
        assert!(left.is_empty(), "left changed after the game: {left:?}");
    }

    #[test]
    #[ignore]
    fn wide_colour_and_hdr_switch_on_and_reset() {
        for monitor in batch5::monitor_colours() {
            for mode in ["wide", "hdr"] {
                let r = batch5::set_monitor_colour(&monitor.name, mode, 1.0, 1.0, "");
                std::thread::sleep(std::time::Duration::from_secs(2));
                let now = batch5::monitor_colours().into_iter().find(|m| m.name == monitor.name).unwrap();
                println!("{} -> {mode}: accepted={} reports mode={} ten_bit={}", monitor.name, r.ok, now.mode, now.ten_bit);
                let reset = batch5::set_monitor_colour(&monitor.name, "srgb", 1.0, 1.0, "");
                std::thread::sleep(std::time::Duration::from_secs(2));
                let back = batch5::monitor_colours().into_iter().find(|m| m.name == monitor.name).unwrap();
                assert!(reset.ok && back.mode == "srgb", "{} did not reset: {}", monitor.name, back.mode);
            }
            let layout = crate::hardware::monitors();
            assert!(layout.iter().any(|m| m.setup.name == monitor.name && m.setup.enabled), "{} is still on", monitor.name);
        }
    }

    #[test]
    #[ignore]
    fn cooling_test_runs_and_reports() {
        let r = batch3::stress_test(30);
        println!("{}s: {:.0}°C -> peak {:.0}°C, lowest speed {:.0}%, fan up to {} RPM", r.seconds, r.start_c, r.peak_c, r.lowest_speed_percent, r.peak_fan_rpm);
        println!("verdict: {}", r.verdict);
        assert!(r.peak_c >= r.start_c && r.peak_c > 30.0 && !r.verdict.is_empty());
    }

    #[test]
    #[ignore]
    fn phone_alert_says_so_when_no_phone_is_reachable() {
        let phones = batch6::phones();
        println!("paired and reachable phones: {}", phones.len());
        let sent = batch6::send_to_phones("Trace: test alert");
        assert_eq!(sent, phones.len());
    }

    /// Needs a working VPN profile and two password prompts (start and stop).
    #[test]
    #[ignore]
    fn split_tunnel_sends_only_chosen_programs_through_the_vpn() {
        use crate::vpn;
        let direct = vpn::public_address().expect("internet").ip;
        let gate = vpn::connect_best_gate("JP");
        println!("vpn gate: {}", gate.message);
        assert!(gate.ok, "no VPN Gate server could be reached, so the split tunnel can't be exercised");
        let active = vpn::list().into_iter().find(|c| c.active).expect("a connected VPN");
        vpn::set_active(&active.uuid, false);

        let started = vpn::split_start(&active.uuid);
        println!("split: {}", started.message);
        assert!(started.ok);
        std::thread::sleep(std::time::Duration::from_secs(3));
        let out = std::env::temp_dir().join("trace-split-ip.txt");
        fs::remove_file(&out).ok();
        assert!(vpn::launch_in_vpn(&format!("sh -c 'curl -4 -s -m 15 https://api.ipify.org > {}'", out.display())).ok);
        std::thread::sleep(std::time::Duration::from_secs(12));
        let through_vpn = fs::read_to_string(&out).unwrap_or_default();
        let still_direct = vpn::public_address().map(|a| a.ip).unwrap_or_default();
        println!("direct {direct} | program in the VPN group {through_vpn} | everything else {still_direct}");
        let stopped = vpn::split_stop();
        vpn::delete(&active.uuid);
        assert!(stopped.ok, "{}", stopped.message);
        assert_eq!(still_direct, direct, "ordinary traffic stayed off the VPN");
        assert!(!through_vpn.is_empty() && through_vpn != direct, "the chosen program went through the VPN");
    }

    /// The SSH fixes write a small settings file; this checks sshd itself accepts it.
    #[test]
    #[ignore]
    fn the_ssh_fix_writes_settings_sshd_accepts() {
        if !crate::sys_util::has_tool("sshd") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("trace-sshd-test-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        // sshd won't even read a configuration without a host key to go with it.
        run(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(dir.join("key"))).unwrap();
        fs::write(dir.join("10-trace.conf"), "PasswordAuthentication no\nKbdInteractiveAuthentication no\n").unwrap();
        fs::write(dir.join("11-trace-root.conf"), "PermitRootLogin prohibit-password\n").unwrap();
        fs::write(dir.join("sshd_config"), format!("HostKey {d}/key\nInclude {d}/*.conf\n", d = dir.display())).unwrap();
        let out = Command::new("sshd").args(["-T", "-f"]).arg(dir.join("sshd_config")).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        println!("sshd reads it as: {:?}", text.lines().filter(|l| l.starts_with("passwordauthentication") || l.starts_with("permitrootlogin") || l.starts_with("kbdinteractive")).collect::<Vec<_>>());
        assert!(text.contains("passwordauthentication no") && text.contains("kbdinteractiveauthentication no"));
        assert!(text.contains("permitrootlogin without-password") || text.contains("permitrootlogin prohibit-password"));
        fs::remove_dir_all(dir).ok();
    }
}

mod root_live {
    use super::*;
    use crate::{batch5, batch6};

    fn require_root() {
        assert_eq!(unsafe { libc::geteuid() }, 0, "run this test binary as root (see the top of this file)");
    }

    #[test]
    #[ignore]
    fn a_maintenance_runs_by_hand_and_as_a_weekly_job() {
        require_root();
        let report = batch5::maintenance_run(now()).expect("maintenance runs");
        for line in &report {
            println!("by hand: {line}");
        }
        assert!(report.iter().any(|l| l.starts_with("Drives trimmed")) && report.iter().any(|l| l.starts_with("System log")));

        let was_on = batch6::auto_maintenance().enabled;
        let on = batch6::set_auto_maintenance(true);
        assert!(on.ok, "{}", on.message);
        assert!(batch6::auto_maintenance().enabled);
        run(Command::new("systemctl").args(["start", "trace-maintenance.service"])).expect("the job runs");
        let auto = batch6::auto_maintenance();
        println!("weekly job: {:?}", auto.last_lines);
        assert!(auto.last_lines.len() >= 3, "the job wrote its report");
        println!("next run: {}", run_lossy(Command::new("systemctl").args(["show", "trace-maintenance.timer", "-p", "NextElapseUSecRealtime", "--value"])));
        if !was_on {
            assert!(batch6::set_auto_maintenance(false).ok);
            assert!(!batch6::auto_maintenance().enabled);
            assert!(!std::path::Path::new("/etc/trace/maintenance.sh").exists());
        }
    }

    #[test]
    #[ignore]
    fn b_usb_wake_and_power_saving_flip_and_flip_back() {
        require_root();
        let devices = crate::hardware::ports().usb;
        // Something that isn't being typed or clicked on right now.
        let has_wake = |d: &&crate::hardware::UsbDevice| std::path::Path::new(&format!("/sys/bus/usb/devices/{}/power/wakeup", d.port)).exists();
        let Some(device) = devices.iter().filter(has_wake).find(|d| ["Fingerprint", "WebCam", "Camera"].iter().any(|k| d.name.contains(k))) else {
            println!("no idle USB device to try this on; skipped");
            return;
        };
        let wake = || read_trimmed(&format!("/sys/bus/usb/devices/{}/power/wakeup", device.port)).unwrap();
        let control = || read_trimmed(&format!("/sys/bus/usb/devices/{}/power/control", device.port)).unwrap();
        let (wake0, control0) = (wake(), control());
        println!("{} ({}): wake={wake0} power={control0}", device.name, device.port);

        assert!(batch5::set_usb_power(&device.port, "wake", wake0 != "enabled").ok);
        assert_ne!(wake(), wake0);
        assert!(batch5::set_usb_power(&device.port, "wake", wake0 == "enabled").ok);
        assert_eq!(wake(), wake0);

        assert!(batch5::set_usb_power(&device.port, "saving", control0 != "auto").ok);
        assert_ne!(control(), control0);
        assert!(batch5::set_usb_power(&device.port, "saving", control0 == "auto").ok);
        assert_eq!(control(), control0);
        println!("both switches changed the device and were put back");
    }

    #[test]
    #[ignore]
    fn c_a_spare_drive_is_erased_and_the_system_drive_is_refused() {
        require_root();
        // A 32 MB drive that exists only in memory, which the kernel presents as a real disk.
        run(Command::new("modprobe").args(["scsi_debug", "dev_size_mb=32", "lbpu=1", "lbprz=1"])).expect("scsi_debug loads");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let name = run_lossy(Command::new("lsblk").args(["-dno", "NAME,MODEL"])).lines().find(|l| l.contains("scsi_debug")).and_then(|l| l.split_whitespace().next().map(String::from)).expect("the test drive appears");
        let result = std::panic::catch_unwind(|| {
            fs::write(format!("/sys/block/{name}/queue/rotational"), "0").unwrap();
            run(Command::new("dd").args(["if=/dev/urandom", &format!("of=/dev/{name}"), "bs=1M", "count=4", "oflag=direct", "status=none"])).unwrap();
            let read = || {
                run(&mut Command::new("sync")).ok();
                fs::write("/proc/sys/vm/drop_caches", "1").ok();
                let out = Command::new("dd").args([&format!("if=/dev/{name}"), "bs=1M", "count=4", "iflag=direct", "status=none"]).output().unwrap().stdout;
                out.iter().filter(|b| **b != 0).count()
            };
            let before = read();
            assert!(before > 1_000_000, "the drive holds data before");

            let drives = batch6::erasable_drives();
            let test_drive = drives.iter().find(|d| d.name == name).expect("listed");
            assert!(test_drive.blocked.is_empty(), "{}", test_drive.blocked);
            let system = drives.iter().find(|d| d.blocked.contains("mounted")).expect("the system drive is listed as blocked");
            assert!(!batch6::secure_erase(&system.name, &system.name).ok, "the system drive must be refused");
            assert!(!batch6::secure_erase(&name, "wrong").ok, "the name must be typed");

            let erased = batch6::secure_erase(&name, &name);
            println!("{}", erased.message);
            assert!(erased.ok);
            let after = read();
            println!("{name}: {before} non-zero bytes before, {after} after");
            assert_eq!(after, 0, "nothing is left on the drive");
        });
        run(Command::new("rmmod").arg("scsi_debug")).ok();
        result.unwrap();
    }

    #[test]
    #[ignore]
    fn d_bbr_and_cake_apply_and_revert() {
        require_root();
        let db = memory_db();
        for (id, value, file) in [
            ("net.ipv4.tcp_congestion_control", "bbr", "/proc/sys/net/ipv4/tcp_congestion_control"),
            ("net.core.default_qdisc", "cake", "/proc/sys/net/core/default_qdisc"),
        ] {
            let original = read_trimmed(file).unwrap();
            let results = crate::tweaks::apply(&db, &[Change { id: id.into(), value: value.into() }], "live test", None);
            println!("{id}: {original} -> {value}: {}", results[0].message);
            assert!(results[0].ok, "{}", results[0].message);
            assert_eq!(read_trimmed(file).unwrap(), value);
            let back = crate::tweaks::apply(&db, &[Change { id: id.into(), value: original.clone() }], "live test", None);
            assert!(back[0].ok);
            assert_eq!(read_trimmed(file).unwrap(), original);
        }
    }

    #[test]
    #[ignore]
    fn e_blocking_one_apps_network_blocks_only_that_app() {
        require_root();
        let dir = std::env::temp_dir().join("trace-block-test");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let script = |tag: &str| format!("while [ ! -e {d}/go ]; do sleep 0.2; done; curl -4 -s -m 6 -o /dev/null https://example.com && echo reached > {d}/{tag} || echo blocked > {d}/{tag}", d = dir.display());
        let start = |unit: &str, tag: &str| {
            Command::new("systemd-run").args(["--quiet", "--collect", "--unit", unit, "sh", "-c", &script(tag)]).status().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(600));
            run_lossy(Command::new("systemctl").args(["show", unit, "-p", "MainPID", "--value"])).parse::<u32>().unwrap()
        };
        let blocked_pid = start("trace-test-blocked", "blocked-app");
        let _free_pid = start("trace-test-free", "free-app");
        let r = crate::network_control::block_process_network(blocked_pid);
        println!("{}", r.message);
        let outcome = std::panic::catch_unwind(|| {
            assert!(r.ok, "{}", r.message);
            fs::write(dir.join("go"), "").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(9));
            let read = |tag: &str| fs::read_to_string(dir.join(tag)).unwrap_or_default().trim().to_string();
            println!("blocked app: {} | other app: {}", read("blocked-app"), read("free-app"));
            assert_eq!(read("blocked-app"), "blocked");
            assert_eq!(read("free-app"), "reached");
        });
        let un = crate::network_control::unblock_all_network();
        assert!(un.ok, "{}", un.message);
        assert!(run_lossy(Command::new("nft").args(["list", "tables"])).lines().all(|l| !l.contains("trace_block")));
        fs::remove_dir_all(&dir).ok();
        outcome.unwrap();
    }

    #[test]
    #[ignore]
    fn f_turning_the_firewall_on_is_safe_to_repeat() {
        require_root();
        if !crate::sys_util::has_tool("firewall-cmd") {
            return;
        }
        let was_active = run_lossy(Command::new("systemctl").args(["is-active", "firewalld"])) == "active";
        let r = batch6::security_fix("firewall_on");
        println!("{}", r.message);
        assert!(r.ok);
        assert_eq!(run_lossy(Command::new("systemctl").args(["is-active", "firewalld"])), "active");
        if !was_active {
            run(Command::new("systemctl").args(["disable", "--now", "firewalld"])).ok();
        }
    }

}
