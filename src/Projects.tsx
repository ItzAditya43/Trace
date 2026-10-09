import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { ActionResult, Tile, bytesToHuman } from "./ui";
import { ProjectJunk } from "./More";

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
  branch: string | null;
  dirty_files: number;
  unpushed_commits: number | null;
  last_commit_subject: string | null;
  remote_url: string | null;
}

const ACTIVITY_LABEL: Record<ProjectInfo["activity"], string> = {
  active: "Active",
  dormant: "Dormant",
  abandoned: "Abandoned",
  no_git: "No git",
};

type Filter = "all" | "attention" | ProjectInfo["activity"];

const SORTS: { key: string; label: string; compare: (a: ProjectInfo, b: ProjectInfo) => number }[] = [
  {
    key: "recent",
    label: "Recently worked on",
    compare: (a, b) => (a.last_commit_days_ago ?? 1e9) - (b.last_commit_days_ago ?? 1e9),
  },
  { key: "size", label: "Largest", compare: (a, b) => b.disk_bytes - a.disk_bytes },
  { key: "name", label: "Name", compare: (a, b) => a.name.localeCompare(b.name) },
  { key: "todos", label: "Most TODOs", compare: (a, b) => b.todo_count - a.todo_count },
];

const ROOT_KEY = "trace.projects.root";

// The last scan, kept for the life of the window: a scan sizes every project
// on disk and takes seconds, so coming back to this page reuses it instead of
// rescanning. The Scan button refreshes it.
let lastScan: { root: string; projects: ProjectInfo[] } | null = null;

function needsAttention(p: ProjectInfo): boolean {
  return p.dirty_files > 0 || (p.unpushed_commits ?? 0) > 0;
}

function lastCommit(days: number | null): string {
  if (days === null) return "no commits";
  if (days === 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 60) return `${days} days ago`;
  if (days < 730) return `${Math.round(days / 30)} months ago`;
  return `${Math.round(days / 365)} years ago`;
}

function ProjectCard({ p }: { p: ProjectInfo }) {
  const [message, setMessage] = useState<ActionResult | null>(null);
  const open = async (using: string) => {
    const r = await invoke<ActionResult>("open_path", { path: p.path, with: using });
    setMessage(r.ok ? null : r);
  };

  return (
    <section className="panel project-card">
      <div className="project-card-head">
        <span className="project-card-name" title={p.path}>
          {p.name}
        </span>
        <span className={`chip activity-chip activity-${p.activity}`}>
          {ACTIVITY_LABEL[p.activity]}
        </span>
      </div>
      <div className="chip-row">
        {p.branch && <span className="chip">⎇ {p.branch}</span>}
        {p.languages.map((l) => (
          <span key={l} className="chip">
            {l}
          </span>
        ))}
      </div>
      {p.last_commit_subject && (
        <p className="project-commit" title={p.last_commit_subject}>
          “{p.last_commit_subject}” <span className="mem-muted">· {lastCommit(p.last_commit_days_ago)}</span>
        </p>
      )}
      <div className="project-flags">
        {p.dirty_files > 0 && (
          <span className="status-fail">● {p.dirty_files} uncommitted file(s)</span>
        )}
        {(p.unpushed_commits ?? 0) > 0 && (
          <span className="status-fail">↑ {p.unpushed_commits} unpushed commit(s)</span>
        )}
        {p.has_git && !needsAttention(p) && (
          <span className="status-ok">
            ✓ clean{p.unpushed_commits === null ? " (no upstream)" : " and pushed"}
          </span>
        )}
      </div>
      <div className="project-stats">
        <span>{bytesToHuman(p.disk_bytes)}</span>
        {p.has_git && <span>{p.commit_count} commits</span>}
        {p.dependency_count !== null && <span>{p.dependency_count} deps</span>}
        <span>{p.todo_count} TODOs</span>
      </div>
      <div className="control-actions project-actions">
        <button className="ctl-btn ctl-btn-sm" onClick={() => open("editor")}>
          Editor
        </button>
        <button className="ctl-btn ctl-btn-sm" onClick={() => open("terminal")}>
          Terminal
        </button>
        <button className="ctl-btn ctl-btn-sm" onClick={() => open("files")}>
          Files
        </button>
      </div>
      <ProjectJunk project={p.path} />
      {message && <p className="status-fail">✗ {message.message}</p>}
    </section>
  );
}

export default function Projects() {
  const [root, setRoot] = useState(() => {
    try {
      return localStorage.getItem(ROOT_KEY) ?? "~/Desktop/CODE";
    } catch {
      return "~/Desktop/CODE";
    }
  });
  const [projects, setProjects] = useState<ProjectInfo[] | null>(
    () => lastScan?.projects ?? null
  );
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");
  const [sortKey, setSortKey] = useState("recent");
  const [query, setQuery] = useState("");

  async function scan() {
    setLoading(true);
    try {
      const expanded = root.startsWith("~")
        ? root.replace("~", (await homeDir()).replace(/\/$/, ""))
        : root;
      const found = await invoke<ProjectInfo[]>("scan_projects", { root: expanded });
      lastScan = { root, projects: found };
      setProjects(found);
      try {
        localStorage.setItem(ROOT_KEY, root);
      } catch {
        // Remembering the folder is a convenience; scanning still worked.
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  // Scan the remembered folder the first time the page opens.
  useEffect(() => {
    if (!lastScan) scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const all = projects ?? [];
  const counts: Record<Filter, number> = {
    all: all.length,
    attention: all.filter(needsAttention).length,
    active: all.filter((p) => p.activity === "active").length,
    dormant: all.filter((p) => p.activity === "dormant").length,
    abandoned: all.filter((p) => p.activity === "abandoned").length,
    no_git: all.filter((p) => p.activity === "no_git").length,
  };
  const filters: { key: Filter; label: string }[] = [
    { key: "all", label: "All" },
    { key: "attention", label: "Needs attention" },
    { key: "active", label: "Active" },
    { key: "dormant", label: "Dormant" },
    { key: "abandoned", label: "Abandoned" },
    { key: "no_git", label: "No git" },
  ];
  const sort = SORTS.find((s) => s.key === sortKey)!;
  const shown = all
    .filter((p) => filter === "all" || (filter === "attention" ? needsAttention(p) : p.activity === filter))
    .filter((p) => p.name.toLowerCase().includes(query.toLowerCase()))
    .sort(sort.compare);

  return (
    <>
      <section className="panel wide">
        <div className="scan-controls">
          <input
            className="port-input"
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && scan()}
            placeholder="Folder that holds your projects, e.g. ~/Desktop/CODE"
          />
          <button className="port-btn" onClick={scan} disabled={loading}>
            {loading ? "Scanning…" : projects ? "Rescan" : "Scan"}
          </button>
        </div>
        {loading && !projects && (
          <p className="loading">
            Measuring every project's size and git state — a large folder
            takes a little while…
          </p>
        )}
      </section>

      {projects && (
        <>
          <div className="tile-grid">
            <Tile label="Projects" value={String(all.length)} detail={`${counts.active} active this week`} />
            <Tile
              label="Uncommitted work"
              value={String(all.filter((p) => p.dirty_files > 0).length)}
              detail="projects with changed files"
              tone={all.some((p) => p.dirty_files > 0) ? "warn" : "ok"}
            />
            <Tile
              label="Not pushed"
              value={String(all.filter((p) => (p.unpushed_commits ?? 0) > 0).length)}
              detail="projects with local-only commits"
              tone={all.some((p) => (p.unpushed_commits ?? 0) > 0) ? "warn" : "ok"}
            />
            <Tile
              label="Disk used"
              value={bytesToHuman(all.reduce((sum, p) => sum + p.disk_bytes, 0))}
              detail="across all projects"
            />
            <Tile
              label="TODOs"
              value={String(all.reduce((sum, p) => sum + p.todo_count, 0))}
              detail="TODO / FIXME notes"
            />
          </div>

          <div className="range-picker project-toolbar">
            {filters
              .filter((f) => f.key === "all" || counts[f.key] > 0)
              .map((f) => (
                <button
                  key={f.key}
                  className={f.key === filter ? "range-btn active" : "range-btn"}
                  onClick={() => setFilter(f.key)}
                >
                  {f.label} ({counts[f.key]})
                </button>
              ))}
            <span className="log-spacer" />
            <input
              className="port-input small"
              placeholder="Find…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <select
              className="port-input small-select"
              value={sortKey}
              onChange={(e) => setSortKey(e.target.value)}
              aria-label="Sort projects"
            >
              {SORTS.map((s) => (
                <option key={s.key} value={s.key}>
                  {s.label}
                </option>
              ))}
            </select>
          </div>

          <div className="project-grid">
            {shown.map((p) => (
              <ProjectCard key={p.path} p={p} />
            ))}
          </div>
          {shown.length === 0 && (
            <p className="diagnose-hint">No projects match.</p>
          )}
        </>
      )}
    </>
  );
}
