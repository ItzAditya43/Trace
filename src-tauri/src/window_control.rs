use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Serialize)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

fn ok(msg: impl Into<String>) -> ActionResult {
    ActionResult { ok: true, message: msg.into() }
}

fn err(msg: impl Into<String>) -> ActionResult {
    ActionResult { ok: false, message: msg.into() }
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn is_hyprland() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
}

#[derive(Deserialize, Serialize)]
pub struct HyprWorkspaceRef {
    id: i32,
    name: String,
}

#[derive(Deserialize, Serialize)]
pub struct WindowInfo {
    address: String,
    class: String,
    title: String,
    pid: u32,
    workspace: HyprWorkspaceRef,
}

pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
    if !is_hyprland() {
        return Err("Not running under Hyprland (only compositor supported so far)".to_string());
    }
    let out = run(Command::new("hyprctl").args(["clients", "-j"]))?;
    serde_json::from_str(&out).map_err(|e| format!("Failed to parse hyprctl output: {e}"))
}

pub fn move_window_to_workspace(address: &str, workspace: i32) -> ActionResult {
    match crate::hypr::move_window_to_workspace(address, workspace) {
        Ok(()) => ok(format!("Moved window to workspace {workspace}")),
        Err(e) => err(format!("Failed to move window: {e}")),
    }
}

pub fn focus_workspace(workspace: i32) -> ActionResult {
    match crate::hypr::focus_workspace(workspace) {
        Ok(()) => ok(format!("Switched to workspace {workspace}")),
        Err(e) => err(format!("Failed to switch workspace: {e}")),
    }
}

pub fn close_window(address: &str) -> ActionResult {
    match crate::hypr::close_window(address) {
        Ok(()) => ok("Asked the window to close"),
        Err(e) => err(format!("Failed to close window: {e}")),
    }
}
