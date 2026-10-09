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

            CREATE TABLE IF NOT EXISTS app_memory (
                ts INTEGER NOT NULL,
                app TEXT NOT NULL,
                ram_bytes INTEGER NOT NULL,
                swap_bytes INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_app_memory_ts ON app_memory(ts);

            CREATE TABLE IF NOT EXISTS pressure_snapshots (
                ts INTEGER NOT NULL,
                cpu_some REAL NOT NULL,
                io_some REAL NOT NULL,
                mem_some REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pressure_ts ON pressure_snapshots(ts);

            CREATE TABLE IF NOT EXISTS boots (
                boot_id TEXT PRIMARY KEY,
                ts INTEGER NOT NULL,
                total_ms INTEGER NOT NULL,
                userspace_ms INTEGER NOT NULL
            );
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

    /// Per-app samples are the bulkiest table, so they're kept for two weeks.
    pub fn prune_app_memory(&self, before_ts: i64) {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM app_memory WHERE ts < ?1", rusqlite::params![before_ts])
            .ok();
    }

    pub fn insert_app_memory(&self, ts: i64, apps: &[(String, u64, u64)]) {
        let mut conn = self.conn.lock().unwrap();
        let Ok(tx) = conn.transaction() else {
            return;
        };
        for (app, ram, swap) in apps {
            tx.execute(
                "INSERT INTO app_memory (ts, app, ram_bytes, swap_bytes) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ts, app, *ram as i64, *swap as i64],
            )
            .ok();
        }
        tx.commit().ok();
    }

    /// Averages samples into `bucket_secs`-wide buckets so a week of history
    /// comes back as a few hundred points per app instead of tens of thousands.
    pub fn app_memory_history(&self, since_ts: i64, bucket_secs: i64) -> Vec<AppMemoryPoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT (ts / ?2) * ?2 AS bucket, app, AVG(ram_bytes), AVG(swap_bytes)
                 FROM app_memory WHERE ts >= ?1 GROUP BY bucket, app ORDER BY bucket ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts, bucket_secs.max(1)], |row| {
                Ok(AppMemoryPoint {
                    ts: row.get(0)?,
                    app: row.get(1)?,
                    ram_bytes: row.get::<_, f64>(2)? as u64,
                    swap_bytes: row.get::<_, f64>(3)? as u64,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    pub fn insert_pressure(&self, ts: i64, cpu_some: f32, io_some: f32, mem_some: f32) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO pressure_snapshots (ts, cpu_some, io_some, mem_some) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, cpu_some, io_some, mem_some],
        )
        .ok();
    }

    pub fn pressure_history(&self, since_ts: i64) -> Vec<PressurePoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts, cpu_some, io_some, mem_some FROM pressure_snapshots
                 WHERE ts >= ?1 ORDER BY ts ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![since_ts], |row| {
                Ok(PressurePoint {
                    ts: row.get(0)?,
                    cpu_some: row.get(1)?,
                    io_some: row.get(2)?,
                    mem_some: row.get(3)?,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    /// Records this boot once; later calls for the same boot are no-ops.
    pub fn record_boot(&self, boot_id: &str, ts: i64, total_ms: u64, userspace_ms: u64) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO boots (boot_id, ts, total_ms, userspace_ms) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![boot_id, ts, total_ms as i64, userspace_ms as i64],
        )
        .ok();
    }

    pub fn boot_history(&self) -> Vec<BootPoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT ts, total_ms, userspace_ms FROM boots ORDER BY ts DESC LIMIT 30")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok(BootPoint {
                    ts: row.get(0)?,
                    total_ms: row.get::<_, i64>(1)? as u64,
                    userspace_ms: row.get::<_, i64>(2)? as u64,
                })
            })
            .unwrap();
        rows.filter_map(Result::ok).collect()
    }

    /// Everything the History page charts, averaged into `bucket_secs`-wide
    /// buckets so a month of 10-second samples comes back as a few hundred points.
    pub fn history(&self, since_ts: i64, bucket_secs: i64) -> History {
        let bucket = bucket_secs.max(10);
        // Scoped so the connection lock is released before `events_since`
        // takes it again below.
        let (resources, pressure, disks) = {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT (ts / ?2) * ?2, AVG(cpu_percent), MAX(cpu_percent),
                        AVG(used_memory_bytes * 100.0 / total_memory_bytes), AVG(used_memory_bytes),
                        AVG(used_swap_bytes), AVG(gpu_usage_percent)
                 FROM resource_snapshots WHERE ts >= ?1 AND total_memory_bytes > 0
                 GROUP BY 1 ORDER BY 1",
            )
            .unwrap();
        let resources = stmt
            .query_map(rusqlite::params![since_ts, bucket], |row| {
                Ok(HistoryPoint {
                    ts: row.get(0)?,
                    cpu_percent: row.get(1)?,
                    cpu_peak: row.get(2)?,
                    ram_percent: row.get(3)?,
                    ram_bytes: row.get::<_, f64>(4)? as u64,
                    swap_bytes: row.get::<_, f64>(5)? as u64,
                    gpu_percent: row.get(6)?,
                })
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        let mut stmt = conn
            .prepare(
                "SELECT (ts / ?2) * ?2, AVG(cpu_some), AVG(io_some), AVG(mem_some)
                 FROM pressure_snapshots WHERE ts >= ?1 GROUP BY 1 ORDER BY 1",
            )
            .unwrap();
        let pressure = stmt
            .query_map(rusqlite::params![since_ts, bucket], |row| {
                Ok(PressurePoint {
                    ts: row.get(0)?,
                    cpu_some: row.get(1)?,
                    io_some: row.get(2)?,
                    mem_some: row.get(3)?,
                })
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        let mut stmt = conn
            .prepare(
                "SELECT (ts / ?2) * ?2, mount_point, AVG(used_bytes), MAX(total_bytes)
                 FROM disk_snapshots WHERE ts >= ?1 GROUP BY 1, 2 ORDER BY 1",
            )
            .unwrap();
        let disks = stmt
            .query_map(rusqlite::params![since_ts, bucket], |row| {
                Ok(DiskPoint {
                    ts: row.get(0)?,
                    mount_point: row.get(1)?,
                    used_bytes: row.get::<_, f64>(2)? as u64,
                    total_bytes: row.get::<_, i64>(3)? as u64,
                })
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        (resources, pressure, disks)
        };
        History {
            resources,
            pressure,
            disks,
            events: self.events_since(since_ts),
        }
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

#[derive(Serialize, Clone)]
pub struct AppMemoryPoint {
    pub ts: i64,
    pub app: String,
    pub ram_bytes: u64,
    pub swap_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct PressurePoint {
    pub ts: i64,
    pub cpu_some: f32,
    pub io_some: f32,
    pub mem_some: f32,
}

#[derive(Serialize, Clone)]
pub struct BootPoint {
    pub ts: i64,
    pub total_ms: u64,
    pub userspace_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct HistoryPoint {
    pub ts: i64,
    pub cpu_percent: f32,
    pub cpu_peak: f32,
    pub ram_percent: f32,
    pub ram_bytes: u64,
    pub swap_bytes: u64,
    pub gpu_percent: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct History {
    pub resources: Vec<HistoryPoint>,
    pub pressure: Vec<PressurePoint>,
    pub disks: Vec<DiskPoint>,
    pub events: Vec<EventRow>,
}
