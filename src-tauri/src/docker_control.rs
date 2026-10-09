use bollard::container::{ListContainersOptions, RestartContainerOptions, StartContainerOptions, StopContainerOptions};
use bollard::Docker;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
}

fn connect() -> Result<Docker, String> {
    Docker::connect_with_socket_defaults()
        .map_err(|e| format!("Cannot connect to Docker daemon: {e} (is Docker running / is your user in the docker group?)"))
}

pub async fn list_containers() -> Result<Vec<ContainerInfo>, String> {
    let docker = connect()?;
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            filters: HashMap::new(),
            ..Default::default()
        }))
        .await
        .map_err(|e| e.to_string())?;

    Ok(containers
        .into_iter()
        .map(|c| ContainerInfo {
            id: c.id.unwrap_or_default().chars().take(12).collect(),
            name: c
                .names
                .and_then(|n| n.first().cloned())
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string(),
            image: c.image.unwrap_or_default(),
            state: c.state.map(|s| format!("{s:?}")).unwrap_or_default(),
            status: c.status.unwrap_or_default(),
        })
        .collect())
}

pub async fn start_container(id: &str) -> Result<String, String> {
    let docker = connect()?;
    docker
        .start_container(id, None::<StartContainerOptions<String>>)
        .await
        .map(|_| format!("Started container {id}"))
        .map_err(|e| format!("Failed to start {id}: {e}"))
}

pub async fn stop_container(id: &str) -> Result<String, String> {
    let docker = connect()?;
    docker
        .stop_container(id, None::<StopContainerOptions>)
        .await
        .map(|_| format!("Stopped container {id}"))
        .map_err(|e| format!("Failed to stop {id}: {e}"))
}

pub async fn restart_container(id: &str) -> Result<String, String> {
    let docker = connect()?;
    docker
        .restart_container(id, None::<RestartContainerOptions>)
        .await
        .map(|_| format!("Restarted container {id}"))
        .map_err(|e| format!("Failed to restart {id}: {e}"))
}

fn is_container_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// The last lines a container printed (stdout and stderr together).
pub fn container_logs(id: &str) -> Result<String, String> {
    if !is_container_id(id) {
        return Err("Not a container id".to_string());
    }
    let out = std::process::Command::new("timeout")
        .args(["10", "docker", "logs", "--tail", "200", id])
        .output()
        .map_err(|e| format!("Couldn't run docker: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    // Trim to something a UI can show without choking.
    let tail: String = text.chars().rev().take(20_000).collect::<Vec<_>>().into_iter().rev().collect();
    Ok(tail)
}

#[derive(Serialize)]
pub struct ContainerStats {
    pub cpu: String,
    pub memory: String,
    pub memory_percent: String,
    pub network: String,
    pub disk: String,
    pub processes: String,
}

/// One reading of a running container's resource use.
pub fn container_stats(id: &str) -> Result<ContainerStats, String> {
    if !is_container_id(id) {
        return Err("Not a container id".to_string());
    }
    let out = std::process::Command::new("timeout")
        .args(["10", "docker", "stats", "--no-stream", "--format", "{{json .}}", id])
        .output()
        .map_err(|e| format!("Couldn't run docker: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|_| "The container isn't running, so it has no live stats".to_string())?;
    let field = |key: &str| json[key].as_str().unwrap_or("—").to_string();
    Ok(ContainerStats {
        cpu: field("CPUPerc"),
        memory: field("MemUsage"),
        memory_percent: field("MemPerc"),
        network: field("NetIO"),
        disk: field("BlockIO"),
        processes: field("PIDs"),
    })
}
