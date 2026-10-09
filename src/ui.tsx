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
