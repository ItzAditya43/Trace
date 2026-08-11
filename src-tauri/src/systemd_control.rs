use serde::Serialize;
use zbus::Connection;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    #[allow(clippy::type_complexity)]
    fn list_units(
        &self,
    ) -> zbus::Result<
        Vec<(
            String, // name
            String, // description
            String, // load state
            String, // active state
            String, // sub state
            String, // follow
            zbus::zvariant::OwnedObjectPath, // unit object path
            u32,    // job id
            String, // job type
            zbus::zvariant::OwnedObjectPath, // job object path
        )>,
    >;

    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[derive(Serialize, Clone)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
}

pub async fn list_units(filter_running_or_named: bool) -> Result<Vec<UnitInfo>, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("Cannot connect to system D-Bus: {e}"))?;
    let proxy = SystemdManagerProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?;
    let units = proxy.list_units().await.map_err(|e| e.to_string())?;

    let mut result: Vec<UnitInfo> = units
        .into_iter()
        .filter(|u| u.0.ends_with(".service"))
        .filter(|u| !filter_running_or_named || u.3 == "active" || u.3 == "failed")
        .map(|u| UnitInfo {
            name: u.0,
            description: u.1,
            load_state: u.2,
            active_state: u.3,
            sub_state: u.4,
        })
        .collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

pub async fn start_unit(name: &str) -> Result<String, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("Cannot connect to system D-Bus: {e}"))?;
    let proxy = SystemdManagerProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .start_unit(name, "replace")
        .await
        .map(|_| format!("Started {name}"))
        .map_err(|e| format!("Failed to start {name}: {e} (systemd will normally prompt for authorization via polkit)"))
}

pub async fn stop_unit(name: &str) -> Result<String, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("Cannot connect to system D-Bus: {e}"))?;
    let proxy = SystemdManagerProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .stop_unit(name, "replace")
        .await
        .map(|_| format!("Stopped {name}"))
        .map_err(|e| format!("Failed to stop {name}: {e}"))
}

pub async fn restart_unit(name: &str) -> Result<String, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("Cannot connect to system D-Bus: {e}"))?;
    let proxy = SystemdManagerProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .restart_unit(name, "replace")
        .await
        .map(|_| format!("Restarted {name}"))
        .map_err(|e| format!("Failed to restart {name}: {e}"))
}
