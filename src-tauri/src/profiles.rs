use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Action {
    FreezeProcess { pid: u32 },
    ResumeProcess { pid: u32 },
    KillProcess { pid: u32, force: bool },
    SetCpuLimit { pid: u32, percent: u32 },
    SetMemoryLimitMb { pid: u32, mb: u64 },
    StartService { name: String },
    StopService { name: String },
    RestartService { name: String },
    StartContainer { id: String },
    StopContainer { id: String },
}

impl Action {
    /// The action that would reverse this one, where reversal is well-defined.
    pub fn inverse(&self) -> Option<Action> {
        match self {
            Action::FreezeProcess { pid } => Some(Action::ResumeProcess { pid: *pid }),
            Action::ResumeProcess { pid } => Some(Action::FreezeProcess { pid: *pid }),
            Action::StartService { name } => Some(Action::StopService { name: name.clone() }),
            Action::StopService { name } => Some(Action::StartService { name: name.clone() }),
            Action::StartContainer { id } => Some(Action::StopContainer { id: id.clone() }),
            Action::StopContainer { id } => Some(Action::StartContainer { id: id.clone() }),
            // Kill and limit changes are not safely reversible — no prior state captured.
            Action::KillProcess { .. } => None,
            Action::SetCpuLimit { .. } => None,
            Action::SetMemoryLimitMb { .. } => None,
            Action::RestartService { .. } => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Action::FreezeProcess { pid } => format!("Freeze PID {pid}"),
            Action::ResumeProcess { pid } => format!("Resume PID {pid}"),
            Action::KillProcess { pid, force } => {
                format!("Kill PID {pid}{}", if *force { " (force)" } else { "" })
            }
            Action::SetCpuLimit { pid, percent } => {
                format!("Limit PID {pid} to {percent}% CPU")
            }
            Action::SetMemoryLimitMb { pid, mb } => format!("Limit PID {pid} to {mb} MB RAM"),
            Action::StartService { name } => format!("Start service {name}"),
            Action::StopService { name } => format!("Stop service {name}"),
            Action::RestartService { name } => format!("Restart service {name}"),
            Action::StartContainer { id } => format!("Start container {id}"),
            Action::StopContainer { id } => format!("Stop container {id}"),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub actions: Vec<Action>,
}

fn profiles_dir() -> PathBuf {
    let mut dir = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("trace");
    dir.push("profiles");
    fs::create_dir_all(&dir).ok();
    dir
}

pub fn list_profiles() -> Vec<Profile> {
    let dir = profiles_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|text| toml::from_str::<Profile>(&text).ok())
        .collect()
}

pub fn save_profile(profile: &Profile) -> Result<(), String> {
    let path = profiles_dir().join(format!("{}.toml", slugify(&profile.name)));
    let text = toml::to_string_pretty(profile).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn ensure_default_profiles() {
    if !list_profiles().is_empty() {
        return;
    }
    let focus = Profile {
        name: "Focus Mode".to_string(),
        description: "Freeze processes you name to cut background distraction/CPU noise. Fully reversible.".to_string(),
        actions: vec![],
    };
    save_profile(&focus).ok();
}
