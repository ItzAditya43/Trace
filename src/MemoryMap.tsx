import { useState } from "react";
import {
  ConfirmButton,
  Sparkline,
  StatusLine,
  bytesToHuman,
  useAction,
  usePoll,
} from "./ui";

interface MemoryCategory {
  key: string;
  label: string;
  bytes: number;
  note: string;
}

interface SwapDevice {
  name: string;
  kind: string;
  total_bytes: number;
  used_bytes: number;
  priority: number;
}

interface ProcessMemory {
  pid: number;
  name: string;
  ram_bytes: number;
  swap_bytes: number;
}

interface AppMemory {
  name: string;
  process_count: number;
  ram_bytes: number;
  anon_bytes: number;
  file_bytes: number;
  shmem_bytes: number;
  swap_bytes: number;
  approximate: boolean;
  processes: ProcessMemory[];
}

interface MemoryBreakdown {
  total_bytes: number;
  available_bytes: number;
  categories: MemoryCategory[];
  swap_total_bytes: number;
  swap_used_bytes: number;
  swap_devices: SwapDevice[];
  compressed: { original_bytes: number; ram_bytes: number };
  pressure_some_avg10: number | null;
  pressure_some_avg60: number | null;
  apps: AppMemory[];
}

// Fixed colour per category so a segment keeps its colour as sizes change.
const CATEGORY_COLORS: Record<string, string> = {
  apps: "#3987e5",
  compressed: "#d95926",
  cache: "#199e70",
  kernel: "#c98500",
  graphics: "#d55181",
  shared: "#008300",
  other: "#5a6072",
  free: "#262a36",
};

const RAM_COLOR = "#3987e5";
const SWAP_COLOR = "#5a6072";
const TOP_APPS = 12;

const GROWTH_RANGES = [
  { label: "1h", secs: 3600 },
  { label: "6h", secs: 6 * 3600 },
  { label: "24h", secs: 24 * 3600 },
  { label: "7d", secs: 7 * 24 * 3600 },
];

interface AppMemoryPoint {
  ts: number;
  app: string;
  ram_bytes: number;
  swap_bytes: number;
}

/** Quit / freeze / restart / cap controls for one app row. */
function AppActions({ app, onDone }: { app: AppMemory; onDone: () => void }) {
  const { result, busy, run } = useAction(onDone);
  const [capMb, setCapMb] = useState(
    String(Math.max(256, Math.round(app.ram_bytes / 1024 / 1024 / 256) * 256))
  );
  const act = (action: string) => run("app_action", { name: app.name, action });

  return (
    <div className="mem-actions">
      <div className="control-actions">
        <ConfirmButton
          label="Restart app"
          confirmLabel="Unsaved work is lost — confirm"
          className="ctl-btn"
          disabled={busy}
          onConfirm={() => act("restart")}
        />
        <button className="ctl-btn" disabled={busy} onClick={() => act("freeze")}>
          Freeze
        </button>
        <button className="ctl-btn" disabled={busy} onClick={() => act("resume")}>
          Resume
        </button>
        <ConfirmButton label="Quit" disabled={busy} onConfirm={() => act("kill")} />
        <ConfirmButton
          label="Force kill"
          disabled={busy}
          onConfirm={() => act("force_kill")}
        />
        <span className="mem-cap">
          <input
            className="port-input small"
            value={capMb}
            onChange={(e) => setCapMb(e.target.value)}
            aria-label="Memory cap in MB"
          />
          <button
            className="ctl-btn"
            disabled={busy || !(parseInt(capMb, 10) > 0)}
            onClick={() =>
              run("limit_app_memory", { name: app.name, mb: parseInt(capMb, 10) })
            }
          >
            Cap at MB
          </button>
        </span>
      </div>
      <StatusLine result={result} />
    </div>
  );
}

function Growth() {
  const [rangeSecs, setRangeSecs] = useState(GROWTH_RANGES[1].secs);
  const { data } = usePoll<AppMemoryPoint[]>(
    "get_app_memory_history",
    { sinceSecsAgo: rangeSecs },
    60000
  );

  const byApp = new Map<string, AppMemoryPoint[]>();
  for (const p of data ?? []) {
    const arr = byApp.get(p.app) ?? [];
    arr.push(p);
    byApp.set(p.app, arr);
  }
  const rows = [...byApp.entries()]
    .filter(([, points]) => points.length >= 2)
    .map(([app, points]) => {
      const totals = points.map((p) => p.ram_bytes + p.swap_bytes);
      return {
        app,
        totals,
        first: totals[0],
        last: totals[totals.length - 1],
        delta: totals[totals.length - 1] - totals[0],
      };
    })
    .sort((a, b) => b.delta - a.delta)
    .slice(0, 10);

  return (
    <section className="panel">
      <h2>Growth Over Time</h2>
      <div className="range-picker">
        {GROWTH_RANGES.map((r) => (
          <button
            key={r.label}
            className={r.secs === rangeSecs ? "range-btn active" : "range-btn"}
            onClick={() => setRangeSecs(r.secs)}
          >
            {r.label}
          </button>
        ))}
      </div>
      {rows.length === 0 ? (
        <p className="diagnose-hint">
          Not enough history yet — Trace records each app's memory once a
          minute while it is running.
        </p>
      ) : (
        <table className="proc-table">
          <thead>
            <tr>
              <th>App</th>
              <th>Trend (RAM + swap)</th>
              <th>Then</th>
              <th>Now</th>
              <th>Change</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.app}>
                <td>{r.app}</td>
                <td>
                  <Sparkline values={r.totals} color={RAM_COLOR} />
                </td>
                <td>{bytesToHuman(r.first)}</td>
                <td>{bytesToHuman(r.last)}</td>
                <td className={r.delta > 256 * 1024 * 1024 ? "status-fail" : ""}>
                  {r.delta >= 0 ? "▲ +" : "▼ −"}
                  {bytesToHuman(Math.abs(r.delta))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="diagnose-hint">
        Sorted by how much each app grew over the range. An app that keeps
        climbing without levelling off is the usual sign of a leak.
      </p>
    </section>
  );
}

function Distribution({ data }: { data: MemoryBreakdown }) {
  const [hovered, setHovered] = useState<string | null>(null);
  const shown = data.categories.filter((c) => c.bytes > 0);
  const inUse = data.total_bytes - data.available_bytes;

  return (
    <section className="panel wide">
      <h2>Where Your RAM Is</h2>
      <div className="mem-stats">
        <div className="mem-stat">
          <span className="mem-stat-value">{bytesToHuman(inUse)}</span>
          <span className="mem-stat-label">
            in use of {bytesToHuman(data.total_bytes)}
          </span>
        </div>
        <div className="mem-stat">
          <span className="mem-stat-value">{bytesToHuman(data.available_bytes)}</span>
          <span className="mem-stat-label">available to apps right now</span>
        </div>
        <div className="mem-stat">
          <span className="mem-stat-value">{bytesToHuman(data.swap_used_bytes)}</span>
          <span className="mem-stat-label">
            pushed to swap of {bytesToHuman(data.swap_total_bytes)}
          </span>
        </div>
        {data.pressure_some_avg60 !== null && (
          <div className="mem-stat">
            <span className="mem-stat-value">
              {data.pressure_some_avg60.toFixed(1)}%
            </span>
            <span className="mem-stat-label">
              of the last minute spent stalled on memory
            </span>
          </div>
        )}
      </div>

      <div className="mem-stack" onMouseLeave={() => setHovered(null)}>
        {shown.map((c) => (
          <div
            key={c.key}
            className={
              "mem-stack-seg" +
              (hovered && hovered !== c.key ? " mem-dim" : "")
            }
            style={{
              flexGrow: c.bytes,
              background: CATEGORY_COLORS[c.key],
            }}
            title={`${c.label}: ${bytesToHuman(c.bytes)} — ${c.note}`}
            onMouseEnter={() => setHovered(c.key)}
          />
        ))}
      </div>

      <div className="mem-legend" onMouseLeave={() => setHovered(null)}>
        {shown.map((c) => (
          <div
            key={c.key}
            className={
              "mem-legend-row" + (hovered === c.key ? " mem-legend-active" : "")
            }
            onMouseEnter={() => setHovered(c.key)}
          >
            <span
              className="pie-swatch"
              style={{ background: CATEGORY_COLORS[c.key] }}
            />
            <span className="mem-legend-label">{c.label}</span>
            <span className="mem-legend-value">{bytesToHuman(c.bytes)}</span>
            <span className="mem-legend-pct">
              {((c.bytes / data.total_bytes) * 100).toFixed(0)}%
            </span>
            <span className="mem-legend-note">{c.note}</span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">
        "Available" is larger than "Free" because file cache is given back the
        moment an app asks for memory.
      </p>
    </section>
  );
}

function Swap({ data }: { data: MemoryBreakdown }) {
  if (data.swap_total_bytes === 0) return null;
  const { original_bytes, ram_bytes } = data.compressed;
  return (
    <section className="panel">
      <h2>Swap — Where Pushed-Out Memory Lives</h2>
      {data.swap_devices.map((d) => (
        <div key={d.name} className="mem-app-row mem-static-row">
          <span className="folder-name" title={d.name}>
            {d.name.replace("/dev/", "")} <span className="mem-muted">({d.kind})</span>
          </span>
          <div className="folder-bar-track">
            <div
              className="mem-bar-seg"
              style={{
                width: `${(d.used_bytes / d.total_bytes) * 100}%`,
                background: d.kind === "compressed RAM" ? CATEGORY_COLORS.compressed : SWAP_COLOR,
              }}
            />
          </div>
          <span className="folder-size">
            {bytesToHuman(d.used_bytes)} / {bytesToHuman(d.total_bytes)}
          </span>
        </div>
      ))}
      {ram_bytes > 0 && (
        <p className="diagnose-hint">
          {bytesToHuman(original_bytes)} of swapped data is compressed into{" "}
          {bytesToHuman(ram_bytes)} of RAM (
          {`${(original_bytes / ram_bytes).toFixed(1)}×`}) — it never touched
          the disk, but it still occupies the "Compressed swap" slice above.
        </p>
      )}
    </section>
  );
}

function Apps({ data, onChange }: { data: MemoryBreakdown; onChange: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);

  const apps = showAll ? data.apps : data.apps.slice(0, TOP_APPS);
  const rest = showAll ? [] : data.apps.slice(TOP_APPS);
  const restRam = rest.reduce((sum, a) => sum + a.ram_bytes, 0);
  const restSwap = rest.reduce((sum, a) => sum + a.swap_bytes, 0);
  const max = Math.max(
    1,
    restRam + restSwap,
    ...data.apps.slice(0, 1).map((a) => a.ram_bytes + a.swap_bytes)
  );

  const bar = (ram: number, swap: number) => (
    <div className="folder-bar-track mem-bar-track">
      <div
        className="mem-bar-seg"
        style={{ width: `${(ram / max) * 100}%`, background: RAM_COLOR }}
      />
      {swap > 0 && (
        <div
          className="mem-bar-seg"
          style={{ width: `${(swap / max) * 100}%`, background: SWAP_COLOR }}
        />
      )}
    </div>
  );

  const size = (ram: number, swap: number) => (
    <span className="folder-size mem-app-size">
      {bytesToHuman(ram)}
      {swap > 0 && <span className="mem-muted"> + {bytesToHuman(swap)} swap</span>}
    </span>
  );

  return (
    <section className="panel wide">
      <h2>Memory by App</h2>
      <div className="mem-key">
        <span>
          <span className="pie-swatch" style={{ background: RAM_COLOR }} /> in RAM
        </span>
        <span>
          <span className="pie-swatch" style={{ background: SWAP_COLOR }} /> pushed
          out to swap
        </span>
      </div>
      <div className="folder-list">
        {apps.map((a) => (
          <div key={a.name}>
            <button
              className="folder-row mem-app-row"
              onClick={() => setExpanded(expanded === a.name ? null : a.name)}
              title={
                `Private ${bytesToHuman(a.anon_bytes)} · files ${bytesToHuman(a.file_bytes)}` +
                ` · shared ${bytesToHuman(a.shmem_bytes)}` +
                (a.approximate ? " (approximate — owned by another user)" : "")
              }
            >
              <span className="folder-name">
                {a.name}
                {a.process_count > 1 && (
                  <span className="mem-muted"> ({a.process_count} processes)</span>
                )}
              </span>
              {bar(a.ram_bytes, a.swap_bytes)}
              {size(a.ram_bytes, a.swap_bytes)}
            </button>
            {expanded === a.name && (
              <div className="mem-expanded">
              <AppActions app={a} onDone={onChange} />
              <table className="proc-table mem-proc-table">
                <thead>
                  <tr>
                    <th>PID</th>
                    <th>In RAM</th>
                    <th>In swap</th>
                  </tr>
                </thead>
                <tbody>
                  {a.processes.map((p) => (
                    <tr key={p.pid}>
                      <td>{p.pid}</td>
                      <td>{bytesToHuman(p.ram_bytes)}</td>
                      <td>{p.swap_bytes > 0 ? bytesToHuman(p.swap_bytes) : "—"}</td>
                    </tr>
                  ))}
                  {a.process_count > a.processes.length && (
                    <tr>
                      <td colSpan={3}>
                        …and {a.process_count - a.processes.length} smaller ones
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
              </div>
            )}
          </div>
        ))}
        {rest.length > 0 && (
          <button className="folder-row mem-app-row" onClick={() => setShowAll(true)}>
            <span className="folder-name">
              Everything else{" "}
              <span className="mem-muted">({rest.length} apps — show)</span>
            </span>
            {bar(restRam, restSwap)}
            {size(restRam, restSwap)}
          </button>
        )}
      </div>
      <p className="diagnose-hint">
        Shared pages are split fairly between the processes using them, so
        these rows add up without double counting. Click an app to see its
        processes and to restart, freeze, quit or cap it; hover for the
        private / file / shared split.
      </p>
    </section>
  );
}

export default function MemoryMap() {
  const { data, error, reload } = usePoll<MemoryBreakdown>(
    "get_memory_breakdown",
    undefined,
    5000
  );

  if (error) return <p className="status-fail">✗ {error}</p>;
  if (!data) return <p className="loading">Accounting for memory…</p>;

  return (
    <>
      <Distribution data={data} />
      <Apps data={data} onChange={reload} />
      <Growth />
      <Swap data={data} />
    </>
  );
}
