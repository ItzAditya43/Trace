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
