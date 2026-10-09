//! One search box over everything Trace knows about.

use crate::db::Db;
use rusqlite::params;
use serde::Serialize;
use sysinfo::System;

#[derive(Serialize, Clone)]
pub struct Hit {
    /// "page" | "setting" | "process" | "service" | "log"
    pub kind: String,
    pub title: String,
    pub detail: String,
    /// Where selecting the hit should go.
    pub page: String,
    pub section: Option<String>,
    /// Text to pre-fill in that page's own search or filter.
    pub query: Option<String>,
}

const PAGES: &[(&str, &str, Option<&str>, &str)] = &[
    ("Overview", "live", None, "live cpu ram swap temperature gpu disks processes"),
    ("Memory", "memory", None, "ram swap zram apps memory usage leak growth"),
    ("Activity", "activity", None, "disk io network traffic wakeups busy"),
    ("Health", "health", None, "score reboot failed services battery wear drives smart errors oom"),
    ("History", "history", None, "charts graphs timeline disk explorer storage"),
    ("Logs", "logs", None, "daily weekly monthly yearly crashes screen time usage packages suspend lid"),
    ("Investigate", "diagnose", None, "slow diagnosis port who is using"),
    ("Projects", "projects", None, "git repos uncommitted unpushed todo code"),
    ("Tweaks", "tweaks", None, "settings presets tuning kernel sysctl hyprland options toggles"),
    ("Control — Apps & Processes", "control", Some("apps"), "kill freeze priority affinity windows workspaces namespaces"),
    ("Control — Power & Memory", "control", Some("power"), "lock suspend reboot shutdown power profile governor cores fans swap brightness volume gpu"),
    ("Control — Network", "control", Some("network"), "vpn kill switch split tunnel wifi bluetooth firewall ports bandwidth"),
    ("Control — Display & Devices", "control", Some("devices"), "monitor resolution refresh scale night light audio output input usb clipboard"),
    ("Control — Services", "control", Some("services"), "systemd docker containers autostart startup logs"),
    ("Control — Automation", "control", Some("automation"), "profiles rules schedules alerts tray start at login action log"),
    ("Control — Packages & Cleanup", "control", Some("packages"), "updates pacman orphans cache trash journal"),
];

fn words_match(haystack: &str, needle_words: &[String]) -> bool {
    let haystack = haystack.to_lowercase();
    needle_words.iter().all(|w| haystack.contains(w))
}

pub fn search(db: &Db, sys: &System, query: &str) -> Vec<Hit> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    if words.is_empty() || query.len() > 100 {
        return Vec::new();
    }
    let mut hits = Vec::new();

    for (title, page, section, keywords) in PAGES {
        if words_match(&format!("{title} {keywords}"), &words) {
            hits.push(Hit {
                kind: "page".to_string(),
                title: title.to_string(),
                detail: "Go to page".to_string(),
                page: page.to_string(),
                section: section.map(String::from),
                query: None,
            });
        }
    }

    // Settings: title matches rank above matches that are only in the help text.
    let mut settings: Vec<(bool, &crate::tweaks::Tweak)> = crate::tweaks::catalog()
        .iter()
        .filter(|t| words_match(&format!("{} {} {} {}", t.title, t.help, t.id, t.category), &words))
        .map(|t| (words_match(t.title, &words), t))
        .collect();
    settings.sort_by_key(|(in_title, _)| !*in_title);
    for (_, t) in settings.into_iter().take(8) {
        hits.push(Hit {
            kind: "setting".to_string(),
            title: t.title.to_string(),
            detail: t.category.to_string(),
            page: "tweaks".to_string(),
            section: None,
            query: Some(t.title.to_string()),
        });
    }

    // Processes, one hit per program name.
    let mut names: std::collections::BTreeMap<String, (usize, u64)> = std::collections::BTreeMap::new();
    for p in sys.processes().values().filter(|p| p.thread_kind().is_none()) {
        let name = p.name().to_string_lossy().to_string();
        if words_match(&name, &words) {
            let entry = names.entry(name).or_default();
            entry.0 += 1;
            entry.1 += p.memory();
        }
    }
    let mut names: Vec<_> = names.into_iter().collect();
    names.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (name, (count, memory)) in names.into_iter().take(5) {
        hits.push(Hit {
            kind: "process".to_string(),
            detail: format!("{count} running · {:.0} MB", memory as f64 / 1024.0 / 1024.0),
            page: "control".to_string(),
            section: Some("apps".to_string()),
            query: Some(name.clone()),
            title: name,
        });
    }

    let units = crate::sys_util::run_lossy(std::process::Command::new("systemctl").args([
        "list-unit-files", "--type=service,timer", "--no-legend", "--plain", "--no-pager",
    ]));
    for line in units.lines() {
        let mut parts = line.split_whitespace();
        let (Some(unit), Some(state)) = (parts.next(), parts.next()) else {
            continue;
        };
        if unit.contains('@') && unit.ends_with("@.service") {
            continue;
        }
        if words_match(unit, &words) {
            hits.push(Hit {
                kind: "service".to_string(),
                title: unit.to_string(),
                detail: state.to_string(),
                page: "control".to_string(),
                section: Some("services".to_string()),
                query: Some(unit.to_string()),
            });
        }
    }
    let service_hits = hits.iter().filter(|h| h.kind == "service").count();
    if service_hits > 6 {
        let mut kept = 0;
        hits.retain(|h| {
            if h.kind != "service" {
                return true;
            }
            kept += 1;
            kept <= 6
        });
    }

    // The logbook: recent events, crashes, package changes and actions.
    let like = format!("%{}%", words.join("%"));
    let conn = db.conn.lock().unwrap();
    let mut log = |sql: &str, label: &str| {
        let Ok(mut stmt) = conn.prepare(sql) else {
            return;
        };
        let rows = stmt.query_map(params![like], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)));
        for (ts, text) in rows.into_iter().flatten().flatten() {
            hits.push(Hit {
                kind: "log".to_string(),
                title: text,
                detail: format!("{label}|{ts}"),
                page: "logs".to_string(),
                section: None,
                query: None,
            });
        }
    };
    log("SELECT ts, message FROM events WHERE message LIKE ?1 ORDER BY ts DESC LIMIT 3", "Event");
    log(
        "SELECT time_us / 1000000, exe || ' crashed' FROM crashes WHERE exe LIKE ?1 ORDER BY time_us DESC LIMIT 3",
        "Crash",
    );
    log(
        "SELECT ts, action || ' ' || name || ' ' || new_version FROM package_events WHERE name LIKE ?1 ORDER BY ts DESC LIMIT 3",
        "Package",
    );
    log(
        "SELECT ts, action || ': ' || message FROM actions_log WHERE action LIKE ?1 OR message LIKE ?1 ORDER BY ts DESC LIMIT 3",
        "Action",
    );
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match() {
        let words = vec!["kill".to_string(), "switch".to_string()];
        assert!(words_match("Control — Network vpn kill switch", &words));
        assert!(!words_match("kill a process", &words));
    }
}
