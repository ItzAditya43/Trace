//! Talking to Hyprland. Since 0.5x Hyprland can be configured in Lua, and
//! in that mode `hyprctl keyword` and the old `hyprctl dispatch <name>` are
//! rejected — everything goes through `hyprctl eval` with the `hl.*` API
//! instead. This module hides the difference.

use crate::sys_util::{run, run_lossy};
use std::process::Command;
use std::sync::OnceLock;

/// True when the running Hyprland uses the Lua config parser.
pub fn is_lua() -> bool {
    static LUA: OnceLock<bool> = OnceLock::new();
    *LUA.get_or_init(|| run_lossy(Command::new("hyprctl").args(["repl", "return 1"])).trim() == "1")
}

pub fn available() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
}

/// A value for a Hyprland option.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Value {
    fn lua(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) => lua_string(s),
        }
    }

    fn legacy(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) => s.clone(),
        }
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n:.3}").trim_end_matches('0').to_string()
    }
}

/// Quotes a string for Lua. Only printable ASCII without quotes or
/// backslashes is accepted, so nothing can break out of the literal.
fn lua_string(s: &str) -> String {
    let safe: String = s
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .filter(|c| !matches!(c, '"' | '\\' | '\'' | '[' | ']'))
        .collect();
    format!("\"{safe}\"")
}

/// `decoration:blur:enabled` + `true` ->
/// `hl.config({ ["decoration"] = { ["blur"] = { ["enabled"] = true } } })`
fn lua_config(option: &str, value: &Value) -> String {
    let parts: Vec<&str> = option.split(':').collect();
    let mut expr = value.lua();
    for part in parts.iter().rev() {
        // The Lua API spells option names with underscores where the old
        // config format used hyphens (`tap-to-click` -> `tap_to_click`).
        expr = format!("{{ [{}] = {expr} }}", lua_string(&part.replace('-', "_")));
    }
    format!("hl.config({expr})")
}

fn eval(code: &str) -> Result<(), String> {
    let out = run(Command::new("hyprctl").args(["eval", code]))?;
    if out.trim() == "ok" || out.trim().is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

fn legacy(args: &[&str]) -> Result<(), String> {
    let out = run(Command::new("hyprctl").args(args))?;
    if out.trim() == "ok" {
        Ok(())
    } else {
        Err(out)
    }
}

/// Reads an option as text: booleans as "true"/"false", numbers trimmed,
/// multi-value options (gaps) as their first value. None if it doesn't exist.
pub fn get_option(option: &str) -> Option<String> {
    let out = run(Command::new("hyprctl").args(["getoption", option, "-j"])).ok()?;
    let json: serde_json::Value = serde_json::from_str(&out).ok()?;
    let object = json.as_object()?;
    for (key, value) in object {
        match key.as_str() {
            "option" | "set" => continue,
            "bool" => return Some(value.as_bool()?.to_string()),
            "int" => return Some(value.as_i64()?.to_string()),
            "float" => return Some(format_number((value.as_f64()? * 1000.0).round() / 1000.0)),
            _ => {
                let text = value.as_str()?;
                if text == "[[EMPTY]]" {
                    return Some(String::new());
                }
                // "4 4 4 4" (css-style gaps) -> "4"
                let first = text.split_whitespace().next().unwrap_or("");
                return Some(if key == "css" { first.to_string() } else { text.to_string() });
            }
        }
    }
    None
}

/// Changes an option for the running session. The config files on disk are
/// not touched, so `hyprctl reload` or logging out restores what they say.
pub fn set_option(option: &str, value: &Value) -> Result<(), String> {
    if is_lua() {
        eval(&lua_config(option, value))
    } else {
        legacy(&["keyword", option, &value.legacy()])
    }
}

fn is_address(address: &str) -> bool {
    address
        .strip_prefix("0x")
        .is_some_and(|hex| !hex.is_empty() && hex.len() <= 16 && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn close_window(address: &str) -> Result<(), String> {
    if !is_address(address) {
        return Err("Not a window address".to_string());
    }
    if is_lua() {
        eval(&format!("hl.dispatch(hl.dsp.window.close({{ window = \"address:{address}\" }}))"))
    } else {
        legacy(&["dispatch", "closewindow", &format!("address:{address}")])
    }
}

pub fn move_window_to_workspace(address: &str, workspace: i32) -> Result<(), String> {
    if !is_address(address) {
        return Err("Not a window address".to_string());
    }
    if is_lua() {
        eval(&format!(
            "hl.dispatch(hl.dsp.window.move({{ workspace = {workspace}, follow = false, window = \"address:{address}\" }}))"
        ))
    } else {
        legacy(&["dispatch", "movetoworkspacesilent", &format!("{workspace},address:{address}")])
    }
}

pub fn focus_workspace(workspace: i32) -> Result<(), String> {
    if is_lua() {
        eval(&format!("hl.dispatch(hl.dsp.focus({{ workspace = {workspace} }}))"))
    } else {
        legacy(&["dispatch", "workspace", &workspace.to_string()])
    }
}

/// `mode` like "1920x1080@60.00" (already validated against the monitor's
/// advertised modes by the caller).
pub fn set_monitor(name: &str, mode: &str, scale: f64) -> Result<(), String> {
    if is_lua() {
        eval(&format!(
            "hl.monitor({{ output = {}, mode = {}, position = \"auto\", scale = {} }})",
            lua_string(name),
            lua_string(mode),
            format_number(scale)
        ))
    } else {
        legacy(&["keyword", "monitor", &format!("{name},{mode},auto,{scale}")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_nested_lua_config() {
        assert_eq!(
            lua_config("decoration:blur:enabled", &Value::Bool(true)),
            r#"hl.config({ ["decoration"] = { ["blur"] = { ["enabled"] = true } } })"#
        );
        assert_eq!(
            lua_config("input:touchpad:tap-to-click", &Value::Bool(false)),
            r#"hl.config({ ["input"] = { ["touchpad"] = { ["tap_to_click"] = false } } })"#
        );
        assert_eq!(
            lua_config("input:sensitivity", &Value::Number(-0.25)),
            r#"hl.config({ ["input"] = { ["sensitivity"] = -0.25 } })"#
        );
    }

    #[test]
    fn lua_strings_cannot_be_escaped() {
        assert_eq!(lua_string(r#"flat"}) os.execute("x") --"#), r#""flat}) os.execute(x) --""#);
        assert_eq!(lua_string("a\\\nb"), "\"ab\"");
    }

    #[test]
    fn validates_window_addresses() {
        assert!(is_address("0x55f89e90a590"));
        assert!(!is_address("55f89e90a590"));
        assert!(!is_address("0x55\" }) os.exit() --"));
    }

    #[test]
    fn formats_numbers_without_noise() {
        assert_eq!(format_number(18.0), "18");
        assert_eq!(format_number(0.7), "0.7");
        assert_eq!(format_number(0.05), "0.05");
    }
}
