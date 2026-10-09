import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ActionResult {
  ok: boolean;
  message: string;
}

export function bytesToHuman(bytes: number): string {
  const gb = bytes / 1024 / 1024 / 1024;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
}

export function formatRate(bytesPerSec: number): string {
  if (bytesPerSec >= 1024 * 1024) {
    return `${(bytesPerSec / 1024 / 1024).toFixed(1)} MB/s`;
  }
  if (bytesPerSec >= 1024) return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
  return bytesPerSec > 0 ? `${bytesPerSec.toFixed(0)} B/s` : "—";
}

export function formatClock(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatDay(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString([], {
    month: "short",
    day: "numeric",
  });
}

/**
 * Fetches a backend command, optionally re-polling. `reload` re-fetches on
 * demand (after an action changed what the command reports).
 */
export function usePoll<T>(
  command: string,
  args?: Record<string, unknown>,
  intervalMs?: number
): { data: T | null; error: string | null; loading: boolean; reload: () => void } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const alive = useRef(true);
  const argsKey = JSON.stringify(args ?? {});

  const reload = useCallback(() => {
    setLoading(true);
    invoke<T>(command, JSON.parse(argsKey))
      .then((d) => {
        if (!alive.current) return;
        setData(d);
        setError(null);
      })
      .catch((e) => alive.current && setError(String(e)))
      .finally(() => alive.current && setLoading(false));
  }, [command, argsKey]);

  useEffect(() => {
    alive.current = true;
    reload();
    const id = intervalMs ? setInterval(reload, intervalMs) : undefined;
    return () => {
      alive.current = false;
      if (id) clearInterval(id);
    };
  }, [reload, intervalMs]);

  return { data, error, loading, reload };
}

/** Runs backend actions and keeps the last result for a `StatusLine`. */
export function useAction(onDone?: () => void) {
  const [result, setResult] = useState<ActionResult | null>(null);
  const [busy, setBusy] = useState(false);

  async function run(command: string, args?: Record<string, unknown>) {
    setBusy(true);
    try {
      const r = await invoke<unknown>(command, args);
      setResult(
        r && typeof r === "object" && "ok" in r
          ? (r as ActionResult)
          : { ok: true, message: typeof r === "string" ? r : "Done" }
      );
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    } finally {
      setBusy(false);
      onDone?.();
    }
  }

  return { result, busy, run };
}

export function StatusLine({ result }: { result: ActionResult | null }) {
  if (!result) return null;
  return (
    <p className={result.ok ? "status-ok" : "status-fail"}>
      {result.ok ? "✓" : "✗"} {result.message}
    </p>
  );
}

/**
 * A button for destructive actions: the first click arms it and swaps the
 * label for a confirmation, the second click within a few seconds runs it.
 */
export function ConfirmButton({
  label,
  confirmLabel = "Click again to confirm",
  onConfirm,
  className = "ctl-btn ctl-danger",
  disabled,
}: {
  label: string;
  confirmLabel?: string;
  onConfirm: () => void;
  className?: string;
  disabled?: boolean;
}) {
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const id = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(id);
  }, [armed]);

  return (
    <button
      className={className + (armed ? " ctl-armed" : "")}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        if (armed) {
          setArmed(false);
          onConfirm();
        } else {
          setArmed(true);
        }
      }}
    >
      {armed ? confirmLabel : label}
    </button>
  );
}

/** A row of mutually exclusive choices (power profile, governor, …). */
export function Segmented({
  options,
  value,
  onChange,
  disabled,
}: {
  options: string[];
  value: string | null;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  return (
    <div className="segmented">
      {options.map((o) => (
        <button
          key={o}
          className={o === value ? "range-btn active" : "range-btn"}
          disabled={disabled}
          onClick={() => o !== value && onChange(o)}
        >
          {o.replace(/[-_]/g, " ")}
        </button>
      ))}
    </div>
  );
}

/** Tiny inline trend line; `values` are plotted left to right from zero. */
export function Sparkline({
  values,
  color,
  width = 120,
  height = 26,
}: {
  values: number[];
  color: string;
  width?: number;
  height?: number;
}) {
  if (values.length < 2) return <span className="mem-muted">—</span>;
  const max = Math.max(...values, 1);
  const points = values
    .map((v, i) => {
      const x = (i / (values.length - 1)) * width;
      const y = height - 2 - (v / max) * (height - 4);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={width} height={height} className="sparkline">
      <polyline points={points} fill="none" stroke={color} strokeWidth="2" />
    </svg>
  );
}

export function formatDuration(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** A headline number with a caption; `tone` colours the value by status. */
export function Tile({
  label,
  value,
  detail,
  tone,
}: {
  label: string;
  value: string;
  detail?: string;
  tone?: "ok" | "warn" | "danger";
}) {
  return (
    <div className="tile">
      <span className="tile-label">{label}</span>
      <span className={"tile-value" + (tone ? ` tile-${tone}` : "")}>{value}</span>
      {detail && <span className="tile-detail">{detail}</span>}
    </div>
  );
}

export interface ChartPoint {
  ts: number;
  v: number;
}

/**
 * Line chart over time with a hover readout. `markers` draws a tick on the
 * baseline at each timestamp (used for events).
 */
export function TimeChart({
  points,
  max,
  color,
  format,
  formatTime,
  markers = [],
  height = 150,
}: {
  points: ChartPoint[];
  max: number;
  color: string;
  format: (v: number) => string;
  formatTime: (ts: number) => string;
  markers?: number[];
  height?: number;
}) {
  const [hover, setHover] = useState<number | null>(null);

  if (points.length < 2) {
    return (
      <div className="chart-empty" style={{ height }}>
        Not enough data yet — history builds up while Trace is running
      </div>
    );
  }
  const minTs = points[0].ts;
  const maxTs = points[points.length - 1].ts;
  const span = Math.max(maxTs - minTs, 1);
  const top = Math.max(max, 1e-9);
  const x = (ts: number) => ((ts - minTs) / span) * 100;
  const y = (v: number) => height - (Math.min(v, top) / top) * height;
  const values = points.map((p) => p.v);
  const avg = values.reduce((a, b) => a + b, 0) / values.length;
  const path = points
    .map((p, i) => `${i === 0 ? "M" : "L"}${x(p.ts).toFixed(2)},${y(p.v).toFixed(2)}`)
    .join(" ");
  const shown = hover != null ? points[hover] : points[points.length - 1];

  function onMove(e: React.MouseEvent<HTMLDivElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const ts = minTs + ((e.clientX - rect.left) / rect.width) * span;
    let nearest = 0;
    for (let i = 1; i < points.length; i++) {
      if (Math.abs(points[i].ts - ts) < Math.abs(points[nearest].ts - ts)) nearest = i;
    }
    setHover(nearest);
  }

  return (
    <div>
      <div className="chart-stats">
        <span className="chart-stat">
          <strong>{format(shown.v)}</strong>{" "}
          {hover != null ? `at ${formatTime(shown.ts)}` : "now"}
        </span>
        <span className="chart-stat">avg {format(avg)}</span>
        <span className="chart-stat">min {format(Math.min(...values))}</span>
        <span className="chart-stat">max {format(Math.max(...values))}</span>
      </div>
      <div className="chart-with-axis">
        <div className="chart-y-axis">
          <span>{format(top)}</span>
          <span>{format(top / 2)}</span>
          <span>{format(0)}</span>
        </div>
        <div
          className="chart-plot"
          style={{ height }}
          onMouseMove={onMove}
          onMouseLeave={() => setHover(null)}
        >
          <svg
            viewBox={`0 0 100 ${height}`}
            preserveAspectRatio="none"
            className="chart-svg"
            style={{ height }}
          >
            <line x1="0" y1={height / 2} x2="100" y2={height / 2} className="chart-grid" />
            <line x1="0" y1="0" x2="100" y2="0" className="chart-grid" />
            <path d={`${path} L100,${height} L0,${height} Z`} fill={color} opacity={0.14} />
            <path
              d={path}
              fill="none"
              stroke={color}
              strokeWidth={2}
              vectorEffect="non-scaling-stroke"
            />
          </svg>
          {markers
            .filter((ts) => ts >= minTs && ts <= maxTs)
            .map((ts, i) => (
              <span key={i} className="chart-marker" style={{ left: `${x(ts)}%` }} />
            ))}
          {hover != null && (
            <>
              <span className="chart-cursor" style={{ left: `${x(shown.ts)}%` }} />
              <span
                className="chart-dot"
                style={{
                  left: `${x(shown.ts)}%`,
                  top: y(shown.v),
                  background: color,
                }}
              />
            </>
          )}
        </div>
      </div>
      <div className="chart-x-axis">
        <span>{formatTime(minTs)}</span>
        <span>{formatTime(maxTs)}</span>
      </div>
    </div>
  );
}

/** Vertical bars, one per bucket, with a readout for the hovered bar. */
export function BarChart({
  bars,
  color,
  format,
  max,
  height = 110,
}: {
  bars: { label: string; value: number; empty?: boolean }[];
  color: string;
  format: (v: number) => string;
  max?: number;
  height?: number;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const top = Math.max(max ?? 0, ...bars.map((b) => b.value), 1e-9);
  const shown = hover != null ? bars[hover] : null;
  // Label roughly eight ticks however many bars there are.
  const every = Math.max(1, Math.ceil(bars.length / 8));

  return (
    <div>
      <div className="chart-stats">
        <span className="chart-stat">
          {shown ? (
            <>
              <strong>{shown.empty ? "no data" : format(shown.value)}</strong> · {shown.label}
            </>
          ) : (
            <>peak {format(Math.max(...bars.map((b) => b.value)))} · hover a bar</>
          )}
        </span>
      </div>
      <div className="bar-chart" style={{ height }} onMouseLeave={() => setHover(null)}>
        {bars.map((b, i) => (
          <div key={i} className="bar-slot" onMouseEnter={() => setHover(i)}>
            <div
              className={"bar-column" + (hover === i ? " hovered" : "")}
              style={{
                height: b.empty ? 2 : `${Math.max((b.value / top) * 100, 1.5)}%`,
                background: b.empty ? "#262a36" : color,
              }}
            />
          </div>
        ))}
      </div>
      <div className="bar-labels">
        {bars.map((b, i) => (
          <span key={i}>{i % every === 0 ? b.label : ""}</span>
        ))}
      </div>
    </div>
  );
}
