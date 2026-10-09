import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ActionResult,
  ConfirmButton,
  Sparkline,
  StatusLine,
  bytesToHuman,
  formatClock,
  formatDay,
  formatDuration,
  useAction,
  usePoll,
} from "./ui";
import { ChargeOnce } from "./Batch5";

/** Tests link, router, internet and name lookups in turn and says where it breaks. */
export function ConnectionDoctor() {
  const [result, setResult] = useState<{ verdict: string; checks: { name: string; ok: boolean; detail: string }[] } | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    try {
      setResult(await invoke("run_connection_doctor"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel">
      <h2>Connection Doctor</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? "Testing… (about 5 seconds)" : result ? "Test again" : "Is it my Wi-Fi, my router or my provider?"}
        </button>
      </div>
      {result && (
        <>
          <div className="explain">
            <strong>{result.verdict}</strong>
          </div>
          {result.checks.map((c) => (
            <div key={c.name} className="meta-row">
              <span className={c.ok ? "status-ok" : "status-fail"}>
                {c.ok ? "✓" : "✗"} {c.name}
              </span>
              <span>{c.detail}</span>
            </div>
          ))}
        </>
      )}
    </section>
  );
}

interface DataUsage {
  iface: string;
  used_bytes: number;
  budget_bytes: number | null;
}

/** Data used this month per network socket, with an optional allowance. */
export function DataBudgets() {
  const usage = usePoll<DataUsage[]>("get_data_usage", undefined, 60000);
  const config = usePoll<{ data_budgets: { iface: string; monthly_gb: number }[] } & Record<string, unknown>>("get_automation");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  if (!usage.data || !config.data) return null;

  async function setBudget(iface: string, text: string) {
    const gb = parseFloat(text);
    const others = (config.data!.data_budgets ?? []).filter((b) => b.iface !== iface);
    const data_budgets = gb > 0 ? [...others, { iface, monthly_gb: gb }] : others;
    await invoke("save_automation", { config: { ...config.data, data_budgets } });
    config.reload();
    usage.reload();
  }

  return (
    <section className="panel">
      <h2>Data Used This Month</h2>
      {usage.data.length === 0 && <p className="diagnose-hint">Nothing recorded yet this month.</p>}
      {usage.data.map((u) => {
        const share = u.budget_bytes ? Math.min(100, (u.used_bytes / u.budget_bytes) * 100) : 0;
        return (
          <div key={u.iface}>
            <div className="control-row">
              <label>
                <strong>{u.iface}</strong> — {bytesToHuman(u.used_bytes)}
                {u.budget_bytes ? ` of ${(u.budget_bytes / 1e9).toFixed(0)} GB (${share.toFixed(0)}%)` : ""}
              </label>
              <input
                className="port-input number-box"
                placeholder="GB"
                value={drafts[u.iface] ?? ""}
                aria-label={`Monthly allowance for ${u.iface} in GB`}
                onChange={(e) => setDrafts({ ...drafts, [u.iface]: e.target.value.replace(/[^\d.]/g, "") })}
              />
              <button className="ctl-btn" onClick={() => setBudget(u.iface, drafts[u.iface] ?? "0")}>
                {u.budget_bytes ? (drafts[u.iface] ? "Change" : "Remove") : "Set allowance"}
              </button>
            </div>
            {u.budget_bytes != null && (
              <div className="bar-track">
                <div
                  className={`bar-fill ${share >= 100 ? "bar-danger" : share >= 80 ? "bar-warn" : "bar-ok"}`}
                  style={{ width: `${share}%` }}
                />
              </div>
            )}
          </div>
        );
      })}
      <p className="diagnose-hint">
        Set a monthly allowance in GB for a connection (a phone hotspot, a capped plan) and Trace
        warns you at 80% and at 100%. Only counted while Trace is running.
      </p>
    </section>
  );
}

interface Listener {
  port: number;
  exposed: boolean;
  protocol: string;
  pid: number;
  process: string;
  folder: string;
}

/** Every port a program is listening on, and the project folder it runs from. */
export function ListeningPorts() {
  const { data, reload } = usePoll<Listener[]>("list_listeners", undefined, 10000);
  const { result, busy, run } = useAction(reload);
  const [filter, setFilter] = useState("");
  const home = (path: string) => path.replace(/^\/home\/[^/]+/, "~");
  const rows = (data ?? []).filter((l) => `${l.port} ${l.process} ${l.folder}`.toLowerCase().includes(filter.toLowerCase()));
  return (
    <section className="panel">
      <h2>Listening Ports ({data?.length ?? 0})</h2>
      <div className="proc-picker">
        <input className="port-input" placeholder="Filter by port, program or folder…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      <div className="unit-list">
        {rows.map((l) => (
          <div key={`${l.protocol}${l.port}${l.pid}`} className="unit-row" title={l.folder}>
            <span className="unit-name">
              <strong>{l.port}</strong> {l.process}{" "}
              <span className="mem-muted">
                {l.protocol}
                {l.folder && l.folder !== "/" ? ` · ${home(l.folder)}` : ""}
              </span>
            </span>
            <span className={l.exposed ? "unit-state status-fail" : "unit-state"}>
              {l.exposed ? "open to the network" : "this machine only"}
            </span>
            <div className="unit-buttons">
              {l.pid > 0 && (
                <ConfirmButton
                  label="Stop"
                  confirmLabel="Confirm"
                  className="ctl-btn-sm ctl-danger"
                  disabled={busy}
                  onConfirm={() => run("kill_process", { pid: l.pid, force: false })}
                />
              )}
            </div>
          </div>
        ))}
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        "Open to the network" means other machines can reach it unless the firewall blocks that
        port. The folder is where the program was started — usually the project a dev server
        belongs to.
      </p>
    </section>
  );
}

interface JunkDir {
  path: string;
  name: string;
  what: string;
  bytes: number;
}

/** Rebuildable folders inside one project, with a button to delete them. */
export function ProjectJunk({ project }: { project: string }) {
  const [junk, setJunk] = useState<JunkDir[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<ActionResult | null>(null);

  async function look() {
    setBusy(true);
    try {
      setJunk(await invoke<JunkDir[]>("get_project_junk", { project }));
    } finally {
      setBusy(false);
    }
  }
  async function clean() {
    if (!junk) return;
    setBusy(true);
    try {
      setMessage(await invoke<ActionResult>("clean_project_junk", { project, paths: junk.map((j) => j.path) }));
      setJunk(null);
    } finally {
      setBusy(false);
    }
  }
  const total = (junk ?? []).reduce((sum, j) => sum + j.bytes, 0);

  return (
    <div>
      {junk === null ? (
        <button className="ctl-btn ctl-btn-sm" onClick={look} disabled={busy}>
          {busy ? "Measuring…" : "Find build files"}
        </button>
      ) : junk.length === 0 ? (
        <span className="mem-muted">Nothing rebuildable to clear.</span>
      ) : (
        <>
          {junk.map((j) => (
            <div key={j.path} className="preset-change" title={j.path}>
              <span>
                {j.path.slice(project.length + 1)} <span className="mem-muted">{j.what}</span>
              </span>
              <span>{bytesToHuman(j.bytes)}</span>
            </div>
          ))}
          <ConfirmButton
            label={`Delete these — frees ${bytesToHuman(total)}`}
            confirmLabel="They are rebuilt on the next install or build — confirm"
            className="ctl-btn ctl-btn-sm ctl-danger"
            disabled={busy}
            onConfirm={clean}
          />
        </>
      )}
      {message && <p className={message.ok ? "status-ok" : "status-fail"}>{message.ok ? "✓" : "✗"} {message.message}</p>}
    </div>
  );
}

interface BatteryCare {
  sleeps: { start_ts: number; end_ts: number; percent_before: number; percent_after: number; on_battery: boolean }[];
  capacity: { day: string; full_wh: number; cycle_count: number }[];
}

/** How the battery's capacity is ageing, and what each sleep cost. */
export function BatteryCareCard() {
  const { data } = usePoll<BatteryCare>("get_battery_care");
  if (!data || (data.capacity.length === 0 && data.sleeps.length === 0)) return null;
  const first = data.capacity[0];
  const last = data.capacity[data.capacity.length - 1];
  return (
    <section className="panel">
      <h2>Battery Over Time</h2>
      {last && (
        <div className="mem-stats">
          <div className="mem-stat">
            <span className="mem-stat-value">{last.full_wh.toFixed(1)} Wh</span>
            <span className="mem-stat-label">
              full charge today
              {first && first.day !== last.day && ` · ${first.full_wh.toFixed(1)} Wh on ${first.day}`}
            </span>
          </div>
          {data.capacity.length > 1 && (
            <div className="mem-stat">
              <Sparkline values={data.capacity.map((c) => c.full_wh)} color="#199e70" width={180} />
              <span className="mem-stat-label">
                capacity over {data.capacity.length} days · {last.cycle_count - first.cycle_count} cycles added
              </span>
            </div>
          )}
        </div>
      )}
      <ChargeOnce />
      <h3 className="sub-heading">Recent sleeps</h3>
      {data.sleeps.length === 0 && (
        <p className="diagnose-hint">None recorded yet. Each time the laptop sleeps, Trace notes how long and how much charge it cost.</p>
      )}
      {data.sleeps.slice(0, 8).map((s) => {
        const hours = (s.end_ts - s.start_ts) / 3600;
        const lost = s.percent_before - s.percent_after;
        return (
          <div key={s.start_ts} className="meta-row">
            <span>
              {formatDay(s.start_ts)} {formatClock(s.start_ts)} · {formatDuration(s.end_ts - s.start_ts)}
            </span>
            <span className={s.on_battery && lost / Math.max(hours, 0.1) > 2 ? "status-fail" : ""}>
              {s.on_battery
                ? lost > 0.5
                  ? `lost ${lost.toFixed(0)}% (${(lost / Math.max(hours, 0.1)).toFixed(1)}% an hour)`
                  : "lost nothing"
                : `on the charger (${s.percent_after.toFixed(0)}% after)`}
            </span>
          </div>
        );
      })}
      <p className="diagnose-hint">
        A healthy sleep costs under 1% an hour. Much more usually means something kept part of the
        machine awake.
      </p>
    </section>
  );
}
