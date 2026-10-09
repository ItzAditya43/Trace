//! The settings engine behind the Tweaks page.
//!
//! Every setting in the catalog (`tweak_catalog.rs`) is data: where it lives
//! (a sysctl, a sysfs file, a systemd unit, a Hyprland option, …) and what
//! kind of control it is. This module knows how to read each kind, validate
//! a new value, apply a batch of changes with a single password prompt,
//! verify they took, remember what was there before, and keep chosen
//! settings across reboots.

use crate::db::Db;
use crate::hypr;
use crate::sys_util::{has_tool, read_trimmed, run, run_lossy, run_privileged};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Harmless to flip either way.
    Safe,
    /// Can change how something behaves in a way you would notice.
    Careful,
    /// Can break apps or hardware behaviour if set wrong; hidden by default.
    Expert,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Persist {
    /// Resets at reboot unless kept; kept ones are re-applied at boot by a
    /// small root-owned service.
    Boot,
    /// Resets when the session ends; kept ones are re-applied when Trace starts.
    Session,
    /// The system itself remembers it.
    Inherent,
    /// Can't be kept.
    Never,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Toggle,
    /// (value, label). Empty means the choices are discovered when reading.
    Choice(Vec<(&'static str, &'static str)>),
    Number { min: f64, max: f64, step: f64, unit: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Custom {
    PowerProfile,
    CpuMaxFreq,
    WifiPowerSave,
    Radio(&'static str),
    KeyboardBacklight,
    KeepAwake,
    TimeSync,
    LocalRtc,
    Dns,
    AudioRate,
    AudioQuantum,
    MicMute,
    Webcam,
    FirewallZone,
    ConnectivityCheck,
}

#[derive(Clone, Copy, Debug)]
pub enum Reader {
    /// `systemd-analyze cat-config <name>`
    CatConfig(&'static str),
    /// A property of logind's Manager object (the effective value).
    Logind(&'static str),
    /// `NetworkManager --print-config`
    NmConfig,
}

/// A key in a config drop-in file that Trace owns.
#[derive(Clone, Copy, Debug)]
pub struct DropIn {
    pub file: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub reader: Reader,
    /// Shell command that makes the service re-read its config ("" = none).
    pub reload: &'static str,
}

#[derive(Clone, Debug)]
pub enum Src {
    /// One or more sysctl keys that are set together (the first is read).
    Sysctl(&'static [&'static str]),
    /// Glob patterns of sysfs files that are set together (the first match is read).
    Sysfs(&'static [&'static str]),
    Unit { units: &'static [&'static str], user: bool, now: bool },
    Hypr(&'static str),
    Gsettings(&'static str, &'static str),
    DropIn(DropIn),
    Custom(Custom),
}

#[derive(Clone, Debug)]
pub struct Tweak {
    pub id: &'static str,
    pub category: &'static str,
    pub title: &'static str,
    pub help: &'static str,
    pub src: Src,
    pub kind: Kind,
    /// Raw values a toggle reads as on / off.
    pub on: &'static str,
    pub off: &'static str,
    /// Raw values to write, when they differ from what is read back.
    pub write_on: Option<&'static str>,
    pub write_off: Option<&'static str>,
    pub risk: Risk,
    pub persist: Option<Persist>,
    /// When a change is felt, if not immediately ("after reboot", …).
    pub effect: &'static str,
    /// For sysfs choices: a file listing the allowed values.
    pub options_file: Option<&'static str>,
    /// Root shell command run before writing (e.g. loading a module).
    pub pre: &'static str,
    /// Only offered while this system service is running.
    pub requires_active: Option<&'static str>,
}

impl Tweak {
    pub fn new(
        id: &'static str,
        category: &'static str,
        title: &'static str,
        help: &'static str,
        src: Src,
        kind: Kind,
    ) -> Self {
        Tweak {
            id,
            category,
            title,
            help,
            src,
            kind,
            on: "1",
            off: "0",
            write_on: None,
            write_off: None,
            risk: Risk::Safe,
            persist: None,
            effect: "",
            options_file: None,
            pre: "",
            requires_active: None,
        }
    }
    pub fn needs_active(mut self, unit: &'static str) -> Self {
        self.requires_active = Some(unit);
        self
    }
    pub fn onoff(mut self, on: &'static str, off: &'static str) -> Self {
        self.on = on;
        self.off = off;
        self
    }
    pub fn write_onoff(mut self, on: &'static str, off: &'static str) -> Self {
        self.write_on = Some(on);
        self.write_off = Some(off);
        self
    }
    pub fn careful(mut self) -> Self {
        self.risk = Risk::Careful;
        self
    }
    pub fn expert(mut self) -> Self {
        self.risk = Risk::Expert;
        self
    }
    pub fn persist(mut self, persist: Persist) -> Self {
        self.persist = Some(persist);
        self
    }
    pub fn effect(mut self, effect: &'static str) -> Self {
        self.effect = effect;
        self
    }
    pub fn options_from(mut self, file: &'static str) -> Self {
        self.options_file = Some(file);
        self
    }
    pub fn pre(mut self, command: &'static str) -> Self {
        self.pre = command;
        self
    }

    pub fn persistence(&self) -> Persist {
        self.persist.unwrap_or(match &self.src {
            Src::Sysctl(_) | Src::Sysfs(_) => Persist::Boot,
            Src::Hypr(_) => Persist::Session,
            Src::Unit { .. } | Src::Gsettings(..) | Src::DropIn(_) => Persist::Inherent,
            Src::Custom(_) => Persist::Never,
        })
    }
}

pub fn catalog() -> &'static [Tweak] {
    static CATALOG: OnceLock<Vec<Tweak>> = OnceLock::new();
    CATALOG.get_or_init(crate::tweak_catalog::build)
}

pub fn find(id: &str) -> Option<&'static Tweak> {
    catalog().iter().find(|t| t.id == id)
}

// ---- reading ----

#[derive(Default, Clone, Debug)]
pub struct Reading {
    /// "on"/"off" for toggles, otherwise the value as written.
    pub value: String,
    pub options: Vec<(String, String)>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub detail: String,
    pub needs_root: bool,
}

struct UnitState {
    load: String,
    active: String,
    file: String,
}

/// Things worth fetching once per pass rather than once per tweak.
pub struct ReadCtx {
    units: HashMap<(bool, String), UnitState>,
    /// Services that some settings depend on, and whether each is running.
    active: HashMap<&'static str, bool>,
    audio_settings: OnceLock<String>,
}

impl ReadCtx {
    pub fn new() -> Self {
        let mut units = HashMap::new();
        for user in [false, true] {
            let names: Vec<&str> = catalog()
                .iter()
                .filter_map(|t| match &t.src {
                    Src::Unit { units, user: u, .. } if *u == user => Some(units.iter().copied()),
                    _ => None,
                })
                .flatten()
                .collect();
            if names.is_empty() {
                continue;
            }
            let mut cmd = Command::new("systemctl");
            if user {
                cmd.arg("--user");
            }
            let out = run_lossy(
                cmd.args(["show", "--property=Id,LoadState,ActiveState,UnitFileState"]).args(&names),
            );
            // One blank-line-separated block per unit, in the order asked.
            for (block, name) in out.split("\n\n").zip(names.iter()) {
                let field = |key: &str| {
                    block
                        .lines()
                        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                        .unwrap_or("")
                        .to_string()
                };
                units.insert(
                    (user, name.to_string()),
                    UnitState {
                        load: field("LoadState"),
                        active: field("ActiveState"),
                        file: field("UnitFileState"),
                    },
                );
            }
        }
        let needed: Vec<&'static str> = {
            let mut names: Vec<&'static str> = catalog().iter().filter_map(|t| t.requires_active).collect();
            names.sort();
            names.dedup();
            names
        };
        // `is-active` prints one state per unit, in the order asked.
        let states = run_lossy(Command::new("systemctl").arg("is-active").args(&needed));
        let active = needed
            .iter()
            .zip(states.lines())
            .map(|(name, state)| (*name, state.trim() == "active"))
            .collect();
        ReadCtx {
            units,
            active,
            audio_settings: OnceLock::new(),
        }
    }

    fn audio_settings(&self) -> &str {
        self.audio_settings
            .get_or_init(|| run_lossy(Command::new("pw-metadata").args(["-n", "settings"])))
    }
}

pub fn expand_globs(patterns: &[&str]) -> Vec<String> {
    patterns
        .iter()
        .flat_map(|p| glob::glob(p).into_iter().flatten().flatten())
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

fn writable(path: &str) -> bool {
    std::ffi::CString::new(path)
        .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
        .unwrap_or(false)
}

/// "[none] mq-deadline kyber" -> ("none", [none, mq-deadline, kyber])
fn parse_bracketed(text: &str) -> Option<(String, Vec<String>)> {
    let mut current = None;
    let options: Vec<String> = text
        .split_whitespace()
        .map(|token| match token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            Some(inner) => {
                current = Some(inner.to_string());
                inner.to_string()
            }
            None => token.to_string(),
        })
        .collect();
    Some((current?, options))
}

fn toggle_value(t: &Tweak, raw: &str) -> String {
    if raw == t.on { "on" } else { "off" }.to_string()
}

fn fixed_options(t: &Tweak) -> Vec<(String, String)> {
    match &t.kind {
        Kind::Choice(options) => options.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect(),
        _ => Vec::new(),
    }
}

fn plain_options(values: &[String]) -> Vec<(String, String)> {
    values.iter().map(|v| (v.clone(), v.replace(['-', '_'], " "))).collect()
}

/// The first whitespace-separated token as a number, tidied ("10" not "10.0").
fn number_text(raw: &str) -> Option<String> {
    let n: f64 = raw.split_whitespace().next()?.parse().ok()?;
    Some(format_number(n))
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Converts a raw value into what the UI works with, for the tweak's kind.
fn shape(t: &Tweak, raw: &str, discovered: Option<Vec<String>>) -> Option<Reading> {
    let mut reading = Reading::default();
    match &t.kind {
        Kind::Toggle => reading.value = toggle_value(t, raw),
        Kind::Choice(fixed) => {
            reading.value = raw.to_string();
            reading.options = if fixed.is_empty() {
                plain_options(&discovered.unwrap_or_default())
            } else {
                fixed_options(t)
            };
            // A current value outside the list is still shown truthfully.
            if !reading.options.iter().any(|(v, _)| v == raw) {
                reading.options.push((raw.to_string(), format!("{raw} (current)")));
            }
            if reading.options.len() < 2 {
                return None;
            }
        }
        Kind::Number { .. } => reading.value = number_text(raw)?,
    }
    Some(reading)
}

fn sysctl_path(key: &str) -> String {
    format!("/proc/sys/{}", key.replace('.', "/"))
}

fn cat_config_value(conf: &str, section: &str, key: &str) -> String {
    let out = run_lossy(Command::new("systemd-analyze").args(["cat-config", conf]));
    ini_lookup(&out, section, key).unwrap_or_default()
}

/// Last uncommented `key=value` under `[section]` — later files override.
fn ini_lookup(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
        } else if current == section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    found = Some(v.trim().to_string());
                }
            }
        }
    }
    found
}

fn default_route_device() -> Option<String> {
    let out = run_lossy(Command::new("ip").args(["-4", "route", "show", "default"]));
    let tokens: Vec<&str> = out.lines().next()?.split_whitespace().collect();
    let at = tokens.iter().position(|t| *t == "dev")?;
    Some(tokens.get(at + 1)?.to_string())
}

fn wifi_interface() -> Option<String> {
    run_lossy(Command::new("iw").arg("dev"))
        .lines()
        .find_map(|l| l.trim().strip_prefix("Interface ").map(str::to_string))
}

fn keyboard_backlight() -> Option<(String, u32, u32)> {
    run_lossy(Command::new("brightnessctl").args(["-l", "-m", "-c", "leds"]))
        .lines()
        .find_map(|line| {
            // device,class,current,percent,max
            let f: Vec<&str> = line.split(',').collect();
            (f.len() >= 5 && f[0].contains("kbd_backlight"))
                .then(|| Some((f[0].to_string(), f[2].parse().ok()?, f[4].parse().ok()?)))?
        })
}

const DNS_PRESETS: [(&str, &str, &str, &str); 5] = [
    ("cloudflare", "Cloudflare (1.1.1.1)", "1.1.1.1,1.0.0.1", "2606:4700:4700::1111,2606:4700:4700::1001"),
    ("quad9", "Quad9 — blocks malware (9.9.9.9)", "9.9.9.9,149.112.112.112", "2620:fe::fe,2620:fe::9"),
    ("google", "Google (8.8.8.8)", "8.8.8.8,8.8.4.4", "2001:4860:4860::8888,2001:4860:4860::8844"),
    ("adguard", "AdGuard — blocks ads (94.140.14.14)", "94.140.14.14,94.140.15.15", "2a10:50c0::ad1:ff,2a10:50c0::ad2:ff"),
    ("mullvad", "Mullvad — no logging (194.242.2.2)", "194.242.2.2", "2a07:e340::2"),
];

/// (connection uuid, device) of the connection carrying the default route.
fn primary_connection() -> Option<(String, String)> {
    let device = default_route_device()?;
    let uuid = run(Command::new("nmcli").args(["-g", "GENERAL.CON-UUID", "device", "show", &device])).ok()?;
    (!uuid.is_empty()).then_some((uuid, device))
}

static KEEP_AWAKE: Mutex<Option<std::process::Child>> = Mutex::new(None);

fn keep_awake_active() -> bool {
    let mut guard = KEEP_AWAKE.lock().unwrap();
    match guard.as_mut().map(|child| child.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *guard = None;
            false
        }
        None => false,
    }
}

fn read_custom(t: &Tweak, custom: Custom, ctx: &ReadCtx) -> Option<Reading> {
    let mut r = Reading::default();
    match custom {
        Custom::PowerProfile => {
            if !has_tool("powerprofilesctl") {
                return None;
            }
            r.value = run(Command::new("powerprofilesctl").arg("get")).ok()?;
            let list = run_lossy(Command::new("powerprofilesctl").arg("list"));
            let names: Vec<String> = list
                .lines()
                .filter_map(|l| {
                    let name = l.trim_start_matches(['*', ' ']).strip_suffix(':')?;
                    (!name.contains(' ')).then(|| name.to_string())
                })
                .collect();
            r.options = plain_options(&names);
        }
        Custom::CpuMaxFreq => {
            let base = "/sys/devices/system/cpu/cpufreq/policy0";
            let max: f64 = read_trimmed(&format!("{base}/cpuinfo_max_freq"))?.parse().ok()?;
            let min: f64 = read_trimmed(&format!("{base}/cpuinfo_min_freq"))?.parse().ok()?;
            let now: f64 = read_trimmed(&format!("{base}/scaling_max_freq"))?.parse().ok()?;
            // Rounded to the slider's 5% steps.
            r.value = format_number(((now / max * 100.0) / 5.0).round() * 5.0);
            r.min = Some(((min / max * 100.0) / 5.0).ceil() * 5.0);
            r.detail = format!("{:.1} GHz of {:.1} GHz", now / 1e6, max / 1e6);
            r.needs_root = true;
        }
        Custom::WifiPowerSave => {
            let iface = wifi_interface()?;
            let out = run(Command::new("iw").args(["dev", &iface, "get", "power_save"])).ok()?;
            r.value = if out.contains("on") { "on" } else { "off" }.to_string();
            r.detail = iface;
            r.needs_root = true;
        }
        Custom::Radio(kind) => {
            let radio = crate::connectivity::list_radios().into_iter().find(|x| x.kind == kind)?;
            r.value = if radio.enabled { "on" } else { "off" }.to_string();
            if radio.hard_blocked {
                r.detail = "off at the hardware switch".to_string();
            }
        }
        Custom::KeyboardBacklight => {
            let (_, current, max) = keyboard_backlight()?;
            r.value = current.to_string();
            r.max = Some(max as f64);
        }
        Custom::KeepAwake => {
            if !has_tool("systemd-inhibit") {
                return None;
            }
            r.value = if keep_awake_active() { "on" } else { "off" }.to_string();
        }
        Custom::TimeSync | Custom::LocalRtc => {
            let prop = if custom == Custom::TimeSync { "NTP" } else { "LocalRTC" };
            let out = run(Command::new("timedatectl").args(["show", "-p", prop, "--value"])).ok()?;
            r.value = if out.trim() == "yes" { "on" } else { "off" }.to_string();
            r.needs_root = true;
        }
        Custom::Dns => {
            let (uuid, device) = primary_connection()?;
            let dns = run_lossy(Command::new("nmcli").args(["-g", "ipv4.dns", "connection", "show", &uuid]));
            let dns = dns.replace(' ', "");
            r.value = if dns.is_empty() {
                "auto".to_string()
            } else {
                DNS_PRESETS
                    .iter()
                    .find(|p| p.2 == dns)
                    .map(|p| p.0.to_string())
                    .unwrap_or_else(|| dns.clone())
            };
            r.options = std::iter::once(("auto".to_string(), "Automatic (from your router)".to_string()))
                .chain(DNS_PRESETS.iter().map(|p| (p.0.to_string(), p.1.to_string())))
                .collect();
            if !r.options.iter().any(|(v, _)| *v == r.value) {
                r.options.push((r.value.clone(), format!("{} (current)", r.value)));
            }
            r.detail = format!("for the current network ({device})");
        }
        Custom::AudioRate | Custom::AudioQuantum => {
            let key = if custom == Custom::AudioRate { "clock.force-rate" } else { "clock.force-quantum" };
            let line = ctx.audio_settings().lines().find(|l| l.contains(&format!("key:'{key}'")))?;
            let value = line.split("value:'").nth(1)?.split('\'').next()?;
            r.value = value.to_string();
            r.options = fixed_options(t);
            if !r.options.iter().any(|(v, _)| v == value) {
                r.options.push((value.to_string(), format!("{value} (current)")));
            }
        }
        Custom::MicMute => {
            let out = run(Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])).ok()?;
            r.value = if out.contains("MUTED") { "on" } else { "off" }.to_string();
        }
        Custom::Webcam => {
            let loaded = std::path::Path::new("/sys/module/uvcvideo").exists();
            if !loaded && run(Command::new("modinfo").args(["-n", "uvcvideo"])).is_err() {
                return None;
            }
            r.value = if loaded { "on" } else { "off" }.to_string();
            r.needs_root = true;
        }
        Custom::FirewallZone => {
            if ctx.units.get(&(false, "firewalld.service".to_string())).map(|u| u.active.as_str()) != Some("active") {
                return None;
            }
            let conf = std::fs::read_to_string("/etc/firewalld/firewalld.conf").ok()?;
            r.value = conf.lines().find_map(|l| l.strip_prefix("DefaultZone="))?.trim().to_string();
            r.options = fixed_options(t);
            if !r.options.iter().any(|(v, _)| *v == r.value) {
                r.options.push((r.value.clone(), format!("{} (current)", r.value)));
            }
            r.needs_root = true;
        }
        Custom::ConnectivityCheck => {
            let out = run(Command::new("busctl").args([
                "get-property",
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                "ConnectivityCheckEnabled",
            ]))
            .ok()?;
            r.value = if out.trim() == "b true" { "on" } else { "off" }.to_string();
            r.needs_root = true;
        }
    }
    Some(r)
}

/// Reads a tweak's current state, or None if this machine doesn't have it.
pub fn read(t: &Tweak, ctx: &ReadCtx) -> Option<Reading> {
    if let Some(unit) = t.requires_active {
        if !ctx.active.get(unit).copied().unwrap_or(false) {
            return None;
        }
    }
    match &t.src {
        Src::Sysctl(keys) => {
            let raw = read_trimmed(&sysctl_path(keys[0]))?;
            let raw = raw.replace('\t', " ");
            let mut r = shape(t, &raw, None)?;
            r.needs_root = true;
            Some(r)
        }
        Src::Sysfs(globs) => {
            let files = expand_globs(globs);
            let first = files.first()?;
            let text = read_trimmed(first)?;
            let bracketed = matches!(&t.kind, Kind::Choice(o) if o.is_empty()) && t.options_file.is_none();
            let (raw, discovered) = if bracketed {
                let (current, options) = parse_bracketed(&text)?;
                (current, Some(options))
            } else {
                let options = t
                    .options_file
                    .and_then(read_trimmed)
                    .map(|s| s.split_whitespace().map(String::from).collect());
                (text.clone(), options)
            };
            let mut r = shape(t, &raw, discovered)?;
            let differs = files.iter().skip(1).any(|f| read_trimmed(f).as_deref() != Some(text.as_str()));
            if differs {
                r.detail = "differs between devices — setting it makes them all the same".to_string();
            } else if files.len() > 1 {
                r.detail = format!("applies to {} devices", files.len());
            }
            r.needs_root = !files.iter().all(|f| writable(f));
            Some(r)
        }
        Src::Unit { units, user, .. } => {
            let states: Vec<&UnitState> = units
                .iter()
                .filter_map(|u| ctx.units.get(&(*user, u.to_string())))
                .filter(|s| s.load != "not-found" && !s.load.is_empty())
                .collect();
            let first = states.first()?;
            if first.file == "masked" {
                return None;
            }
            let on = states
                .iter()
                .any(|s| s.file.starts_with("enabled") || matches!(s.active.as_str(), "active" | "activating"));
            Some(Reading {
                value: if on { "on" } else { "off" }.to_string(),
                detail: format!(
                    "{}, {}",
                    if first.file.is_empty() { "no install info" } else { &first.file },
                    first.active
                ),
                needs_root: !user,
                ..Default::default()
            })
        }
        Src::Hypr(option) => {
            if !hypr::available() {
                return None;
            }
            let raw = hypr::get_option(option)?;
            shape(t, &raw, None)
        }
        Src::Gsettings(schema, key) => {
            let raw = run(Command::new("gsettings").args(["get", schema, key])).ok()?;
            // 'prefer-dark' -> prefer-dark, "uint32 24" -> 24
            let raw = raw.trim().trim_matches('\'');
            let raw = raw.rsplit(' ').next().unwrap_or(raw);
            shape(t, raw, None)
        }
        Src::DropIn(d) => {
            let raw = match d.reader {
                Reader::CatConfig(conf) => cat_config_value(conf, d.section, d.key),
                Reader::Logind(prop) => {
                    let out = run(Command::new("busctl").args([
                        "get-property",
                        "org.freedesktop.login1",
                        "/org/freedesktop/login1",
                        "org.freedesktop.login1.Manager",
                        prop,
                    ]))
                    .ok()?;
                    // `s "ignore"` for text properties, `b false` for switches.
                    match out.trim().strip_prefix("b ") {
                        Some(flag) => if flag == "true" { "yes" } else { "no" }.to_string(),
                        None => out.split('"').nth(1)?.to_string(),
                    }
                }
                Reader::NmConfig => {
                    let out = run(Command::new("NetworkManager").arg("--print-config")).ok()?;
                    ini_lookup(&out, d.section, d.key).unwrap_or_default()
                }
            };
            let mut r = shape(t, &raw, None)?;
            r.needs_root = true;
            Some(r)
        }
        Src::Custom(custom) => read_custom(t, *custom, ctx),
    }
}

// ---- validating and planning a write ----

/// Wraps a value in single quotes for the root shell. Values reach here
/// already checked against the tweak's allowed set; this is the backstop
/// that keeps anything shell-active out regardless.
pub fn shell_quote(value: &str) -> Result<String, String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || " ._:+,@/%=-[]".contains(c);
    if value.len() > 200 || !value.chars().all(allowed) {
        return Err(format!("“{value}” contains characters that aren't allowed in a setting"));
    }
    Ok(format!("'{value}'"))
}

/// Checks `value` against the tweak's kind and current options, returning
/// it in canonical form.
pub fn validate(t: &Tweak, reading: &Reading, value: &str) -> Result<String, String> {
    match &t.kind {
        Kind::Toggle => match value {
            "on" | "off" => Ok(value.to_string()),
            _ => Err("Expected on or off".to_string()),
        },
        Kind::Choice(_) => {
            // Only choices offered right now — never the "(current)" stand-in
            // for an unknown value, which may be arbitrary text.
            let offered = reading
                .options
                .iter()
                .any(|(v, label)| v == value && !label.ends_with("(current)"));
            if offered || value == reading.value {
                Ok(value.to_string())
            } else {
                Err(format!("{value} isn't one of the available choices"))
            }
        }
        Kind::Number { min, max, step, .. } => {
            let n: f64 = value.parse().map_err(|_| "Expected a number".to_string())?;
            let lo = reading.min.unwrap_or(*min);
            let hi = reading.max.unwrap_or(*max);
            if !n.is_finite() || n < lo || n > hi {
                return Err(format!("Must be between {} and {}", format_number(lo), format_number(hi)));
            }
            Ok(if step.fract() == 0.0 { format_number(n.round()) } else { format_number(n) })
        }
    }
}

fn raw_for_write(t: &Tweak, value: &str) -> String {
    match (&t.kind, value) {
        (Kind::Toggle, "on") => t.write_on.unwrap_or(t.on).to_string(),
        (Kind::Toggle, _) => t.write_off.unwrap_or(t.off).to_string(),
        _ => value.to_string(),
    }
}

pub enum Plan {
    /// A fragment for the single privileged shell script.
    Root(String),
    /// A command run as the user.
    Exec(Vec<String>),
    Hypr(&'static str, hypr::Value),
    KeepAwake(bool),
    /// A drop-in key change; merged per file before becoming a Root fragment.
    DropIn(DropIn, String),
}

fn argv(parts: &[&str]) -> Plan {
    Plan::Exec(parts.iter().map(|s| s.to_string()).collect())
}

fn plan_custom(custom: Custom, value: &str) -> Result<Plan, String> {
    Ok(match custom {
        Custom::PowerProfile => argv(&["powerprofilesctl", "set", value]),
        Custom::CpuMaxFreq => {
            let percent: u32 = value.parse().map_err(|_| "Expected a percentage".to_string())?;
            Plan::Root(format!(
                "for p in /sys/devices/system/cpu/cpufreq/policy*; do m=$(cat \"$p/cpuinfo_max_freq\"); echo $(( m / 100 * {percent} )) > \"$p/scaling_max_freq\"; done"
            ))
        }
        Custom::WifiPowerSave => {
            let iface = wifi_interface().ok_or("No Wi-Fi interface")?;
            Plan::Root(format!("iw dev {} set power_save {value}", shell_quote(&iface)?))
        }
        Custom::Radio(kind) => argv(&["rfkill", if value == "on" { "unblock" } else { "block" }, kind]),
        Custom::KeyboardBacklight => {
            let (device, _, _) = keyboard_backlight().ok_or("No keyboard backlight")?;
            argv(&["brightnessctl", "-q", "-d", &device, "set", value])
        }
        Custom::KeepAwake => Plan::KeepAwake(value == "on"),
        Custom::TimeSync => Plan::Root(format!("timedatectl set-ntp {}", if value == "on" { "true" } else { "false" })),
        Custom::LocalRtc => Plan::Root(format!(
            "timedatectl set-local-rtc {} --adjust-system-clock",
            if value == "on" { "1" } else { "0" }
        )),
        Custom::Dns => {
            let (uuid, _) = primary_connection().ok_or("No active network connection")?;
            let (v4, v6, ignore) = match DNS_PRESETS.iter().find(|p| p.0 == value) {
                Some(p) => (p.2, p.3, "yes"),
                None if value == "auto" => ("", "", "no"),
                None => return Err("Unknown DNS choice".to_string()),
            };
            argv(&[
                "nmcli", "connection", "modify", "uuid", &uuid,
                "ipv4.dns", v4, "ipv4.ignore-auto-dns", ignore,
                "ipv6.dns", v6, "ipv6.ignore-auto-dns", ignore,
            ])
        }
        Custom::AudioRate => argv(&["pw-metadata", "-n", "settings", "0", "clock.force-rate", value]),
        Custom::AudioQuantum => argv(&["pw-metadata", "-n", "settings", "0", "clock.force-quantum", value]),
        Custom::MicMute => argv(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", if value == "on" { "1" } else { "0" }]),
        Custom::Webcam => Plan::Root(if value == "on" { "modprobe uvcvideo" } else { "modprobe -r uvcvideo" }.to_string()),
        Custom::FirewallZone => Plan::Root(format!(
            "timeout 40 firewall-cmd -q --set-default-zone={}",
            shell_quote(value)?
        )),
        Custom::ConnectivityCheck => Plan::Root(format!(
            "busctl set-property org.freedesktop.NetworkManager /org/freedesktop/NetworkManager org.freedesktop.NetworkManager ConnectivityCheckEnabled b {}",
            if value == "on" { "true" } else { "false" }
        )),
    })
}

/// Turns a validated value into the action that sets it.
pub fn plan(t: &Tweak, value: &str) -> Result<Plan, String> {
    let raw = raw_for_write(t, value);
    let with_pre = |fragment: String| {
        if t.pre.is_empty() { fragment } else { format!("{} ; {fragment}", t.pre) }
    };
    Ok(match &t.src {
        Src::Sysctl(keys) => {
            let assignments: Result<Vec<String>, String> =
                keys.iter().map(|k| shell_quote(&format!("{k}={raw}"))).collect();
            Plan::Root(with_pre(format!("sysctl -q -w {}", assignments?.join(" "))))
        }
        Src::Sysfs(globs) => Plan::Root(with_pre(format!(
            "rc=0; for f in {}; do [ -e \"$f\" ] || continue; echo {} > \"$f\" || rc=1; done; [ \"$rc\" = 0 ]",
            globs.join(" "),
            shell_quote(&raw)?
        ))),
        Src::Unit { units, user, now } => {
            let verb = if value == "on" { "enable" } else { "disable" };
            if *user {
                let mut parts = vec!["systemctl", "--user", verb];
                if *now {
                    parts.push("--now");
                }
                parts.extend(units.iter());
                argv(&parts)
            } else {
                Plan::Root(format!(
                    "systemctl {verb}{} {}",
                    if *now { " --now" } else { "" },
                    units.join(" ")
                ))
            }
        }
        Src::Hypr(option) => Plan::Hypr(
            option,
            match &t.kind {
                Kind::Toggle => hypr::Value::Bool(value == "on"),
                Kind::Number { .. } => hypr::Value::Number(value.parse().map_err(|_| "Expected a number".to_string())?),
                Kind::Choice(_) => match value.parse::<f64>() {
                    Ok(n) => hypr::Value::Number(n),
                    Err(_) => hypr::Value::Text(value.to_string()),
                },
            },
        ),
        Src::Gsettings(schema, key) => {
            let raw = if matches!(t.kind, Kind::Toggle) {
                if value == "on" { "true" } else { "false" }.to_string()
            } else {
                raw
            };
            argv(&["gsettings", "set", schema, key, &raw])
        }
        Src::DropIn(d) => Plan::DropIn(*d, raw),
        Src::Custom(custom) => plan_custom(*custom, value)?,
    })
}

// ---- config drop-ins ----

/// Minimal ordered INI document: enough to own a drop-in file.
#[derive(Default, Debug, PartialEq)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                ini.sections.push((name.to_string(), Vec::new()));
            } else if let (Some((k, v)), Some(section)) = (line.split_once('='), ini.sections.last_mut()) {
                section.1.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        ini
    }

    /// Sets a key, or removes it when `value` is empty ("use the default").
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let at = match self.sections.iter().position(|(name, _)| name == section) {
            Some(at) => at,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        let entries = &mut self.sections[at].1;
        entries.retain(|(k, _)| k != key);
        if !value.is_empty() {
            entries.push((key.to_string(), value.to_string()));
        }
        self.sections.retain(|(_, entries)| !entries.is_empty());
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    pub fn render(&self) -> String {
        let mut out = String::from("# Written by Trace (Tweaks). Delete this file to return to the defaults.\n");
        for (name, entries) in &self.sections {
            out.push_str(&format!("[{name}]\n"));
            for (k, v) in entries {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        out
    }
}

fn dropin_fragment(d: &DropIn, ini: &Ini) -> Result<String, String> {
    for (_, entries) in &ini.sections {
        for (k, v) in entries {
            shell_quote(k)?;
            shell_quote(v)?;
        }
    }
    let write = if ini.is_empty() {
        format!("rm -f {}", d.file)
    } else {
        let dir = d.file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("/");
        format!(
            "mkdir -p {dir} && cat > {} <<'TRACE_EOF'\n{}TRACE_EOF\n",
            d.file,
            ini.render()
        )
    };
    Ok(if d.reload.is_empty() { write } else { format!("{write}\n{}", d.reload) })
}

// ---- keeping settings across reboots ----

const BOOT_SCRIPT: &str = "/etc/trace/tweaks.sh";
const BOOT_UNIT: &str = "/etc/systemd/system/trace-tweaks.service";

fn boot_script(kept: &[(&'static Tweak, String)]) -> Result<String, String> {
    let mut script = String::from(
        "#!/bin/sh\n# Written by Trace: the settings you chose to keep after a reboot.\n# Manage them in Trace -> Tweaks; edits made here are overwritten.\n",
    );
    for (t, value) in kept {
        if let Plan::Root(fragment) = plan(t, value)? {
            script.push_str(&format!("# {}\n( {fragment} ) || true\n", t.id));
        }
    }
    Ok(script)
}

/// Root fragment that installs (or, with nothing kept, removes) the boot service.
fn boot_install_fragment(kept: &[(&'static Tweak, String)]) -> Result<String, String> {
    let boot: Vec<(&'static Tweak, String)> = kept
        .iter()
        .filter(|(t, _)| t.persistence() == Persist::Boot)
        .cloned()
        .collect();
    if boot.is_empty() {
        return Ok(format!(
            "systemctl disable trace-tweaks.service 2>/dev/null; rm -f {BOOT_UNIT} {BOOT_SCRIPT}; rmdir /etc/trace 2>/dev/null; systemctl daemon-reload"
        ));
    }
    // The script runs a little after boot rather than during it: devices
    // (battery, Wi-Fi) and the power-profile daemon have settled by then,
    // and it adds nothing to boot time.
    Ok(format!(
        "mkdir -p /etc/trace && cat > {BOOT_SCRIPT} <<'TRACE_BOOT_EOF'\n{}TRACE_BOOT_EOF\nchmod 755 {BOOT_SCRIPT} && cat > {BOOT_UNIT} <<'TRACE_BOOT_EOF'\n[Unit]\nDescription=Apply the settings kept by Trace\nAfter=basic.target power-profiles-daemon.service\n\n[Service]\nType=simple\nExecStart=/bin/sh -c 'sleep 15; exec {BOOT_SCRIPT}'\n\n[Install]\nWantedBy=multi-user.target\nTRACE_BOOT_EOF\nsystemctl daemon-reload && systemctl enable trace-tweaks.service",
        boot_script(&boot)?
    ))
}

// ---- trusted helper: root-level changes without a prompt each time ----
//
// Opt-in. A root-owned script that accepts nothing but "<setting> <value>"
// pairs from a list baked in when it is installed, plus a polkit rule that
// lets the logged-in user run exactly that script. Trace can then change
// those settings by itself (thermal rules, schedules) — and every call is
// written to a log the user can read.

pub const HELPER_SCRIPT: &str = "/etc/trace/helper.sh";
const HELPER_RULE: &str = "/etc/polkit-1/rules.d/49-trace-helper.rules";
pub const HELPER_LOG: &str = "/var/log/trace-helper.log";

/// Whether a setting may go through the helper at all. Deliberately narrow:
/// performance, power and memory tunables that are harmless to flip — no
/// services, no config files, nothing marked expert, nothing security-related.
fn helper_eligible(t: &Tweak) -> bool {
    if t.risk == Risk::Expert || t.category == "Privacy & Security" || t.category == "Developer" {
        return false;
    }
    match &t.src {
        Src::Sysctl(_) | Src::Sysfs(_) => true,
        Src::Custom(c) => matches!(c, Custom::CpuMaxFreq | Custom::WifiPowerSave),
        _ => false,
    }
}

/// Every value of a setting the helper will accept.
fn helper_values(t: &Tweak, reading: &Reading) -> Vec<String> {
    match &t.kind {
        Kind::Toggle => vec!["on".to_string(), "off".to_string()],
        Kind::Choice(_) => reading
            .options
            .iter()
            .filter(|(_, label)| !label.ends_with("(current)"))
            .map(|(v, _)| v.clone())
            .collect(),
        Kind::Number { min, max, step, .. } => {
            let (lo, hi) = (reading.min.unwrap_or(*min), reading.max.unwrap_or(*max));
            let mut values = Vec::new();
            let mut n = lo;
            while n <= hi + 1e-9 && values.len() < 400 {
                values.push(format_number(n));
                n += step;
            }
            values
        }
    }
}

/// The helper script for this machine: one `case` arm per allowed
/// (setting, value), each running exactly the fragment a manual change runs.
pub fn helper_script() -> String {
    let ctx = ReadCtx::new();
    let mut arms = String::new();
    for t in catalog().iter().filter(|t| helper_eligible(t)) {
        let Some(reading) = read(t, &ctx) else {
            continue;
        };
        for value in helper_values(t, &reading) {
            if shell_quote(&value).is_err() {
                continue;
            }
            if let Ok(Plan::Root(fragment)) = plan(t, &value) {
                arms.push_str(&format!("  '{} {}') run() {{ {fragment}\n  }} ;;\n", t.id, value));
            }
        }
    }
    format!(
        "#!/bin/sh\n# Trace's trusted helper. Written by Trace; edits are overwritten.\n# It accepts only the settings and values listed below, and logs every call.\n# Remove it from Trace (Tweaks) or by deleting this file and\n# {HELPER_RULE}.\nPATH=/usr/sbin:/usr/bin:/sbin:/bin\ncheck=0\n[ \"$1\" = \"--check\" ] && {{ check=1; shift; }}\n[ $# -eq 2 ] || {{ echo 'usage: helper.sh [--check] <setting> <value>' >&2; exit 2; }}\ncase \"$1 $2\" in\n{arms}  *) echo \"not an allowed setting or value: $1 $2\" >&2; exit 3 ;;\nesac\n[ \"$check\" = 1 ] && {{ echo allowed; exit 0; }}\nout=$(run 2>&1); rc=$?\nprintf '%s uid=%s %s=%s rc=%s %s\\n' \"$(date '+%Y-%m-%d %H:%M:%S')\" \"${{PKEXEC_UID:-?}}\" \"$1\" \"$2\" \"$rc\" \"$(printf '%s' \"$out\" | tr '\\n' ' ' | cut -c1-160)\" >> {HELPER_LOG}\n[ \"$rc\" = 0 ] || printf '%s\\n' \"$out\" >&2\nexit $rc\n"
    )
}

#[derive(Serialize)]
pub struct HelperStatus {
    pub installed: bool,
    /// False when settings were added or changed since it was installed.
    pub current: bool,
    /// How many (setting, value) pairs it accepts.
    pub allowed: usize,
    pub log: Vec<String>,
}

pub fn helper_status() -> HelperStatus {
    let on_disk = std::fs::read_to_string(HELPER_SCRIPT).ok();
    let expected = helper_script();
    let log = std::fs::read_to_string(HELPER_LOG)
        .map(|t| t.lines().rev().take(60).map(String::from).collect())
        .unwrap_or_default();
    HelperStatus {
        installed: on_disk.is_some() && std::path::Path::new(HELPER_RULE).exists(),
        current: on_disk.as_deref() == Some(expected.as_str()),
        allowed: expected.matches(") run() {").count(),
        log,
    }
}

/// Installs or removes the helper (one password prompt either way).
pub fn set_helper(install: bool) -> Result<String, String> {
    if !install {
        run_privileged(&format!("rm -f {HELPER_SCRIPT} {HELPER_RULE}; rmdir /etc/trace 2>/dev/null; true"))?;
        return Ok("Trusted helper removed — root-level changes ask for your password again".to_string());
    }
    let script = helper_script();
    if script.contains("TRACE_HELPER_EOF") {
        return Err("internal: unexpected content in the helper".to_string());
    }
    let rule = format!(
        "// Written by Trace: lets the logged-in desktop user run Trace's helper,\n// and only that, without a password prompt.\npolkit.addRule(function(action, subject) {{\n    if (action.id == \"org.freedesktop.policykit.exec\" &&\n        action.lookup(\"program\") == \"{HELPER_SCRIPT}\" &&\n        subject.local && subject.active) {{\n        return polkit.Result.YES;\n    }}\n}});\n"
    );
    run_privileged(&format!(
        "mkdir -p /etc/trace && cat > {HELPER_SCRIPT} <<'TRACE_HELPER_EOF'\n{script}TRACE_HELPER_EOF\nchown root:root {HELPER_SCRIPT} && chmod 755 {HELPER_SCRIPT} && sh -n {HELPER_SCRIPT} && cat > {HELPER_RULE} <<'TRACE_HELPER_EOF'\n{rule}TRACE_HELPER_EOF\nchmod 644 {HELPER_RULE} && touch {HELPER_LOG} && chmod 644 {HELPER_LOG}"
    ))?;
    Ok("Trusted helper installed — the listed settings can now change without a password, and every use is logged".to_string())
}

/// Runs one change through the helper. None when the helper isn't installed
/// or doesn't cover this setting; Some(result) when it was used.
fn via_helper(t: &Tweak, value: &str) -> Option<Result<(), String>> {
    if !helper_eligible(t) || !std::path::Path::new(HELPER_SCRIPT).exists() || !std::path::Path::new(HELPER_RULE).exists() {
        return None;
    }
    // Only use it if it knows this exact pair — an out-of-date helper is
    // skipped rather than tripping its own refusal.
    let known = std::fs::read_to_string(HELPER_SCRIPT).is_ok_and(|s| s.contains(&format!("  '{} {}') run()", t.id, value)));
    if !known {
        return None;
    }
    Some(run(Command::new("pkexec").args([HELPER_SCRIPT, t.id, value])).map(|_| ()))
}

// ---- storage ----

pub fn init(db: &Db) {
    let conn = db.conn.lock().unwrap();
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tweak_baseline (
            tweak_id TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            ts INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tweak_changes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            batch INTEGER NOT NULL,
            tweak_id TEXT NOT NULL,
            old_value TEXT NOT NULL,
            new_value TEXT NOT NULL,
            ok INTEGER NOT NULL,
            source TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tweak_changes_ts ON tweak_changes(ts);
        CREATE TABLE IF NOT EXISTS tweak_persist (
            tweak_id TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )
    .expect("failed to init tweak schema");
}

fn kept_map(db: &Db) -> HashMap<String, String> {
    let conn = db.conn.lock().unwrap();
    let Ok(mut stmt) = conn.prepare("SELECT tweak_id, value FROM tweak_persist") else {
        return HashMap::new();
    };
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn kept_tweaks(map: &HashMap<String, String>) -> Vec<(&'static Tweak, String)> {
    let mut kept: Vec<(&'static Tweak, String)> = catalog()
        .iter()
        .filter_map(|t| map.get(t.id).map(|v| (t, v.clone())))
        .collect();
    kept.sort_by_key(|(t, _)| t.id);
    kept
}

fn baseline_map(db: &Db) -> HashMap<String, String> {
    let conn = db.conn.lock().unwrap();
    let Ok(mut stmt) = conn.prepare("SELECT tweak_id, value FROM tweak_baseline") else {
        return HashMap::new();
    };
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

// ---- listing ----

#[derive(Serialize, Clone)]
pub struct OptionDto {
    pub value: String,
    pub label: String,
}

#[derive(Serialize, Clone)]
pub struct TweakState {
    pub id: String,
    pub category: String,
    pub title: String,
    pub help: String,
    /// "toggle" | "choice" | "number"
    pub kind: String,
    pub options: Vec<OptionDto>,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: String,
    pub value: String,
    /// The value when Trace first saw this setting.
    pub original: String,
    pub changed: bool,
    pub needs_root: bool,
    pub risk: Risk,
    pub persistence: Persist,
    pub kept: bool,
    pub effect: String,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct ChangeRow {
    pub ts: i64,
    pub batch: i64,
    pub tweak_id: String,
    pub title: String,
    pub old_value: String,
    pub new_value: String,
    pub ok: bool,
    pub source: String,
}

/// Reads every tweak this machine has. The first value ever seen for each
/// is remembered as its "original", so changes can be spotted and reset.
pub fn list(db: &Db) -> Vec<TweakState> {
    let ctx = ReadCtx::new();
    let mut baseline = baseline_map(db);
    let kept = kept_map(db);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut new_baselines = Vec::new();

    let states: Vec<TweakState> = catalog()
        .iter()
        .filter_map(|t| {
            let r = read(t, &ctx)?;
            let original = baseline.entry(t.id.to_string()).or_insert_with(|| {
                new_baselines.push((t.id, r.value.clone()));
                r.value.clone()
            });
            let (min, max, step, unit) = match &t.kind {
                Kind::Number { min, max, step, unit } => {
                    (r.min.unwrap_or(*min), r.max.unwrap_or(*max), *step, unit.to_string())
                }
                _ => (0.0, 0.0, 0.0, String::new()),
            };
            Some(TweakState {
                id: t.id.to_string(),
                category: t.category.to_string(),
                title: t.title.to_string(),
                help: t.help.to_string(),
                kind: match t.kind {
                    Kind::Toggle => "toggle",
                    Kind::Choice(_) => "choice",
                    Kind::Number { .. } => "number",
                }
                .to_string(),
                options: r.options.iter().map(|(v, l)| OptionDto { value: v.clone(), label: l.clone() }).collect(),
                min,
                max,
                step,
                unit,
                changed: *original != r.value,
                original: original.clone(),
                value: r.value,
                needs_root: r.needs_root,
                risk: t.risk,
                persistence: t.persistence(),
                kept: kept.contains_key(t.id),
                effect: t.effect.to_string(),
                detail: r.detail,
            })
        })
        .collect();

    if !new_baselines.is_empty() {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction();
        if let Ok(tx) = tx {
            for (id, value) in new_baselines {
                tx.execute(
                    "INSERT OR IGNORE INTO tweak_baseline (tweak_id, value, ts) VALUES (?1, ?2, ?3)",
                    params![id, value, now],
                )
                .ok();
            }
            tx.commit().ok();
        }
    }
    states
}

/// Current state of just the named settings (for pinned switches).
pub fn list_some(db: &Db, ids: &[String]) -> Vec<TweakState> {
    let all = list(db);
    ids.iter().filter_map(|id| all.iter().find(|t| t.id == *id).cloned()).collect()
}

pub fn recent_changes(db: &Db, limit: i64) -> Vec<ChangeRow> {
    let conn = db.conn.lock().unwrap();
    let Ok(mut stmt) = conn.prepare(
        "SELECT ts, batch, tweak_id, old_value, new_value, ok, source FROM tweak_changes ORDER BY id DESC LIMIT ?1",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![limit], |r| {
        let id: String = r.get(2)?;
        Ok(ChangeRow {
            ts: r.get(0)?,
            batch: r.get(1)?,
            title: find(&id).map(|t| t.title.to_string()).unwrap_or_else(|| id.clone()),
            tweak_id: id,
            old_value: r.get(3)?,
            new_value: r.get(4)?,
            ok: r.get::<_, i32>(5)? != 0,
            source: r.get(6)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Whether the boot service on disk matches what is kept.
pub fn boot_service_state(db: &Db) -> &'static str {
    let kept = kept_tweaks(&kept_map(db));
    let boot: Vec<_> = kept.into_iter().filter(|(t, _)| t.persistence() == Persist::Boot).collect();
    match (std::fs::read_to_string(BOOT_SCRIPT), boot.is_empty()) {
        (Err(_), true) => "none",
        (Err(_), false) => "missing",
        (Ok(_), true) => "stale",
        (Ok(text), false) => {
            if boot_script(&boot).is_ok_and(|expected| expected == text) { "installed" } else { "stale" }
        }
    }
}

// ---- applying ----

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Change {
    pub id: String,
    pub value: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ChangeResult {
    pub id: String,
    pub title: String,
    pub ok: bool,
    pub message: String,
    pub old_value: String,
    pub new_value: String,
}

/// Runs root fragments in one privileged shell. Each reports separately, so
/// one failing setting doesn't hide whether the others worked.
fn run_root_batch(fragments: &[(usize, String)], tail: Option<&str>) -> Result<HashMap<usize, Result<(), String>>, String> {
    let mut script = String::new();
    for (index, fragment) in fragments {
        script.push_str(&format!(
            "out=$( {{ {fragment}\n}} 2>&1 ) && echo \"TRACE_OK {index}\" || echo \"TRACE_FAIL {index} $(printf '%s' \"$out\" | tr '\\n' ' ' | cut -c1-200)\"\n"
        ));
    }
    if let Some(tail) = tail {
        script.push_str(&format!("{{ {tail}\n}} >/dev/null 2>&1 && echo TRACE_TAIL_OK || echo TRACE_TAIL_FAIL\n"));
    }
    let out = run_privileged(&script)?;
    let mut results = HashMap::new();
    for line in out.lines() {
        if let Some(index) = line.strip_prefix("TRACE_OK ").and_then(|i| i.trim().parse().ok()) {
            results.insert(index, Ok(()));
        } else if let Some(rest) = line.strip_prefix("TRACE_FAIL ") {
            let (index, message) = rest.split_once(' ').unwrap_or((rest, ""));
            if let Ok(index) = index.parse() {
                results.insert(index, Err(message.trim().to_string()));
            }
        }
    }
    Ok(results)
}

fn execute_user(plan: &Plan) -> Result<(), String> {
    match plan {
        Plan::Exec(parts) => run(Command::new(&parts[0]).args(&parts[1..])).map(|_| ()),
        Plan::Hypr(option, value) => hypr::set_option(option, value),
        Plan::KeepAwake(on) => {
            let mut guard = KEEP_AWAKE.lock().unwrap();
            if let Some(mut child) = guard.take() {
                // SIGTERM, so systemd-inhibit can take its `tail` down with it.
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                child.wait().ok();
            }
            if *on {
                // The inhibitor holds for as long as this `tail` runs, and
                // `tail --pid` exits by itself when Trace does — so the
                // machine can never be left unable to sleep by a crash.
                let child = Command::new("systemd-inhibit")
                    .args([
                        "--what=idle:sleep",
                        "--who=Trace",
                        "--why=Keep awake is switched on in Trace",
                        "tail",
                        &format!("--pid={}", std::process::id()),
                        "-f",
                        "/dev/null",
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                *guard = Some(child);
            }
            Ok(())
        }
        Plan::Root(_) | Plan::DropIn(..) => Err("internal: privileged plan run as user".to_string()),
    }
}

/// Applies a batch of changes. `keep` additionally marks each changed
/// setting to be kept (true) or no longer kept (false) across reboots.
pub fn apply(db: &Db, changes: &[Change], source: &str, keep: Option<bool>) -> Vec<ChangeResult> {
    apply_with(db, changes, source, keep, true)
}

/// As `apply`, but with `allow_root` false nothing that needs a password is
/// attempted — used when Trace acts by itself (schedules, triggers), where
/// a password prompt appearing out of nowhere would be wrong.
pub fn apply_with(db: &Db, changes: &[Change], source: &str, keep: Option<bool>, allow_root: bool) -> Vec<ChangeResult> {
    let ctx = ReadCtx::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut kept = kept_map(db);
    let kept_before = kept.clone();

    struct Item {
        tweak: &'static Tweak,
        old: String,
        new: String,
        outcome: Option<Result<(), String>>,
    }
    let mut items: Vec<Item> = Vec::new();
    let mut failures: Vec<ChangeResult> = Vec::new();
    let mut root: Vec<(usize, String)> = Vec::new();
    let mut dropins: HashMap<&'static str, (DropIn, Ini, Vec<usize>)> = HashMap::new();

    for change in changes {
        let fail = |message: String| ChangeResult {
            id: change.id.clone(),
            title: find(&change.id).map(|t| t.title.to_string()).unwrap_or_else(|| change.id.clone()),
            ok: false,
            message,
            old_value: String::new(),
            new_value: change.value.clone(),
        };
        let Some(tweak) = find(&change.id) else {
            failures.push(fail("Unknown setting".to_string()));
            continue;
        };
        let Some(reading) = read(tweak, &ctx) else {
            failures.push(fail("This machine doesn't have that setting".to_string()));
            continue;
        };
        let value = match validate(tweak, &reading, &change.value) {
            Ok(v) => v,
            Err(e) => {
                failures.push(fail(e));
                continue;
            }
        };
        let planned = match plan(tweak, &value) {
            Ok(p) => p,
            Err(e) => {
                failures.push(fail(e));
                continue;
            }
        };
        let index = items.len();
        let mut item = Item { tweak, old: reading.value.clone(), new: value.clone(), outcome: None };
        // With the trusted helper installed, settings it covers never need a prompt.
        let helped = if matches!(planned, Plan::Root(_)) { via_helper(tweak, &value) } else { None };
        match planned {
            Plan::Root(_) if helped.is_some() => item.outcome = helped,
            Plan::Root(_) | Plan::DropIn(..) if !allow_root => {
                item.outcome = Some(Err(
                    "Needs your password, so it was skipped in this automatic run (the trusted helper under Tweaks removes that limit)".to_string(),
                ));
            }
            Plan::Root(fragment) => root.push((index, fragment)),
            Plan::DropIn(d, raw) => {
                let entry = dropins.entry(d.file).or_insert_with(|| {
                    let existing = std::fs::read_to_string(d.file).unwrap_or_default();
                    (d, Ini::parse(&existing), Vec::new())
                });
                entry.1.set(d.section, d.key, &raw);
                entry.2.push(index);
            }
            user_plan => item.outcome = Some(execute_user(&user_plan)),
        }
        match keep {
            Some(true) if tweak.persistence() != Persist::Never && tweak.persistence() != Persist::Inherent => {
                kept.insert(tweak.id.to_string(), value.clone());
            }
            Some(false) => {
                kept.remove(tweak.id);
            }
            // A setting that is already kept stays kept at its new value.
            _ => {
                if let Some(existing) = kept.get_mut(tweak.id) {
                    *existing = value.clone();
                }
            }
        }
        items.push(item);
    }

    // One fragment per drop-in file, carrying every change made to it.
    let mut shared: HashMap<usize, Vec<usize>> = HashMap::new();
    for (_, (d, ini, indices)) in dropins {
        match dropin_fragment(&d, &ini) {
            Ok(fragment) => {
                root.push((indices[0], fragment));
                shared.insert(indices[0], indices);
            }
            Err(e) => {
                for index in indices {
                    items[index].outcome = Some(Err(e.clone()));
                }
            }
        }
    }

    // The boot service only needs rewriting when what it applies changed.
    let boot_of = |map: &HashMap<String, String>| -> Vec<(String, String)> {
        let mut boot: Vec<(String, String)> = kept_tweaks(map)
            .into_iter()
            .filter(|(t, _)| t.persistence() == Persist::Boot)
            .map(|(t, v)| (t.id.to_string(), v))
            .collect();
        boot.sort();
        boot
    };
    let tail = if boot_of(&kept) != boot_of(&kept_before) {
        boot_install_fragment(&kept_tweaks(&kept)).ok()
    } else {
        None
    };

    let mut keep_saved = true;
    if !root.is_empty() || tail.is_some() {
        match run_root_batch(&root, tail.as_deref()) {
            Ok(results) => {
                for (index, _) in &root {
                    let outcome = results
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| Err("No result reported".to_string()));
                    for target in shared.get(index).cloned().unwrap_or_else(|| vec![*index]) {
                        items[target].outcome = Some(outcome.clone());
                    }
                }
            }
            Err(e) => {
                keep_saved = false;
                for (index, _) in &root {
                    for target in shared.get(index).cloned().unwrap_or_else(|| vec![*index]) {
                        items[target].outcome = Some(Err(e.clone()));
                    }
                }
            }
        }
    }

    // Read everything back: a write the kernel accepted but ignored is a failure.
    let verify = ReadCtx::new();
    let mut results = failures;
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().ok();
        for item in &items {
            let actual = read(item.tweak, &verify).map(|r| r.value);
            let (ok, message) = match (&item.outcome, &actual) {
                (Some(Err(e)), _) => (false, e.clone()),
                (_, Some(actual)) if *actual == item.new => (true, "Applied".to_string()),
                // Settings that only take hold later can't be confirmed now.
                (_, _) if !item.tweak.effect.is_empty() => (true, format!("Saved — takes effect {}", item.tweak.effect)),
                (_, Some(actual)) => (
                    false,
                    format!("The system accepted the change but still reports {actual}"),
                ),
                (_, None) => (false, "Couldn't read the setting back".to_string()),
            };
            if let Some(tx) = &tx {
                tx.execute(
                    "INSERT INTO tweak_changes (ts, batch, tweak_id, old_value, new_value, ok, source)
                     VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6)",
                    params![now, item.tweak.id, item.old, item.new, ok as i32, source],
                )
                .ok();
            }
            results.push(ChangeResult {
                id: item.tweak.id.to_string(),
                title: item.tweak.title.to_string(),
                ok,
                message,
                old_value: item.old.clone(),
                new_value: item.new.clone(),
            });
        }
        if keep_saved {
            if let Some(tx) = &tx {
                tx.execute("DELETE FROM tweak_persist", []).ok();
                for (id, value) in &kept {
                    tx.execute("INSERT INTO tweak_persist (tweak_id, value) VALUES (?1, ?2)", params![id, value])
                        .ok();
                }
            }
        }
        if let Some(tx) = tx {
            tx.commit().ok();
        }
    }
    results
}

/// Marks a setting as kept (at its current value) or not kept, without changing it.
pub fn set_kept(db: &Db, id: &str, keep: bool) -> Result<String, String> {
    let tweak = find(id).ok_or("Unknown setting")?;
    let persistence = tweak.persistence();
    if matches!(persistence, Persist::Never | Persist::Inherent) {
        return Err("This setting can't be kept that way".to_string());
    }
    let reading = read(tweak, &ReadCtx::new()).ok_or("This machine doesn't have that setting")?;
    let mut kept = kept_map(db);
    if keep {
        kept.insert(id.to_string(), reading.value.clone());
    } else {
        kept.remove(id);
    }
    if persistence == Persist::Boot {
        let fragment = boot_install_fragment(&kept_tweaks(&kept))?;
        run_privileged(&fragment)?;
    }
    let conn = db.conn.lock().unwrap();
    if keep {
        conn.execute(
            "INSERT OR REPLACE INTO tweak_persist (tweak_id, value) VALUES (?1, ?2)",
            params![id, reading.value],
        )
    } else {
        conn.execute("DELETE FROM tweak_persist WHERE tweak_id = ?1", params![id])
    }
    .map_err(|e| e.to_string())?;
    Ok(if keep {
        match persistence {
            Persist::Boot => "Will be re-applied shortly after every boot".to_string(),
            _ => "Will be re-applied whenever Trace starts".to_string(),
        }
    } else {
        "No longer kept — it returns to the default after a restart".to_string()
    })
}

/// The changes made in one batch, reversed.
pub fn undo_changes(db: &Db, batch: i64) -> Vec<Change> {
    let conn = db.conn.lock().unwrap();
    let Ok(mut stmt) = conn.prepare(
        "SELECT tweak_id, old_value FROM tweak_changes WHERE batch = ?1 AND ok = 1 ORDER BY id DESC",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![batch], |r| Ok(Change { id: r.get(0)?, value: r.get(1)? }))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Releases "keep awake" (called when Trace exits).
pub fn release_keep_awake() {
    execute_user(&Plan::KeepAwake(false)).ok();
}

/// Re-applies kept session-level settings (Hyprland options and the like).
/// Called once when Trace starts; needs no password.
pub fn reapply_session(db: &Db) {
    let ctx = ReadCtx::new();
    for (tweak, value) in kept_tweaks(&kept_map(db)) {
        if tweak.persistence() != Persist::Session {
            continue;
        }
        let Some(reading) = read(tweak, &ctx) else {
            continue;
        };
        if reading.value == value {
            continue;
        }
        if let Ok(planned) = validate(tweak, &reading, &value).and_then(|v| plan(tweak, &v)) {
            if !matches!(planned, Plan::Root(_) | Plan::DropIn(..)) {
                execute_user(&planned).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bracketed_choices() {
        let (current, options) = parse_bracketed("[none] mq-deadline kyber bfq ").unwrap();
        assert_eq!(current, "none");
        assert_eq!(options, ["none", "mq-deadline", "kyber", "bfq"]);
        assert!(parse_bracketed("no current marked").is_none());
    }

    #[test]
    fn shell_quote_rejects_anything_shell_active() {
        assert_eq!(shell_quote("mq-deadline").unwrap(), "'mq-deadline'");
        assert_eq!(shell_quote("vm.swappiness=100").unwrap(), "'vm.swappiness=100'");
        for bad in ["a'b", "$(reboot)", "a;b", "a`b`", "a\nb", "a|b", "a&b", "a>b", "a\\b", "a\"b"] {
            assert!(shell_quote(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ini_sets_replaces_and_removes_keys() {
        let mut ini = Ini::parse("# comment\n[Login]\nHandleLidSwitch=ignore\nHandlePowerKey=poweroff\n");
        ini.set("Login", "HandleLidSwitch", "suspend");
        assert_eq!(
            ini.render().lines().skip(1).collect::<Vec<_>>(),
            ["[Login]", "HandlePowerKey=poweroff", "HandleLidSwitch=suspend"]
        );
        ini.set("Login", "HandleLidSwitch", "");
        ini.set("Login", "HandlePowerKey", "");
        assert!(ini.is_empty());
    }

    #[test]
    fn ini_lookup_takes_the_last_uncommented_value() {
        let text = "# /etc/a.conf\n[Journal]\n#SystemMaxUse=\nSystemMaxUse=1G\n\n# /etc/b.conf\n[Journal]\nSystemMaxUse=500M\n[Other]\nSystemMaxUse=9\n";
        assert_eq!(ini_lookup(text, "Journal", "SystemMaxUse").as_deref(), Some("500M"));
        assert_eq!(ini_lookup(text, "Journal", "Storage"), None);
    }

    #[test]
    fn validates_by_kind() {
        let number = Tweak::new("t", "c", "t", "h", Src::Sysctl(&["vm.swappiness"]), Kind::Number { min: 0.0, max: 200.0, step: 5.0, unit: "" });
        let r = Reading::default();
        assert_eq!(validate(&number, &r, "60").unwrap(), "60");
        assert_eq!(validate(&number, &r, "60.4").unwrap(), "60");
        assert!(validate(&number, &r, "201").is_err());
        assert!(validate(&number, &r, "60; reboot").is_err());

        let toggle = Tweak::new("t", "c", "t", "h", Src::Sysctl(&["a.b"]), Kind::Toggle);
        assert!(validate(&toggle, &r, "on").is_ok());
        assert!(validate(&toggle, &r, "1").is_err());

        let choice = Tweak::new("t", "c", "t", "h", Src::Sysfs(&["/sys/x"]), Kind::Choice(vec![]));
        let r = Reading {
            value: "weird value".to_string(),
            options: vec![
                ("none".to_string(), "none".to_string()),
                ("weird value".to_string(), "weird value (current)".to_string()),
            ],
            ..Default::default()
        };
        assert!(validate(&choice, &r, "none").is_ok());
        assert!(validate(&choice, &r, "bfq").is_err());
        assert!(validate(&choice, &r, "weird value").is_ok(), "the current value is always allowed");
    }

    #[test]
    fn every_catalog_entry_is_well_formed_and_plans_safely() {
        let path_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./*:@-".contains(c));
        let mut ids = std::collections::HashSet::new();
        for t in catalog() {
            assert!(ids.insert(t.id), "duplicate id {}", t.id);
            assert!(!t.title.is_empty() && t.help.len() > 20, "{} needs a real description", t.id);
            assert!(crate::tweak_catalog::CATEGORIES.iter().any(|(name, _)| *name == t.category), "{} category", t.id);
            match &t.src {
                Src::Sysctl(keys) => assert!(keys.iter().all(|k| path_ok(k) && !k.contains('/')), "{}", t.id),
                Src::Sysfs(globs) => assert!(globs.iter().all(|g| g.starts_with("/sys/") && path_ok(g)), "{}", t.id),
                Src::Unit { units, .. } => assert!(units.iter().all(|u| path_ok(u) && u.contains('.')), "{}", t.id),
                Src::DropIn(d) => {
                    assert!(d.file.starts_with("/etc/") && path_ok(d.file), "{}", t.id);
                    assert!(matches!(t.kind, Kind::Choice(_)), "{} drop-ins are choices", t.id);
                }
                _ => {}
            }
            // Every fixed value must survive quoting, and every plan must build.
            let samples: Vec<String> = match &t.kind {
                Kind::Toggle => vec!["on".to_string(), "off".to_string()],
                Kind::Choice(options) => options.iter().map(|(v, _)| v.to_string()).collect(),
                Kind::Number { min, max, .. } => vec![format_number(*min), format_number(*max)],
            };
            for sample in samples {
                shell_quote(&raw_for_write(t, &sample)).unwrap_or_else(|e| panic!("{}: {e}", t.id));
                match plan(t, &sample) {
                    Ok(Plan::Root(fragment)) => {
                        assert!(!fragment.contains('`') && !fragment.contains("$(reboot"), "{}", t.id)
                    }
                    Ok(_) => {}
                    // Custom plans may need hardware this test machine lacks.
                    Err(_) if matches!(t.src, Src::Custom(_)) => {}
                    Err(e) => panic!("{}: {e}", t.id),
                }
            }
        }
        assert!(catalog().len() > 150, "catalog has {} entries", catalog().len());
    }

    #[test]
    fn boot_script_only_contains_boot_level_settings() {
        let swappiness = find("vm.swappiness").unwrap();
        let script = boot_script(&[(swappiness, "100".to_string())]).unwrap();
        assert!(script.contains("sysctl -q -w 'vm.swappiness=100'"));
        let install = boot_install_fragment(&[(swappiness, "100".to_string())]).unwrap();
        assert!(install.contains("trace-tweaks.service") && install.contains("systemctl enable"));
        let remove = boot_install_fragment(&[]).unwrap();
        assert!(remove.contains("systemctl disable") && remove.contains("rm -f"));
    }

    /// Writes a handful of password-free settings back to the values they
    /// already have, through the full apply path. Nothing visibly changes,
    /// but it proves each kind of write (Hyprland, GTK, audio, backlight)
    /// really works on this machine. Run with `--ignored`.
    #[test]
    #[ignore]
    fn applies_user_level_settings_idempotently() {
        let db = Db { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        init(&db);
        let ctx = ReadCtx::new();
        // Every Hyprland and GTK setting, plus one of each other password-free kind.
        let changes: Vec<Change> = catalog()
            .iter()
            .filter(|t| {
                matches!(t.src, Src::Hypr(_) | Src::Gsettings(..))
                    || ["audio.quantum", "audio.rate", "audio.mic_mute", "laptop.kbd_backlight", "power.profile"].contains(&t.id)
            })
            .filter_map(|t| Some(Change { id: t.id.to_string(), value: read(t, &ctx)?.value }))
            .collect();
        assert!(changes.len() >= 3, "expected some user-level settings on this machine");
        for result in apply(&db, &changes, "test", None) {
            assert!(result.ok, "{}: {} ({} -> {})", result.id, result.message, result.old_value, result.new_value);
            assert_eq!(result.old_value, result.new_value);
        }
        assert_eq!(recent_changes(&db, 100).len(), changes.len());
    }

    /// Keep-awake holds a real sleep inhibitor and releases it again.
    #[test]
    #[ignore]
    fn keep_awake_takes_and_releases_an_inhibitor() {
        let listed = || run_lossy(Command::new("systemd-inhibit").arg("--list")).contains("Keep awake is switched on in Trace");
        execute_user(&Plan::KeepAwake(true)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert!(keep_awake_active() && listed());
        release_keep_awake();
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert!(!keep_awake_active() && !listed());
    }

    #[test]
    fn helper_only_accepts_listed_settings_and_values() {
        let script = helper_script();
        let path = std::env::temp_dir().join(format!("trace-helper-test-{}.sh", std::process::id()));
        std::fs::write(&path, &script).unwrap();
        let sh = |args: &[&str]| Command::new("sh").arg(&path).args(args).output().unwrap();
        assert!(Command::new("sh").arg("-n").arg(&path).status().unwrap().success(), "script must parse");
        // `--check` proves the lookup without changing anything.
        if script.contains("'vm.swappiness 100') run()") {
            assert!(sh(&["--check", "vm.swappiness", "100"]).status.success());
        }
        for bad in [
            vec!["--check", "vm.swappiness", "100; reboot"],
            vec!["--check", "vm.swappiness", "9999"],
            vec!["--check", "kernel.yama.ptrace_scope", "0"],
            vec!["--check", "svc.ssh", "on"],
            vec!["--check", "nonsense", "1"],
            vec!["--check", "vm.swappiness"],
        ] {
            assert!(!sh(&bad).status.success(), "{bad:?} must be refused");
        }
        // Nothing expert, security-related or service-level is ever listed.
        for forbidden in ["kernel.yama", "svc.", "vm.overcommit_memory", "journald.", "logind.", "kernel.sysrq"] {
            assert!(!script.contains(&format!("  '{forbidden}")), "{forbidden} must not be in the helper");
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn root_batch_script_is_valid_shell() {
        // Run the batch wrapper unprivileged with harmless fragments to make
        // sure the generated script parses and reports per fragment.
        let fragments = [(0usize, "true".to_string()), (1, "echo nope >&2; false".to_string())];
        let mut script = String::new();
        for (index, fragment) in &fragments {
            script.push_str(&format!(
                "out=$( {{ {fragment}\n}} 2>&1 ) && echo \"TRACE_OK {index}\" || echo \"TRACE_FAIL {index} $(printf '%s' \"$out\" | tr '\\n' ' ' | cut -c1-200)\"\n"
            ));
        }
        let out = run(Command::new("sh").args(["-c", &script])).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), ["TRACE_OK 0", "TRACE_FAIL 1 nope"]);
    }
}

#[cfg(test)]
mod timing {
    /// Reading the whole catalogue has to stay quick: it runs every time the
    /// Tweaks page opens.
    #[test]
    fn listing_every_setting_is_fast() {
        let db = crate::db::Db { conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        super::init(&db);
        let started = std::time::Instant::now();
        let states = super::list(&db);
        let elapsed = started.elapsed();
        eprintln!("listed {} settings in {:?}", states.len(), elapsed);
        assert!(elapsed.as_secs() < 6, "took {elapsed:?}");
    }
}
