use serde::Serialize;
use tokio::process::Command;

#[derive(Serialize, Clone)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

/// Runs `du -b --max-depth=1 <path>` and returns the immediate children
/// sorted largest-first. This walks the whole subtree to size each child,
/// so on a large directory (home, /) it can take tens of seconds — it's
/// meant to be triggered on demand from the UI, not polled.
pub async fn scan_folder(path: &str) -> Result<Vec<FolderEntry>, String> {
    let output = Command::new("du")
        .args(["-b", "--max-depth=1", path])
        .output()
        .await
        .map_err(|e| format!("Failed to run du: {e}"))?;

    // du exits non-zero if it hits any permission-denied subdirectory, but
    // still prints everything it could read on stdout — that's still a
    // useful partial result, so only bail if stdout is truly empty.
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("du produced no output for {path}: {stderr}"));
    }

    let mut entries: Vec<FolderEntry> = text
        .lines()
        .filter_map(|line| {
            let (size_str, entry_path) = line.split_once('\t')?;
            let size_bytes: u64 = size_str.parse().ok()?;
            // Skip the summary line for the queried path itself.
            if entry_path.trim_end_matches('/') == path.trim_end_matches('/') {
                return None;
            }
            let name = entry_path.rsplit('/').next().unwrap_or(entry_path).to_string();
            Some(FolderEntry {
                name,
                path: entry_path.to_string(),
                size_bytes,
                is_dir: true,
            })
        })
        .collect();

    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    entries.truncate(50);
    Ok(entries)
}
