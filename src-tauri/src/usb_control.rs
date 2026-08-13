use serde::Serialize;
use std::fs;

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

#[derive(Serialize)]
pub struct UsbDevice {
    pub device_id: String,
    pub vendor_id: String,
    pub product_id: String,
    pub manufacturer: String,
    pub product: String,
    pub authorized: bool,
}

fn read_trim(path: &std::path::Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn list_devices() -> Vec<UsbDevice> {
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Only top-level USB devices have idVendor; interfaces don't.
        let Some(vendor_id) = read_trim(&path.join("idVendor")) else {
            continue;
        };
        let product_id = read_trim(&path.join("idProduct")).unwrap_or_default();
        let manufacturer = read_trim(&path.join("manufacturer")).unwrap_or_default();
        let product = read_trim(&path.join("product")).unwrap_or_default();
        let authorized = read_trim(&path.join("authorized"))
            .map(|s| s == "1")
            .unwrap_or(true);
        devices.push(UsbDevice {
            device_id: entry.file_name().to_string_lossy().to_string(),
            vendor_id,
            product_id,
            manufacturer,
            product,
            authorized,
        });
    }
    devices.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    devices
}

pub fn set_authorized(device_id: &str, authorized: bool) -> ActionResult {
    let path = format!("/sys/bus/usb/devices/{device_id}/authorized");
    let value = if authorized { "1" } else { "0" };
    match fs::write(&path, value) {
        Ok(()) => ok(format!(
            "{} device {device_id}",
            if authorized { "Authorized" } else { "Deauthorized" }
        )),
        Err(e) => err(format!(
            "Failed to write {path}: {e} (usually needs root — deauthorizing your own \
             keyboard/mouse this way can lock you out until replug, so double-check the device)"
        )),
    }
}
