use crate::control_process::ActionResult;
use crate::sys_util::{err, has_tool, is_safe_token, privileged_action, run_lossy};
use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct PendingUpdate {
    pub name: String,
    pub current: String,
    pub new: String,
}

#[derive(Serialize)]
pub struct PackageStatus {
    /// "pacman" | "apt" | "dnf" | "zypper" | "none"
    pub manager: String,
    pub updates: Vec<PendingUpdate>,
    /// Packages installed as dependencies that nothing needs any more.
    pub orphans: Vec<String>,
    pub cache_bytes: u64,
    pub can_clean_cache: bool,
}

fn dir_size(path: &str) -> u64 {
    run_lossy(Command::new("du").args(["-sb", path]))
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Checking for updates talks to the mirrors, so this can take several
/// seconds — it's fetched on demand, never polled.
pub fn status() -> PackageStatus {
    if has_tool("pacman") {
        // `checkupdates` (pacman-contrib) syncs into a private copy of the
        // database, so it needs no root and never leaves a partial upgrade.
        let updates = run_lossy(&mut Command::new("checkupdates"))
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f.len() == 4).then(|| PendingUpdate {
                    name: f[0].to_string(),
                    current: f[1].to_string(),
                    new: f[3].to_string(),
                })
            })
            .collect();
        let orphans = run_lossy(Command::new("pacman").arg("-Qdtq"))
            .lines()
            .map(String::from)
            .collect();
        return PackageStatus {
            manager: "pacman".to_string(),
            updates,
            orphans,
            cache_bytes: dir_size("/var/cache/pacman/pkg"),
            can_clean_cache: has_tool("paccache"),
        };
    }
    if has_tool("apt") {
        // "name/suite new-version arch [upgradable from: old-version]"
        let updates = run_lossy(Command::new("apt").args(["list", "--upgradable"]))
            .lines()
            .filter_map(|l| {
                let (name, rest) = l.split_once('/')?;
                let f: Vec<&str> = rest.split_whitespace().collect();
                Some(PendingUpdate {
                    name: name.to_string(),
                    current: f.last()?.trim_end_matches(']').to_string(),
                    new: f.get(1)?.to_string(),
                })
            })
            .collect();
        // What `apt autoremove` would take: "Remv name [version]".
        let orphans = run_lossy(Command::new("apt-get").args(["-s", "autoremove"]))
            .lines()
            .filter_map(|l| l.strip_prefix("Remv ").and_then(|r| r.split_whitespace().next()).map(String::from))
            .collect();
        return PackageStatus {
            manager: "apt".to_string(),
            updates,
            orphans,
            cache_bytes: dir_size("/var/cache/apt/archives"),
            can_clean_cache: true,
        };
    }
    if has_tool("dnf") {
        // "name.arch  version  repo"; -q drops headers. Exit code 100 means updates exist.
        let out = Command::new("dnf").args(["-q", "check-update"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let updates = out
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f.len() == 3 && f[0].contains('.')).then(|| PendingUpdate {
                    name: f[0].rsplit_once('.').map(|(n, _)| n).unwrap_or(f[0]).to_string(),
                    current: String::new(),
                    new: f[1].to_string(),
                })
            })
            .collect();
        let orphans = run_lossy(Command::new("dnf").args(["-q", "repoquery", "--unneeded", "--qf", "%{name}"])).lines().map(String::from).collect();
        return PackageStatus {
            manager: "dnf".to_string(),
            updates,
            orphans,
            cache_bytes: dir_size("/var/cache/dnf") + dir_size("/var/cache/libdnf5"),
            can_clean_cache: true,
        };
    }
    if has_tool("zypper") {
        // "v | repo | name | current | new | arch"
        let updates = run_lossy(Command::new("zypper").args(["-q", "--no-refresh", "list-updates"]))
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('|').map(str::trim).collect();
                (f.len() >= 5 && f[0] == "v").then(|| PendingUpdate { name: f[2].to_string(), current: f[3].to_string(), new: f[4].to_string() })
            })
            .collect();
        return PackageStatus {
            manager: "zypper".to_string(),
            updates,
            orphans: Vec::new(),
            cache_bytes: dir_size("/var/cache/zypp/packages"),
            can_clean_cache: true,
        };
    }
    PackageStatus {
        manager: "none".to_string(),
        updates: Vec::new(),
        orphans: Vec::new(),
        cache_bytes: 0,
        can_clean_cache: false,
    }
}

pub fn clean_cache() -> ActionResult {
    if has_tool("paccache") {
        // Keeps the two most recent versions of each package for downgrades.
        return privileged_action("paccache -rk2 && paccache -ruk0", "Cleaned old packages from the cache (kept the last 2 versions of each)");
    }
    if has_tool("apt") {
        return privileged_action("apt-get clean", "Cleaned the apt package cache");
    }
    if has_tool("dnf") {
        return privileged_action("dnf clean packages", "Cleaned the dnf package cache");
    }
    if has_tool("zypper") {
        return privileged_action("zypper -n clean --all", "Cleaned the zypper package cache");
    }
    err("No supported package cache found")
}

pub fn remove_orphans() -> ActionResult {
    if has_tool("apt-get") && !has_tool("pacman") {
        return privileged_action("DEBIAN_FRONTEND=noninteractive apt-get -y autoremove", "Removed packages nothing needs any more");
    }
    if has_tool("dnf") && !has_tool("pacman") {
        return privileged_action("dnf -y autoremove", "Removed packages nothing needs any more");
    }
    if !has_tool("pacman") {
        return err("Removing unneeded packages isn't supported with this package manager");
    }
    let orphans: Vec<String> = run_lossy(Command::new("pacman").arg("-Qdtq"))
        .lines()
        .filter(|name| is_safe_token(name))
        .map(String::from)
        .collect();
    if orphans.is_empty() {
        return err("There are no orphaned packages");
    }
    privileged_action(
        &format!("pacman -Rns --noconfirm {}", orphans.join(" ")),
        format!("Removed {} orphaned package(s)", orphans.len()),
    )
}
