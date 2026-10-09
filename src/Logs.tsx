import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  BarChart,
  Tile,
  bytesToHuman,
  formatDuration,
  usePoll,
} from "./ui";

interface Totals {
  hours_recorded: number;
  cpu_avg: number;
  cpu_max: number;
  ram_avg: number;
  ram_max: number;
  swap_max_bytes: number;
  cpu_stall_avg: number;
  mem_stall_avg: number;
  io_stall_avg: number;
}

interface LogReport {
  kind: Kind;
  offset: number;
  start_ts: number;
  end_ts: number;
  totals: Totals;
  buckets: { start_ts: number; totals: Totals; crashes: number }[];
  apps: {
    app: string;
    running_secs: number;
    focus_secs: number;
    ram_avg_bytes: number;
    ram_peak_bytes: number;
  }[];
  crashes: { ts: number; pid: number; exe: string; signal: number; kind: string }[];
  crash_count: number;
  oom_count: number;
  events: { ts: number; event_type: string; severity: string; message: string }[];
  event_counts: { event_type: string; count: number }[];
  actions: { ts: number; action: string; ok: boolean; message: string }[];
  actions_failed: number;
  boots: { ts: number; total_ms: number; userspace_ms: number }[];
}

type Kind = "day" | "week" | "month" | "year";

const KINDS: { key: Kind; label: string }[] = [
  { key: "day", label: "Daily" },
  { key: "week", label: "Weekly" },
  { key: "month", label: "Monthly" },
  { key: "year", label: "Yearly" },
];

const SIGNALS: Record<number, string> = {
  4: "illegal instruction",
  6: "aborted",
  7: "bus error",
  8: "arithmetic error",
  9: "killed",
  11: "segmentation fault",
};

const EVENT_NAMES: Record<string, string> = {
  ram_increase: "RAM jumps",
  swap_activated: "Swap started",
  cpu_high: "CPU spikes",
  load_high: "Overload",
  disk_growth: "Disk growth",
  vram_increase: "VRAM jumps",
  memory_stall: "Memory stalls",
  io_stall: "Disk stalls",
};

function periodTitle(r: LogReport): string {
  const start = new Date(r.start_ts * 1000);
  const lastDay = new Date((r.end_ts - 1) * 1000);
  const day: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };
  switch (r.kind) {
    case "day":
      return start.toLocaleDateString([], { weekday: "long", ...day, year: "numeric" });
    case "week":
      return `${start.toLocaleDateString([], day)} – ${lastDay.toLocaleDateString([], { ...day, year: "numeric" })}`;
    case "month":
      return start.toLocaleDateString([], { month: "long", year: "numeric" });
    case "year":
      return String(start.getFullYear());
  }
}

function bucketLabel(kind: Kind, ts: number): string {
  const d = new Date(ts * 1000);
  switch (kind) {
    case "day":
      return d.toLocaleTimeString([], { hour: "numeric" });
    case "week":
      return d.toLocaleDateString([], { weekday: "short" });
    case "month":
      return String(d.getDate());
    case "year":
      return d.toLocaleDateString([], { month: "short" });
  }
}

function stamp(ts: number, kind: Kind): string {
  const d = new Date(ts * 1000);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (kind === "day") return time;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

function Limited<T>({
  rows,
  render,
  initial = 8,
}: {
  rows: T[];
  render: (row: T, i: number) => React.ReactNode;
  initial?: number;
}) {
  const [all, setAll] = useState(false);
  const shown = all ? rows : rows.slice(0, initial);
  return (
    <>
      {shown.map(render)}
      {rows.length > shown.length && (
        <button className="ctl-btn" onClick={() => setAll(true)}>
          Show all {rows.length}
        </button>
      )}
    </>
  );
}

export default function Logs() {
  const [kind, setKind] = useState<Kind>("day");
  const [offset, setOffset] = useState(0);
  const [exported, setExported] = useState<string | null>(null);
  const { data: r } = usePoll<LogReport>("get_log_report", { kind, offset }, 60000);

  async function exportReport() {
    try {
      setExported(`✓ Saved to ${await invoke<string>("export_log_report", { kind, offset })}`);
    } catch (e) {
      setExported(`✗ ${e}`);
    }
  }

  const controls = (
    <div className="range-picker log-controls">
      {KINDS.map((k) => (
        <button
          key={k.key}
          className={k.key === kind ? "range-btn active" : "range-btn"}
          onClick={() => {
            setKind(k.key);
            setOffset(0);
            setExported(null);
          }}
        >
          {k.label}
        </button>
      ))}
      <span className="log-spacer" />
      <button className="range-btn" onClick={() => setOffset(offset + 1)}>
        ← Earlier
      </button>
      <span className="log-period">{r && r.kind === kind ? periodTitle(r) : "…"}</span>
      <button className="range-btn" disabled={offset === 0} onClick={() => setOffset(offset - 1)}>
        Later →
      </button>
      <button className="range-btn" onClick={exportReport}>
        Export
      </button>
    </div>
  );

  if (!r || r.kind !== kind) {
    return (
      <>
        {controls}
        <p className="loading">Reading the logbook…</p>
      </>
    );
  }

  const t = r.totals;
  const bars = (pick: (b: Totals) => number) =>
    r.buckets.map((b) => ({
      label: bucketLabel(kind, b.start_ts),
      value: pick(b.totals),
      empty: b.totals.hours_recorded === 0,
    }));
  const focusTotal = r.apps.reduce((sum, a) => sum + a.focus_secs, 0);
  const maxFocus = Math.max(1, ...r.apps.map((a) => a.focus_secs));
  const used = r.apps.filter((a) => a.focus_secs > 0);
  const background = r.apps
    .filter((a) => a.focus_secs === 0)
    .sort((a, b) => b.running_secs - a.running_secs);
  const warnings = r.events.filter((e) => e.severity !== "info").length;
  const empty = t.hours_recorded === 0;

  return (
    <>
      {controls}
      {exported && (
        <p className={exported.startsWith("✓") ? "status-ok" : "status-fail"}>{exported}</p>
      )}

      <div className="tile-grid">
        <Tile
          label="Recorded"
          value={formatDuration(t.hours_recorded * 3600)}
          detail="machine on with Trace running"
        />
        <Tile
          label="Screen time"
          value={formatDuration(focusTotal)}
          detail={used[0] ? `most in ${used[0].app}` : "no focused apps recorded"}
        />
        <Tile label="CPU" value={`${t.cpu_avg.toFixed(0)}%`} detail={`average · peak ${t.cpu_max.toFixed(0)}%`} />
        <Tile
          label="RAM"
          value={`${t.ram_avg.toFixed(0)}%`}
          detail={`average · peak ${t.ram_max.toFixed(0)}%`}
          tone={t.ram_max > 90 ? "danger" : t.ram_max > 75 ? "warn" : undefined}
        />
        <Tile label="Swap peak" value={bytesToHuman(t.swap_max_bytes)} />
        <Tile
          label="Crashes"
          value={String(r.crash_count)}
          detail={r.oom_count > 0 ? `+ ${r.oom_count} killed for memory` : "programs that died"}
          tone={r.crash_count + r.oom_count > 0 ? "warn" : "ok"}
        />
        <Tile
          label="Warnings"
          value={String(warnings)}
          detail="spikes, stalls, swap"
          tone={warnings > 0 ? "warn" : "ok"}
        />
        <Tile
          label="Actions taken"
          value={String(r.actions.length)}
          detail={r.actions_failed > 0 ? `${r.actions_failed} failed` : "from Control and rules"}
        />
      </div>

      {empty && (
        <p className="diagnose-hint">
          Nothing was recorded in this period — Trace only logs while it is
          running.
        </p>
      )}

      <section className="panel">
        <h2>Average CPU</h2>
        <BarChart bars={bars((b) => b.cpu_avg)} color="#3987e5" max={100} format={(v) => `${v.toFixed(0)}%`} />
      </section>
      <section className="panel">
        <h2>Average RAM</h2>
        <BarChart bars={bars((b) => b.ram_avg)} color="#199e70" max={100} format={(v) => `${v.toFixed(0)}%`} />
      </section>
      <section className="panel">
        <h2>Time Recorded</h2>
        <BarChart
          bars={bars((b) => b.hours_recorded)}
          color="#9085e9"
          format={(v) => formatDuration(v * 3600)}
        />
      </section>
      <section className="panel">
        <h2>Crashes</h2>
        <BarChart
          bars={r.buckets.map((b) => ({ label: bucketLabel(kind, b.start_ts), value: b.crashes }))}
          color="#d95926"
          format={(v) => v.toFixed(0)}
        />
      </section>

      <section className="panel wide">
        <h2>Apps You Used ({used.length})</h2>
        {used.length === 0 ? (
          <p className="diagnose-hint">
            No foreground time recorded for this period. Screen time is
            tracked from the focused window (Hyprland) from now on.
          </p>
        ) : (
          <div className="folder-list">
            {used.slice(0, 15).map((a) => (
              <div key={a.app} className="mem-app-row mem-static-row">
                <span className="folder-name">{a.app}</span>
                <div className="folder-bar-track">
                  <div
                    className="mem-bar-seg"
                    style={{ width: `${(a.focus_secs / maxFocus) * 100}%`, background: "#3987e5" }}
                  />
                </div>
                <span className="folder-size">
                  {formatDuration(a.focus_secs)}
                  <span className="mem-muted"> · avg {bytesToHuman(a.ram_avg_bytes)}</span>
                </span>
              </div>
            ))}
          </div>
        )}
        <p className="diagnose-hint">
          Time each app's window was in focus. Trace can't tell whether you
          were at the keyboard, so a window left focused overnight still counts.
        </p>
      </section>

      <section className="panel">
        <h2>Also Running ({background.length})</h2>
        {background.length === 0 && <p className="diagnose-hint">Nothing recorded.</p>}
        <Limited
          rows={background}
          initial={10}
          render={(a) => (
            <div key={a.app} className="meta-row">
              <span>{a.app}</span>
              <span>
                {formatDuration(a.running_secs)} · peak {bytesToHuman(a.ram_peak_bytes)}
              </span>
            </div>
          )}
        />
        <p className="diagnose-hint">
          Ran in the background without ever being the focused window.
        </p>
      </section>

      <section className="panel">
        <h2>Crash Log ({r.crashes.length})</h2>
        {r.crashes.length === 0 && <p className="status-ok">✓ No program crashed.</p>}
        <Limited
          rows={r.crashes}
          render={(c, i) => (
            <div key={i} className="event-row">
              <span className="event-time">{stamp(c.ts, kind)}</span>
              <span className="event-message">
                <strong>{c.exe}</strong>{" "}
                {c.kind === "oom"
                  ? "was killed because memory ran out"
                  : `crashed (${SIGNALS[c.signal] ?? `signal ${c.signal}`})`}
              </span>
            </div>
          )}
        />
        <p className="diagnose-hint">
          Copied from the system's crash records so they stay here after the
          journal rotates them away.
        </p>
      </section>

      <section className="panel">
        <h2>Warnings &amp; Events ({r.events.length})</h2>
        <div className="chip-row">
          {r.event_counts.map((e) => (
            <span key={e.event_type} className="chip">
              {EVENT_NAMES[e.event_type] ?? e.event_type} ×{e.count}
            </span>
          ))}
        </div>
        {r.events.length === 0 && <p className="status-ok">✓ Nothing unusual happened.</p>}
        <Limited
          rows={r.events}
          render={(e, i) => (
            <div key={i} className="event-row">
              <span className="event-time">{stamp(e.ts, kind)}</span>
              <span className="event-message">{e.message}</span>
            </div>
          )}
        />
      </section>

      <section className="panel">
        <h2>Actions &amp; Boots</h2>
        {r.boots.map((b) => (
          <div key={b.ts} className="event-row">
            <span className="event-time">{stamp(b.ts, kind)}</span>
            <span className="event-message">
              Booted in {(b.total_ms / 1000).toFixed(1)} s
            </span>
          </div>
        ))}
        {r.actions.length + r.boots.length === 0 && (
          <p className="diagnose-hint">No boots or control actions in this period.</p>
        )}
        <Limited
          rows={r.actions}
          render={(a, i) => (
            <div key={i} className="event-row">
              <span className="event-time">{stamp(a.ts, kind)}</span>
              <span className="event-message">
                <span className={a.ok ? "status-ok" : "status-fail"}>{a.ok ? "✓" : "✗"}</span>{" "}
                {a.action}: {a.message}
              </span>
            </div>
          )}
        />
      </section>
    </>
  );
}
