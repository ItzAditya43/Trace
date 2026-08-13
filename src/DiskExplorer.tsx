import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";

interface FolderEntry {
  name: string;
  path: string;
  size_bytes: number;
  is_dir: boolean;
}

function bytesToHuman(bytes: number): string {
  const gb = bytes / 1024 / 1024 / 1024;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
}

export default function DiskExplorer() {
  const [path, setPath] = useState<string>("");
  const [history, setHistory] = useState<string[]>([]);
  const [entries, setEntries] = useState<FolderEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    homeDir().then((h) => setPath(h.replace(/\/$/, "")));
  }, []);

  async function scan(target: string) {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<FolderEntry[]>("scan_folder_sizes", { path: target });
      setEntries(result);
    } catch (e) {
      setError(String(e));
      setEntries(null);
    } finally {
      setLoading(false);
    }
  }

  function open(entry: FolderEntry) {
    setHistory((h) => [...h, path]);
    setPath(entry.path);
    scan(entry.path);
  }

  function goUp() {
    if (history.length === 0) return;
    const prev = history[history.length - 1];
    setHistory((h) => h.slice(0, -1));
    setPath(prev);
    scan(prev);
  }

  const maxSize = entries && entries.length > 0 ? entries[0].size_bytes : 1;

  return (
    <section className="panel">
      <h2>Disk Usage Explorer</h2>
      <div className="scan-controls">
        <input
          className="port-input"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="/home/you or /"
          onKeyDown={(e) => e.key === "Enter" && scan(path)}
        />
        <button className="port-btn" onClick={() => scan(path)} disabled={loading}>
          {loading ? "Scanning…" : "Scan"}
        </button>
        {history.length > 0 && (
          <button className="ctl-btn" onClick={goUp} disabled={loading}>
            ↑ Up
          </button>
        )}
      </div>
      <p className="diagnose-hint">
        Walks the folder tree with `du`, so a big directory (home, /) can
        take tens of seconds — click any row to drill into it.
      </p>

      {error && <p className="status-fail">✗ {error}</p>}

      {entries && (
        <div className="folder-list">
          {entries.map((e) => (
            <button key={e.path} className="folder-row" onClick={() => open(e)}>
              <span className="folder-name" title={e.path}>{e.name}</span>
              <div className="folder-bar-track">
                <div
                  className="folder-bar-fill"
                  style={{ width: `${(e.size_bytes / maxSize) * 100}%` }}
                />
              </div>
              <span className="folder-size">{bytesToHuman(e.size_bytes)}</span>
            </button>
          ))}
          {entries.length === 0 && (
            <p className="diagnose-hint">No subdirectories with measurable size here.</p>
          )}
        </div>
      )}
    </section>
  );
}
