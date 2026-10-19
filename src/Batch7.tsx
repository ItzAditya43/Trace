import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ConfirmButton, StatusLine, bytesToHuman, formatClock, formatDay, formatDuration, useAction, usePoll } from "./ui";

interface ProcessDetail {
  pid: number;
  name: string;
  command: string;
  exe: string;
  folder: string;
  user: string;
  state: string;
  threads: number;
  started_secs_ago: number;
  ram_bytes: number;
  unit: string;
  parents: [number, string][];
  children: [number, string][];
  open_files: string[];
  open_file_count: number;
  connections: string[];
  environment: string[];
}

/** Everything about one process: what started it, what it has open, who it talks to. */
export function ProcessDetails() {
  const [pid, setPid] = useState("");
  const [detail, setDetail] = useState<ProcessDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState("files");
  async function look(p = pid) {
    setError(null);
    try {
      setDetail(await invoke("get_process_detail", { pid: parseInt(p) }));
    } catch (e) {
      setError(String(e));
      setDetail(null);
    }
  }
  return (
    <section className="panel wide">
      <h2>Process Details</h2>
      <div className="control-actions">
        <input className="port-input number-box" placeholder="PID" value={pid} onChange={(e) => setPid(e.target.value.replace(/\D/g, ""))} onKeyDown={(e) => e.key === "Enter" && look()} />
        <button className="ctl-btn ctl-primary" disabled={!pid} onClick={() => look()}>
          Look inside
        </button>
        <span className="mem-muted">The PID is in the Processes list above, or in Top Processes on the Overview.</span>
      </div>
      {error && <p className="status-fail">✗ {error}</p>}
      {detail && (
        <>
          <div className="meta-row">
            <strong>
              {detail.name} <span className="mem-muted">PID {detail.pid}</span>
            </strong>
            <span>
              {detail.user} · {detail.state} · {detail.threads} threads · {bytesToHuman(detail.ram_bytes)} · running {formatDuration(detail.started_secs_ago)}
            </span>
          </div>
          <div className="plain-row">
            <span className="mem-muted">Command</span> <code>{detail.command || detail.exe}</code>
          </div>
          <div className="plain-row">
            <span className="mem-muted">Started in</span> {detail.folder || "—"} <span className="mem-muted">· belongs to</span> {detail.unit || "—"}
          </div>
          <div className="plain-row">
            <span className="mem-muted">Why it is running:</span>{" "}
            {[...detail.parents].reverse().map(([p, n]) => (
              <button key={p} className="link-button" onClick={() => { setPid(String(p)); look(String(p)); }}>
                {n} ({p})
              </button>
            )).reduce<React.ReactNode[]>((acc, el) => (acc.length ? [...acc, " → ", el] : [el]), [])}
            {" → "}
            <strong>{detail.name}</strong>
          </div>
          {detail.children.length > 0 && (
            <div className="plain-row">
              <span className="mem-muted">Started by it:</span>{" "}
              {detail.children.map(([p, n]) => (
                <button key={p} className="link-button" onClick={() => { setPid(String(p)); look(String(p)); }}>
                  {n} ({p}){" "}
                </button>
              ))}
            </div>
          )}
          <div className="chip-row">
            {[
              ["files", `Open files (${detail.open_file_count})`],
              ["net", `Connections (${detail.connections.length})`],
              ["env", `Environment (${detail.environment.length})`],
            ].map(([id, label]) => (
              <button key={id} className={`chip chip-button${tab === id ? " chip-on" : ""}`} onClick={() => setTab(id)}>
                {label}
              </button>
            ))}
          </div>
          <pre className="log-box">
            {(tab === "files" ? detail.open_files : tab === "net" ? detail.connections : detail.environment).join("\n") ||
              (tab === "env" ? "(only readable for your own processes)" : "(none)")}
          </pre>
        </>
      )}
    </section>
  );
}

interface ChangedSetting {
  id: string;
  title: string;
  now: string;
  original: string;
}

/** One button that undoes every setting Trace has changed on this machine. */
export function PutEverythingBack() {
  const changed = usePoll<ChangedSetting[]>("get_changed_settings", undefined, 30000);
  const { result, busy, run } = useAction(() => changed.reload());
  if (!changed.data) return null;
  return (
    <section className="panel">
      <h2>Put Everything Back</h2>
      {changed.data.length === 0 ? (
        <p className="diagnose-hint">✓ Every setting is as Trace first found it on this machine.</p>
      ) : (
        <>
          <p className="diagnose-hint">
            {changed.data.length} setting(s) differ from what Trace first saw. This sets all of them back at once (one
            password prompt) and stops keeping them after a restart.
          </p>
          <div className="event-scroll">
            {changed.data.map((c) => (
              <div key={c.id} className="plain-row">
                {c.title} <span className="mem-muted">now {c.now} → back to {c.original}</span>
              </div>
            ))}
          </div>
          <div className="control-actions">
            <ConfirmButton label="Put all of them back" confirmLabel="Undo every change?" disabled={busy} onConfirm={() => run("put_everything_back")} />
          </div>
        </>
      )}
      <StatusLine result={result} />
    </section>
  );
}

interface GameSession {
  ts: number;
  game: string;
  minutes: number;
  avg_fps: number;
  low_fps: number;
}

/** Frame rate per game session, from the MangoHud feed. */
export function GameHistory() {
  const { data } = usePoll<GameSession[]>("get_game_sessions", undefined, 300000);
  if (!data || data.length === 0) return null;
  return (
    <section className="panel">
      <h2>Games Played</h2>
      <table className="proc-table">
        <thead>
          <tr>
            <th>Game</th>
            <th>When</th>
            <th>Played</th>
            <th>Average</th>
            <th>1% low</th>
          </tr>
        </thead>
        <tbody>
          {data.map((g) => (
            <tr key={g.ts + g.game}>
              <td>{g.game}</td>
              <td>
                {formatDay(g.ts)} {formatClock(g.ts)}
              </td>
              <td>{g.minutes < 1 ? "<1 min" : `${g.minutes.toFixed(0)} min`}</td>
              <td>{g.avg_fps.toFixed(0)} FPS</td>
              <td className={g.low_fps < g.avg_fps * 0.5 ? "status-warn" : ""}>{g.low_fps.toFixed(0)} FPS</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="diagnose-hint">
        Recorded for games started with the frame-rate launch option (Stats Overlay card). A 1% low far under the
        average means stutter, even when the average looks fine.
      </p>
    </section>
  );
}

/** A self-contained page describing this machine's health, to send when asking for help. */
export function ShareReport() {
  const { result, busy, run } = useAction();
  return (
    <section className="panel">
      <h2>Share A Health Report</h2>
      <p className="diagnose-hint">
        One page with the score, the issues, the hardware, failed services and recent errors. Handy when asking someone
        for help. It is saved to your Downloads folder and goes nowhere by itself.
      </p>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" disabled={busy} onClick={() => run("export_health_report")}>
          {busy ? "Writing…" : "Save the report"}
        </button>
      </div>
      <StatusLine result={result} />
    </section>
  );
}

/** ntfy: alerts on any phone or computer, through a topic only you know. */
export function NtfySetting({ value, onSave }: { value: string; onSave: (url: string) => void }) {
  const [url, setUrl] = useState(value);
  const { result, busy, run } = useAction();
  return (
    <>
      <div className="control-row">
        <label>
          Also send alerts to ntfy
          <span className="mem-muted"> — any phone or computer with the ntfy app; pick a topic name nobody would guess</span>
        </label>
      </div>
      <div className="control-actions">
        <input className="port-input grow-input" placeholder="https://ntfy.sh/your-private-topic" value={url} onChange={(e) => setUrl(e.target.value.trim())} />
        <button className="ctl-btn" disabled={busy || !url} onClick={() => run("test_ntfy", { url })}>
          Test
        </button>
        <button className="ctl-btn" disabled={url === value} onClick={() => onSave(url)}>
          {url ? "Save" : "Turn off"}
        </button>
      </div>
      <StatusLine result={result} />
    </>
  );
}
