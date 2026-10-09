use crate::memory_map::AppMemory;
use crate::sys_util::{data_dir, local_time, notify};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;

/// "If <app> uses more than <threshold>, do <action>".
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub app: String,
    pub threshold_mb: u64,
    /// Count swapped-out memory towards the threshold too.
    #[serde(default)]
    pub include_swap: bool,
    /// "notify" | "freeze" | "kill" | "restart"
    pub action: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Trigger {
    /// Local time; `days` uses 0 = Sunday and empty means every day.
    Time { hour: u32, minute: u32, #[serde(default)] days: Vec<u32> },
    AppLaunch { app: String },
    /// The charger was unplugged / plugged in.
    OnBattery,
    OnCharger,
    /// The battery fell to this level while discharging.
    BatteryBelow { percent: u32 },
    /// A Wi-Fi network with this name was joined ("" = any network not listed elsewhere).
    WifiJoined { ssid: String },
    /// An external monitor was connected / the last one was removed.
    MonitorConnected,
    MonitorDisconnected,
}

/// What the triggers look at, sampled once per tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Situation {
    pub on_battery: bool,
    pub battery_percent: u32,
    pub wifi: String,
    pub monitors: usize,
}

impl Situation {
    pub fn read() -> Self {
        let battery = crate::health::battery_health().into_iter().next();
        Situation {
            on_battery: battery.as_ref().is_some_and(|b| b.status == "Discharging"),
            battery_percent: battery.map(|b| b.percent.round() as u32).unwrap_or(100),
            wifi: crate::sys_util::run_lossy(
                std::process::Command::new("nmcli").args(["-t", "-f", "active,ssid", "dev", "wifi"]),
            )
            .lines()
            .find_map(|l| l.strip_prefix("yes:").map(str::to_string))
            .unwrap_or_default(),
            monitors: std::fs::read_dir("/sys/class/drm")
                .map(|d| {
                    d.flatten()
                        .filter(|e| {
                            std::fs::read_to_string(e.path().join("status")).is_ok_and(|s| s.trim() == "connected")
                        })
                        .count()
                })
                .unwrap_or(1),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Schedule {
    pub id: String,
    pub profile: String,
    pub trigger: Trigger,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "yes")]
    pub alerts_enabled: bool,
    /// Closing the window hides Trace instead of quitting it.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    /// Settings shown as one-click switches on the Overview page.
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Apps that are frozen after going unused for a while.
    #[serde(default)]
    pub sleepers: Vec<crate::extras::Sleeper>,
    /// Settings changed while the machine is hot, and put back when it cools.
    #[serde(default)]
    pub thermal: Vec<crate::extras::ThermalRule>,
    /// Monthly data allowances per network interface.
    #[serde(default)]
    pub data_budgets: Vec<crate::more::DataBudget>,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            alerts_enabled: true,
            close_to_tray: true,
            rules: Vec::new(),
            schedules: Vec::new(),
            pinned: Vec::new(),
            sleepers: Vec::new(),
            thermal: Vec::new(),
            data_budgets: Vec::new(),
        }
    }
}

fn config_path() -> std::path::PathBuf {
    data_dir().join("automation.json")
}

pub fn load() -> Config {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(config_path(), text).map_err(|e| e.to_string())
}

/// What the background loop remembers between ticks so that rules, schedules
/// and alerts fire once per occurrence instead of every tick.
#[derive(Default)]
pub struct State {
    last_fired: HashMap<String, i64>,
    schedule_last_day: HashMap<String, u32>,
    running_apps: Option<HashSet<String>>,
    situation: Option<Situation>,
}

const RULE_COOLDOWN_SECS: i64 = 600;
const ALERT_COOLDOWN_SECS: i64 = 1800;

impl State {
    fn cooled_down(&mut self, key: &str, now: i64, cooldown: i64) -> bool {
        if self.last_fired.get(key).is_some_and(|last| now - last < cooldown) {
            return false;
        }
        self.last_fired.insert(key.to_string(), now);
        true
    }

    /// Desktop notification, at most once per `ALERT_COOLDOWN_SECS` per title.
    pub fn alert(&mut self, title: &str, body: &str, now: i64) {
        if self.cooled_down(&format!("alert:{title}"), now, ALERT_COOLDOWN_SECS) {
            notify(title, body, true);
        }
    }
}

pub struct RuleOutcome {
    pub description: String,
    pub ok: bool,
    pub message: String,
}

pub fn evaluate_rules(config: &Config, apps: &[AppMemory], state: &mut State, now: i64) -> Vec<RuleOutcome> {
    let mut outcomes = Vec::new();
    for rule in config.rules.iter().filter(|r| r.enabled) {
        let Some(app) = apps.iter().find(|a| a.name == rule.app) else {
            continue;
        };
        let used = app.ram_bytes + if rule.include_swap { app.swap_bytes } else { 0 };
        let used_mb = used / 1024 / 1024;
        if used_mb <= rule.threshold_mb {
            continue;
        }
        if !state.cooled_down(&format!("rule:{}", rule.id), now, RULE_COOLDOWN_SECS) {
            continue;
        }
        let summary = format!("{} is using {} MB (limit {} MB)", rule.app, used_mb, rule.threshold_mb);
        let result = match rule.action.as_str() {
            "notify" => crate::sys_util::ok("Notified"),
            action => crate::memory_map::app_action(&rule.app, action),
        };
        notify(
            &format!("Trace rule: {}", rule.app),
            &format!("{summary}\n{}", if rule.action == "notify" { "" } else { result.message.as_str() }),
            false,
        );
        outcomes.push(RuleOutcome {
            description: format!("rule:{} → {}", rule.app, rule.action),
            ok: result.ok,
            message: format!("{summary}. {}", result.message),
        });
    }
    outcomes
}

fn running_app_names() -> HashSet<String> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_string_lossy().to_string();
            pid.parse::<u32>().ok()?;
            crate::memory_map::app_name(&pid)
        })
        .collect()
}

/// Names of profiles whose schedule came due on this tick.
pub fn due_schedules(config: &Config, state: &mut State) -> Vec<String> {
    let active: Vec<&Schedule> = config.schedules.iter().filter(|s| s.enabled).collect();
    if active.is_empty() {
        state.running_apps = None;
        return Vec::new();
    }
    let now = local_time();
    // Situation triggers compare this tick with the last one, so each
    // fires once on the change rather than for as long as it stays true.
    let needs_situation = active
        .iter()
        .any(|s| !matches!(s.trigger, Trigger::Time { .. } | Trigger::AppLaunch { .. }));
    let situation = needs_situation.then(Situation::read);
    let before = state.situation.clone();
    let watches_launches = active.iter().any(|s| matches!(s.trigger, Trigger::AppLaunch { .. }));
    let running = watches_launches.then(running_app_names);

    let mut due = Vec::new();
    for schedule in active {
        let fire = match &schedule.trigger {
            Trigger::Time { hour, minute, days } => {
                now.hour == *hour
                    && now.minute == *minute
                    && (days.is_empty() || days.contains(&now.weekday))
                    && state.schedule_last_day.insert(schedule.id.clone(), now.yday) != Some(now.yday)
            }
            // Only a launch counts: the app must have been absent on the
            // previous tick (and there must have been a previous tick).
            Trigger::AppLaunch { app } => match (&running, &state.running_apps) {
                (Some(current), Some(previous)) => current.contains(app) && !previous.contains(app),
                _ => false,
            },
            other => match (&situation, &before) {
                (Some(now), Some(was)) => situation_fired(other, was, now),
                _ => false,
            },
        };
        if fire {
            due.push(schedule.profile.clone());
        }
    }
    state.running_apps = running;
    state.situation = situation;
    due
}

fn situation_fired(trigger: &Trigger, was: &Situation, now: &Situation) -> bool {
    match trigger {
        Trigger::OnBattery => now.on_battery && !was.on_battery,
        Trigger::OnCharger => !now.on_battery && was.on_battery,
        Trigger::BatteryBelow { percent } => {
            now.on_battery && now.battery_percent <= *percent && was.battery_percent > *percent
        }
        Trigger::WifiJoined { ssid } => !now.wifi.is_empty() && now.wifi != was.wifi && (ssid.is_empty() || *ssid == now.wifi),
        Trigger::MonitorConnected => now.monitors > was.monitors,
        Trigger::MonitorDisconnected => now.monitors < was.monitors,
        Trigger::Time { .. } | Trigger::AppLaunch { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, ram_mb: u64, swap_mb: u64) -> AppMemory {
        AppMemory {
            name: name.to_string(),
            process_count: 1,
            ram_bytes: ram_mb * 1024 * 1024,
            anon_bytes: 0,
            file_bytes: 0,
            shmem_bytes: 0,
            swap_bytes: swap_mb * 1024 * 1024,
            approximate: false,
            processes: Vec::new(),
        }
    }

    fn config(include_swap: bool) -> Config {
        Config {
            pinned: Vec::new(),
            sleepers: Vec::new(),
            thermal: Vec::new(),
            data_budgets: Vec::new(),
            alerts_enabled: true,
            close_to_tray: true,
            rules: vec![Rule {
                id: "r1".to_string(),
                app: "trace-test-app-that-does-not-exist".to_string(),
                threshold_mb: 1000,
                include_swap,
                action: "notify".to_string(),
                enabled: true,
            }],
            schedules: Vec::new(),
        }
    }

    #[test]
    fn rule_fires_once_per_cooldown_and_only_over_threshold() {
        let name = "trace-test-app-that-does-not-exist";
        let mut state = State::default();
        let under = [app(name, 900, 500)];
        assert!(evaluate_rules(&config(false), &under, &mut state, 0).is_empty());
        // Swap pushes it over only when the rule counts swap.
        assert_eq!(evaluate_rules(&config(true), &under, &mut state, 0).len(), 1);
        assert!(evaluate_rules(&config(true), &under, &mut state, 60).is_empty());
        assert_eq!(evaluate_rules(&config(true), &under, &mut state, RULE_COOLDOWN_SECS + 1).len(), 1);
    }

    #[test]
    fn situation_triggers_fire_on_the_change_only() {
        let plugged = Situation { on_battery: false, battery_percent: 80, wifi: "home".to_string(), monitors: 1 };
        let unplugged = Situation { on_battery: true, ..plugged.clone() };
        assert!(situation_fired(&Trigger::OnBattery, &plugged, &unplugged));
        assert!(!situation_fired(&Trigger::OnBattery, &unplugged, &unplugged));
        assert!(situation_fired(&Trigger::OnCharger, &unplugged, &plugged));

        let low = Situation { battery_percent: 19, ..unplugged.clone() };
        let at_21 = Situation { battery_percent: 21, ..unplugged.clone() };
        assert!(situation_fired(&Trigger::BatteryBelow { percent: 20 }, &at_21, &low));
        assert!(!situation_fired(&Trigger::BatteryBelow { percent: 20 }, &low, &low));

        let cafe = Situation { wifi: "cafe".to_string(), ..plugged.clone() };
        assert!(situation_fired(&Trigger::WifiJoined { ssid: "cafe".to_string() }, &plugged, &cafe));
        assert!(situation_fired(&Trigger::WifiJoined { ssid: String::new() }, &plugged, &cafe));
        assert!(!situation_fired(&Trigger::WifiJoined { ssid: "home".to_string() }, &plugged, &cafe));

        let docked = Situation { monitors: 2, ..plugged.clone() };
        assert!(situation_fired(&Trigger::MonitorConnected, &plugged, &docked));
        assert!(situation_fired(&Trigger::MonitorDisconnected, &docked, &plugged));
    }

    #[test]
    fn config_tolerates_missing_fields() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert!(c.alerts_enabled && c.rules.is_empty());
    }
}
