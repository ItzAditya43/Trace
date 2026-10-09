//! Named bundles of settings, and suggestions drawn from this machine's own state.

use crate::sys_util::data_dir;
use crate::tweaks::{Change, TweakState};
use serde::{Deserialize, Serialize};

struct Builtin {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    changes: &'static [(&'static str, &'static str)],
}

/// Stands for "the value this setting had when Trace first saw it".
const ORIGINAL: &str = "@original";

const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "battery",
        name: "Battery saver",
        description: "Trades speed and eye-candy for hours: low-power CPU profile, no turbo, sleeping radios and devices, no blur.",
        changes: &[
            ("power.profile", "power-saver"),
            ("cpu.boost", "off"),
            ("cpu.turbo_intel", "off"),
            ("pcie.aspm", "powersave"),
            ("usb.runtime_pm", "on"),
            ("wifi.power_save", "on"),
            ("audio.power_save", "1"),
            ("kernel.nmi_watchdog", "off"),
            ("vm.dirty_writeback_centisecs", "1500"),
            ("hypr.blur", "off"),
        ],
    },
    Builtin {
        id: "balanced",
        name: "Balanced",
        description: "The everyday middle ground. Also the way back from Battery saver, Performance, Gaming or Quiet.",
        changes: &[
            ("power.profile", "balanced"),
            ("platform.profile", "balanced"),
            ("cpu.boost", "on"),
            ("cpu.turbo_intel", "on"),
            ("cpu.max_freq", "100"),
            ("power.keep_awake", "off"),
            // Everything else the power presets touch goes back to how
            // this machine was when Trace first saw it.
            ("pcie.aspm", ORIGINAL),
            ("usb.runtime_pm", ORIGINAL),
            ("wifi.power_save", ORIGINAL),
            ("audio.power_save", ORIGINAL),
            ("kernel.nmi_watchdog", ORIGINAL),
            ("kernel.split_lock_mitigate", ORIGINAL),
            ("vm.compaction_proactiveness", ORIGINAL),
            ("vm.max_map_count", ORIGINAL),
            ("vm.dirty_writeback_centisecs", ORIGINAL),
            ("hypr.blur", ORIGINAL),
            ("hypr.vrr", ORIGINAL),
            ("hypr.allow_tearing", ORIGINAL),
        ],
    },
    Builtin {
        id: "performance",
        name: "Performance",
        description: "Everything the hardware has, for compiling, rendering or exporting. Louder fans and shorter battery life.",
        changes: &[
            ("power.profile", "performance"),
            ("platform.profile", "performance"),
            ("cpu.boost", "on"),
            ("cpu.turbo_intel", "on"),
            ("cpu.max_freq", "100"),
            ("wifi.power_save", "off"),
        ],
    },
    Builtin {
        id: "gaming",
        name: "Gaming",
        description: "Performance, plus the switches games care about: no split-lock throttling, no background memory compaction, adaptive sync and tearing for full-screen windows, and the machine stays awake.",
        changes: &[
            ("power.profile", "performance"),
            ("platform.profile", "performance"),
            ("cpu.boost", "on"),
            ("cpu.turbo_intel", "on"),
            ("cpu.max_freq", "100"),
            ("kernel.split_lock_mitigate", "off"),
            ("vm.compaction_proactiveness", "0"),
            ("vm.max_map_count", "2147483642"),
            ("wifi.power_save", "off"),
            ("hypr.vrr", "2"),
            ("hypr.allow_tearing", "on"),
            ("power.keep_awake", "on"),
        ],
    },
    Builtin {
        id: "quiet",
        name: "Quiet and cool",
        description: "Keeps the fans down: low-power profile, the firmware's quiet mode, no turbo, and CPU speed capped at 70%.",
        changes: &[
            ("power.profile", "power-saver"),
            ("platform.profile", "quiet"),
            ("cpu.boost", "off"),
            ("cpu.turbo_intel", "off"),
            ("cpu.max_freq", "70"),
        ],
    },
    Builtin {
        id: "zram",
        name: "Tuned for compressed-RAM swap (zram)",
        description: "The memory settings distributions ship alongside zram: swap early and cheaply, read one page at a time, reclaim sooner, drop the redundant zswap layer, and kill a process rather than freeze when RAM truly runs out.",
        changes: &[
            ("vm.swappiness", "180"),
            ("vm.page-cluster", "0"),
            ("vm.watermark_boost_factor", "0"),
            ("vm.watermark_scale_factor", "125"),
            ("zswap.enabled", "off"),
            ("mglru.min_ttl", "1000"),
        ],
    },
    Builtin {
        id: "light",
        name: "Lightweight desktop",
        description: "Turns off the effects that cost graphics power: animations, blur, shadows and dimming.",
        changes: &[
            ("hypr.animations", "off"),
            ("hypr.blur", "off"),
            ("hypr.shadow", "off"),
            ("hypr.dim_inactive", "off"),
            ("gtk.animations", "off"),
        ],
    },
    Builtin {
        id: "hardening",
        name: "Security hardening",
        description: "Widely recommended kernel and network hardening that ordinary desktop use doesn't notice, plus the firewall. It does stop debuggers from attaching to programs they didn't start.",
        changes: &[
            ("kernel.dmesg_restrict", "on"),
            ("kernel.kptr_restrict", "1"),
            ("kernel.yama.ptrace_scope", "1"),
            ("kernel.unprivileged_bpf_disabled", "2"),
            ("net.accept_redirects", "off"),
            ("net.send_redirects", "off"),
            ("net.ipv4.tcp_syncookies", "on"),
            ("net.ipv4.tcp_rfc1337", "on"),
            ("fs.protected_symlinks", "on"),
            ("fs.protected_hardlinks", "on"),
            ("dev.tty.legacy_tiocsti", "off"),
            ("svc.firewall", "on"),
        ],
    },
    Builtin {
        id: "developer",
        name: "Developer workstation",
        description: "Raises the limits dev tools run into and opens up profiling and debugging: more file watchers, perf without sudo, and debuggers may attach to any of your programs (a deliberate loosening of security).",
        changes: &[
            ("fs.inotify.max_user_watches", "1048576"),
            ("fs.inotify.max_user_instances", "8192"),
            ("vm.max_map_count", "1048576"),
            ("kernel.perf_event_paranoid", "1"),
            ("kernel.yama.ptrace_scope", "0"),
            ("kernel.task_delayacct", "on"),
        ],
    },
];

#[derive(Serialize, Deserialize, Clone)]
struct SavedPreset {
    name: String,
    changes: Vec<(String, String)>,
}

fn saved_path() -> std::path::PathBuf {
    data_dir().join("tweak-presets.json")
}

fn load_saved() -> Vec<SavedPreset> {
    std::fs::read_to_string(saved_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn store_saved(presets: &[SavedPreset]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    std::fs::write(saved_path(), text).map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
pub struct PresetChange {
    pub id: String,
    pub title: String,
    pub value: String,
    pub value_label: String,
    pub current: String,
    /// False when the setting already has this value.
    pub differs: bool,
    pub needs_root: bool,
}

#[derive(Serialize, Clone)]
pub struct PresetView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    /// Only the settings this machine has, with a value it accepts.
    pub changes: Vec<PresetChange>,
    /// Settings in the preset that this machine doesn't have.
    pub skipped: usize,
}

fn label_for(state: &TweakState, value: &str) -> String {
    match state.kind.as_str() {
        "toggle" => value.to_string(),
        "number" => format!("{value}{}", if state.unit.is_empty() { String::new() } else { format!(" {}", state.unit) }),
        _ => state
            .options
            .iter()
            .find(|o| o.value == value)
            .map(|o| o.label.clone())
            .unwrap_or_else(|| value.to_string()),
    }
}

fn resolve(pairs: &[(String, String)], states: &[TweakState]) -> (Vec<PresetChange>, usize) {
    let mut skipped = 0;
    let changes = pairs
        .iter()
        .filter_map(|(id, value)| {
            let state = states.iter().find(|s| s.id == *id);
            let value = &match (value.as_str(), state) {
                (ORIGINAL, Some(s)) => s.original.clone(),
                _ => value.clone(),
            };
            let usable = state.filter(|s| s.kind != "choice" || s.options.iter().any(|o| o.value == *value));
            let Some(state) = usable else {
                skipped += 1;
                return None;
            };
            Some(PresetChange {
                id: id.clone(),
                title: state.title.clone(),
                value: value.clone(),
                value_label: label_for(state, value),
                current: label_for(state, &state.value),
                differs: state.value != *value,
                needs_root: state.needs_root,
            })
        })
        .collect();
    (changes, skipped)
}

pub fn list(states: &[TweakState]) -> Vec<PresetView> {
    let builtin = BUILTIN.iter().map(|b| {
        let pairs: Vec<(String, String)> = b.changes.iter().map(|(i, v)| (i.to_string(), v.to_string())).collect();
        let (changes, skipped) = resolve(&pairs, states);
        PresetView {
            id: b.id.to_string(),
            name: b.name.to_string(),
            description: b.description.to_string(),
            builtin: true,
            changes,
            skipped,
        }
    });
    let saved = load_saved().into_iter().map(|s| {
        let (changes, skipped) = resolve(&s.changes, states);
        PresetView {
            id: format!("saved:{}", s.name),
            name: s.name,
            description: "Saved by you.".to_string(),
            builtin: false,
            changes,
            skipped,
        }
    });
    builtin.chain(saved).collect()
}

/// The changes a preset would make right now (settings already at the
/// preset's value are left out).
pub fn changes_for(id: &str, states: &[TweakState]) -> Option<Vec<Change>> {
    let preset = list(states).into_iter().find(|p| p.id == id || p.name == id)?;
    Some(
        preset
            .changes
            .into_iter()
            .filter(|c| c.differs)
            .map(|c| Change { id: c.id, value: c.value })
            .collect(),
    )
}

/// Saves every setting that currently differs from its original value.
pub fn save_current(name: &str, states: &[TweakState]) -> Result<usize, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err("Give the preset a name of 1–60 characters".to_string());
    }
    if BUILTIN.iter().any(|b| b.name.eq_ignore_ascii_case(name)) {
        return Err("That name belongs to a built-in preset".to_string());
    }
    let changes: Vec<(String, String)> = states
        .iter()
        .filter(|s| s.changed)
        .map(|s| (s.id.clone(), s.value.clone()))
        .collect();
    if changes.is_empty() {
        return Err("Nothing differs from the original settings yet, so there is nothing to save".to_string());
    }
    let mut saved = load_saved();
    saved.retain(|s| s.name != name);
    let count = changes.len();
    saved.push(SavedPreset { name: name.to_string(), changes });
    store_saved(&saved)?;
    Ok(count)
}

pub fn delete_saved(name: &str) -> Result<(), String> {
    let mut saved = load_saved();
    let before = saved.len();
    saved.retain(|s| s.name != name);
    if saved.len() == before {
        return Err("No saved preset with that name".to_string());
    }
    store_saved(&saved)
}

// ---- recommendations ----

#[derive(Serialize, Clone)]
pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub value: String,
    pub value_label: String,
    pub current: String,
    pub reason: String,
    pub needs_root: bool,
}

/// Facts about this machine that the suggestions depend on.
pub struct Facts {
    pub zram_swap: bool,
    pub has_ssd: bool,
    pub has_battery: bool,
    pub battery_wear_percent: f32,
    pub journal_bytes: u64,
    pub memory_gb: f64,
}

impl Facts {
    pub fn gather() -> Self {
        let batteries = crate::health::battery_health();
        Facts {
            zram_swap: std::fs::read_to_string("/proc/swaps").is_ok_and(|s| s.contains("zram")),
            has_ssd: crate::health::list_drives().iter().any(|d| !d.rotational),
            has_battery: !batteries.is_empty(),
            battery_wear_percent: batteries.iter().map(|b| b.wear_percent).fold(0.0, f32::max),
            journal_bytes: crate::desktop::journal_bytes(),
            memory_gb: std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|m| m.lines().next()?.split_whitespace().nth(1)?.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
                / 1024.0,
        }
    }
}

/// Suggestions that follow from how this particular machine is set up.
/// Deliberately short: only things with a clear reason here and now.
pub fn recommendations(states: &[TweakState], facts: &Facts) -> Vec<Recommendation> {
    let get = |id: &str| states.iter().find(|s| s.id == id);
    let value = |id: &str| get(id).map(|s| s.value.as_str()).unwrap_or("");
    let number = |id: &str| value(id).parse::<f64>().ok();
    let mut out = Vec::new();
    let mut suggest = |id: &str, target: &str, reason: String| {
        let Some(state) = get(id) else {
            return;
        };
        if state.value == target || (state.kind == "choice" && !state.options.iter().any(|o| o.value == target)) {
            return;
        }
        out.push(Recommendation {
            id: id.to_string(),
            title: state.title.clone(),
            value: target.to_string(),
            value_label: label_for(state, target),
            current: label_for(state, &state.value),
            reason,
            needs_root: state.needs_root,
        });
    };

    if facts.zram_swap {
        if value("zswap.enabled") == "on" {
            suggest("zswap.enabled", "off",
                "Your swap is zram, which already compresses in RAM. With zswap on as well, every page is compressed twice and two pools compete for the same memory.".to_string());
        }
        if value("vm.watermark_boost_factor") != "0" {
            suggest("vm.watermark_boost_factor", "0",
                "With zram, this kernel behaviour causes bursts of needless swapping. Turning it off is part of the standard zram tuning.".to_string());
        }
        if number("vm.watermark_scale_factor").is_some_and(|n| n < 125.0) {
            suggest("vm.watermark_scale_factor", "125",
                "Starts background memory clean-up earlier, so apps are less likely to stall waiting for free memory. Part of the standard zram tuning.".to_string());
        }
        if number("vm.page-cluster").is_some_and(|n| n > 0.0) {
            suggest("vm.page-cluster", "0",
                "Reading several pages at once from compressed RAM swap only wastes time; one at a time is fastest.".to_string());
        }
    }
    if facts.memory_gb > 0.0 && facts.memory_gb <= 16.5 && number("mglru.min_ttl") == Some(0.0) {
        suggest("mglru.min_ttl", "1000", format!(
            "With {:.0} GB of RAM this machine can run out. This makes the kernel end one program at that point instead of the whole desktop freezing while it thrashes.",
            facts.memory_gb
        ));
    }
    if facts.has_battery && facts.battery_wear_percent >= 15.0 && number("battery.charge_limit") == Some(100.0) {
        suggest("battery.charge_limit", "80", format!(
            "This battery has already lost {:.0}% of its capacity. Stopping at 80% slows further wear considerably; raise it again before a day away from a socket.",
            facts.battery_wear_percent
        ));
    }
    if facts.has_ssd && value("svc.fstrim") == "off" {
        suggest("svc.fstrim", "on", "You have an SSD and nothing is trimming it. A weekly trim keeps it fast and extends its life.".to_string());
    }
    if value("svc.wait_online") == "on" {
        suggest("svc.wait_online", "off", "It delays boot until the network is up, which only matters if network drives are mounted at start.".to_string());
    }
    if value("svc.firewall") == "off" {
        suggest("svc.firewall", "on", "A firewall is installed but not running, so every listening program is reachable from any network you join.".to_string());
    }
    if value("svc.printer_discovery") == "on" {
        suggest("svc.printer_discovery", "off", "Automatic printer discovery listens on the network and has had serious security holes. Printing to printers you already added keeps working.".to_string());
    }
    if facts.has_battery && value("kernel.nmi_watchdog") == "on" {
        suggest("kernel.nmi_watchdog", "off", "It wakes every core several times a second for a check only kernel developers need, which costs battery.".to_string());
    }
    if facts.journal_bytes > 1_500_000_000 && value("journald.max_use").is_empty() {
        suggest("journald.max_use", "500M", format!(
            "The system journal is using {:.1} GB. A limit keeps months of logs without the bulk.",
            facts.journal_bytes as f64 / 1e9
        ));
    }
    if number("fs.inotify.max_user_watches").is_some_and(|n| n < 524288.0) {
        suggest("fs.inotify.max_user_watches", "524288", "The current limit is low enough that editors and build tools on a large project will run out of file watchers.".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tweaks::{self, Kind};

    #[test]
    fn every_builtin_preset_refers_to_real_settings_with_valid_values() {
        let mut ids = std::collections::HashSet::new();
        for preset in BUILTIN {
            assert!(ids.insert(preset.id), "duplicate preset {}", preset.id);
            for (id, value) in preset.changes {
                let tweak = tweaks::find(id).unwrap_or_else(|| panic!("{}: unknown setting {id}", preset.id));
                if *value == ORIGINAL {
                    continue;
                }
                match &tweak.kind {
                    Kind::Toggle => assert!(matches!(*value, "on" | "off"), "{}: {id}={value}", preset.id),
                    Kind::Number { min, max, .. } => {
                        let n: f64 = value.parse().unwrap_or_else(|_| panic!("{}: {id}={value}", preset.id));
                        assert!(n >= *min && n <= *max, "{}: {id}={value} out of range", preset.id);
                    }
                    // Discovered choices can only be checked on a real machine.
                    Kind::Choice(options) => assert!(
                        options.is_empty() || options.iter().any(|(v, _)| v == value),
                        "{}: {id}={value} isn't an option",
                        preset.id
                    ),
                }
            }
        }
    }
}
