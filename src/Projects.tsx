import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ProjectInfo {
  name: string;
  path: string;
  languages: string[];
  has_git: boolean;
  commit_count: number;
  last_commit_days_ago: number | null;
  todo_count: number;
  disk_bytes: number;
  dependency_count: number | null;
  activity: "active" | "dormant" | "abandoned" | "no_git";
}

const ACTIVITY_META: Record<
  ProjectInfo["activity"],
  { label: string; icon: string; className: string }
> = {
  active: { label: "Active", icon: "🔥", className: "activity-active" },
  dormant: { label: "Dormant", icon: "🟡", className: "activity-dormant" },
  abandoned: { label: "Abandoned", icon: "💀", className: "activity-abandoned" },
  no_git: { label: "No git repo", icon: "⚪", className: "activity-nogit" },
};

function bytesToGB(bytes: number): string {
  return (bytes / 1024 / 1024 / 1024).toFixed(2);
}

function bytesToHuman(bytes: number): string {
  const gb = bytes / 1024 / 1024 / 1024;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
}

export default function Projects() {
  const [root, setRoot] = useState("~/Desktop/CODE");
  const [projects, setProjects] = useState<ProjectInfo[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<ProjectInfo | null>(null);

  async function scan() {
    setLoading(true);
    setSelected(null);
    try {
      const expandedRoot = root.startsWith("~")
        ? root.replace("~", await homeDir())
        : root;
      const result = await invoke<ProjectInfo[]>("scan_projects", {
        root: expandedRoot,
      });
      result.sort((a, b) => b.disk_bytes - a.disk_bytes);
      setProjects(result);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  async function homeDir(): Promise<string> {
    try {
      const { homeDir: getHome } = await import("@tauri-apps/api/path");
      return await getHome();
    } catch {
      return "/root";
    }
  }

  const groups: Record<string, ProjectInfo[]> = {
    active: [],
    dormant: [],
    abandoned: [],
    no_git: [],
  };
  (projects ?? []).forEach((p) => groups[p.activity].push(p));

  return (
    <div>
      <section className="panel">
        <div className="scan-controls">
          <input
            className="port-input"
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            placeholder="Path to scan, e.g. ~/Desktop/CODE"
          />
          <button className="port-btn" onClick={scan} disabled={loading}>
            {loading ? "Scanning…" : "Scan"}
          </button>
        </div>
        <p className="diagnose-hint">
          Looks for git repos, package manifests, TODOs, and disk usage in
          each immediate subdirectory.
        </p>
      </section>

      {projects && (
        <div className="projects-layout">
          <div className="projects-list">
            {(["active", "dormant", "abandoned", "no_git"] as const).map(
              (key) =>
                groups[key].length > 0 && (
                  <div key={key} className="project-group">
                    <h3 className={ACTIVITY_META[key].className}>
                      {ACTIVITY_META[key].icon} {ACTIVITY_META[key].label} (
                      {groups[key].length})
                    </h3>
                    {groups[key].map((p) => (
                      <button
                        key={p.path}
                        className={
                          "project-item" +
                          (selected?.path === p.path ? " selected" : "")
                        }
                        onClick={() => setSelected(p)}
                      >
                        <span className="project-name">{p.name}</span>
                        <span className="project-size">
                          {bytesToHuman(p.disk_bytes)}
                        </span>
                      </button>
                    ))}
                  </div>
                )
            )}
          </div>

          <div className="project-detail">
            {selected ? (
              <>
                <h2 className="project-detail-title">{selected.name}</h2>
                <div className="detail-row">
                  <span>Languages</span>
                  <span>
                    {selected.languages.length
                      ? selected.languages.join(", ")
                      : "Unknown"}
                  </span>
                </div>
                <div className="detail-row">
                  <span>Git</span>
                  <span>
                    {selected.has_git
                      ? `${selected.commit_count} commits`
                      : "Not a git repo"}
                  </span>
                </div>
                {selected.last_commit_days_ago !== null && (
                  <div className="detail-row">
                    <span>Last commit</span>
                    <span>
                      {selected.last_commit_days_ago === 0
                        ? "Today"
                        : `${selected.last_commit_days_ago} days ago`}
                    </span>
                  </div>
                )}
                {selected.dependency_count !== null && (
                  <div className="detail-row">
                    <span>Dependencies</span>
                    <span>{selected.dependency_count}</span>
                  </div>
                )}
                <div className="detail-row">
                  <span>TODOs / FIXMEs</span>
                  <span>{selected.todo_count}</span>
                </div>
                <div className="detail-row">
                  <span>Disk usage</span>
                  <span>{bytesToGB(selected.disk_bytes)} GB</span>
                </div>
                <div className="detail-row">
                  <span>Path</span>
                  <span className="project-path">{selected.path}</span>
                </div>
              </>
            ) : (
              <p className="diagnose-hint">Select a project to see details.</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
