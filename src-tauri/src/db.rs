use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db {
    pub conn: Mutex<Connection>,
}

fn db_path() -> PathBuf {
    let mut dir = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("trace");
    std::fs::create_dir_all(&dir).ok();
    dir.push("history.sqlite3");
    dir
}

impl Db {
    pub fn open() -> Self {
        let conn = Connection::open(db_path()).expect("failed to open trace history db");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS resource_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                cpu_percent REAL NOT NULL,
                used_memory_bytes INTEGER NOT NULL,
                total_memory_bytes INTEGER NOT NULL,
                used_swap_bytes INTEGER NOT NULL,
                gpu_usage_percent REAL,
                gpu_vram_used_bytes INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_resource_ts ON resource_snapshots(ts);

            CREATE TABLE IF NOT EXISTS disk_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                mount_point TEXT NOT NULL,
                used_bytes INTEGER NOT NULL,
                total_bytes INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_disk_ts ON disk_snapshots(ts);

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                severity TEXT NOT NULL,
                message TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);

            CREATE TABLE IF NOT EXISTS actions_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                action TEXT NOT NULL,
                ok INTEGER NOT NULL,
                message TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_actions_ts ON actions_log(ts);
            ",
        )
        .expect("failed to init schema");
        Db {
            conn: Mutex::new(conn),
        }
    }

    pub fn insert_resource(
        &self,
        ts: i64,
        cpu_percent: f32,
        used_memory_bytes: u64,
        total_memory_bytes: u64,
        used_swap_bytes: u64,
        gpu_usage_percent: Option<f32>,
        gpu_vram_used_bytes: Option<u64>,
    ) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO resource_snapshots
             (ts, cpu_percent, used_memory_bytes, total_memory_bytes, used_swap_bytes, gpu_usage_percent, gpu_vram_used_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                ts,
                cpu_percent,
                used_memory_bytes as i64,
                total_memory_bytes as i64,
                used_swap_bytes as i64,
                gpu_usage_percent,
                gpu_vram_used_bytes.map(|v| v as i64),
            ],
        )
        .ok();
    }

    pub fn insert_disk(&self, ts: i64, mount_point: &str, used_bytes: u64, total_bytes: u64) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO disk_snapshots (ts, mount_point, used_bytes, total_bytes) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, mount_point, used_bytes as i64, total_bytes as i64],
        )
        .ok();
    }

    pub fn insert_event(&self, ts: i64, event_type: &str, severity: &str, message: &str) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (ts, event_type, severity, message) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, event_type, severity, message],
        )
        .ok();
    }

    pub fn events_since(&self, since_ts: i64) -> Vec<EventRow> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts, event_type, severity, message FROM events
                 WHERE ts >= ?1 ORDER BY ts ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts], |row| {
                Ok(EventRow {
                    ts: row.get(0)?,
                    event_type: row.get(1)?,
                    severity: row.get(2)?,
                    message: row.get(3)?,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    pub fn insert_action(&self, ts: i64, action: &str, ok: bool, message: &str) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO actions_log (ts, action, ok, message) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, action, ok as i32, message],
        )
        .ok();
    }

    pub fn actions_since(&self, since_ts: i64) -> Vec<ActionRow> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts, action, ok, message FROM actions_log
                 WHERE ts >= ?1 ORDER BY ts DESC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts], |row| {
                Ok(ActionRow {
                    ts: row.get(0)?,
                    action: row.get(1)?,
                    ok: row.get::<_, i32>(2)? != 0,
                    message: row.get(3)?,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    pub fn resource_history(&self, since_ts: i64) -> Vec<ResourcePoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts, cpu_percent, used_memory_bytes, total_memory_bytes, used_swap_bytes, gpu_usage_percent
                 FROM resource_snapshots WHERE ts >= ?1 ORDER BY ts ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts], |row| {
                Ok(ResourcePoint {
                    ts: row.get(0)?,
                    cpu_percent: row.get(1)?,
                    used_memory_bytes: row.get::<_, i64>(2)? as u64,
                    total_memory_bytes: row.get::<_, i64>(3)? as u64,
                    used_swap_bytes: row.get::<_, i64>(4)? as u64,
                    gpu_usage_percent: row.get(5)?,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    pub fn disk_history(&self, since_ts: i64) -> Vec<DiskPoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts, mount_point, used_bytes, total_bytes
                 FROM disk_snapshots WHERE ts >= ?1 ORDER BY ts ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts], |row| {
                Ok(DiskPoint {
                    ts: row.get(0)?,
                    mount_point: row.get(1)?,
                    used_bytes: row.get::<_, i64>(2)? as u64,
                    total_bytes: row.get::<_, i64>(3)? as u64,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }
}

#[derive(Serialize, Clone)]
pub struct ResourcePoint {
    pub ts: i64,
    pub cpu_percent: f32,
    pub used_memory_bytes: u64,
    pub total_memory_bytes: u64,
    pub used_swap_bytes: u64,
    pub gpu_usage_percent: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct DiskPoint {
    pub ts: i64,
    pub mount_point: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct ActionRow {
    pub ts: i64,
    pub action: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Serialize, Clone)]
pub struct EventRow {
    pub ts: i64,
    pub event_type: String,
    pub severity: String,
    pub message: String,
}
