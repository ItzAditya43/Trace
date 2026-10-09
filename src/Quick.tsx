import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Segmented, StatusLine, formatClock, formatDay, useAction, usePoll } from "./ui";

interface PinnedTweak {
  id: string;
  title: string;
  kind: "toggle" | "choice" | "number";
  options: { value: string; label: string }[];
  value: string;
  needs_root: boolean;
}

interface Explanation {
  headline: string;
  points: string[];
}

/** One-click switches for the settings pinned on the Tweaks page. */
export function PinnedSwitches() {
  const { data, reload } = usePoll<PinnedTweak[]>("get_pinned_tweaks", undefined, 20000);
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  if (!data || data.length === 0) return null;

  async function set(t: PinnedTweak, value: string) {
    setBusy(t.id);
    setFailed(null);
    try {
      const results = await invoke<{ ok: boolean; title: string; message: string }[]>("apply_tweaks", {
        changes: [{ id: t.id, value }],
        source: "pinned",
        keep: null,
      });
      const bad = results.find((r) => !r.ok);
      if (bad) setFailed(`${bad.title}: ${bad.message}`);
    } finally {
      setBusy(null);
      reload();
    }
  }

  return (
    <section className="panel wide">
      <h2>Your Switches</h2>
      <div className="pinned-grid">
        {data.map((t) => (
          <div key={t.id} className="pinned">
            <span className="tweak-title">
              {t.title}
              {t.needs_root && " 🔒"}
            </span>
            {t.kind === "toggle" ? (
              <Segmented
                options={["on", "off"]}
                value={t.value}
                disabled={busy === t.id}
                onChange={(v) => set(t, v)}
              />
            ) : t.kind === "choice" ? (
              <select
                className="port-input tweak-select"
                value={t.value}
                disabled={busy === t.id}
                onChange={(e) => set(t, e.target.value)}
              >
                {t.options.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            ) : (
              <span className="mem-muted">{t.value}</span>
            )}
          </div>
        ))}
      </div>
      {failed && <p className="status-fail">✗ {failed}</p>}
    </section>
  );
}

/** "Why is it hot / loud / slow right now?" */
export function ExplainNow() {
  const [answer, setAnswer] = useState<Explanation | null>(null);
  const [busy, setBusy] = useState(false);

  async function ask() {
    setBusy(true);
    try {
      setAnswer(await invoke<Explanation>("explain_now"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="control-actions">
        <button className="ctl-btn" onClick={ask} disabled={busy}>
          {busy ? "Looking…" : answer ? "Ask again" : "Why is it like this right now?"}
        </button>
      </div>
      {answer && (
        <div className="explain">
          <strong>{answer.headline}</strong>
          <ul>
            {answer.points.map((p, i) => (
              <li key={i}>{p}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** Countdown to lock / sleep / shut down, and "tell me when it's done" watches. */
export function TimersControl() {
  const { data, reload } = usePoll<{
    watches: { pid: number; name: string; since: number }[];
    countdown: { action: string; at: number } | null;
  }>("get_pending", undefined, 5000);
  const { result, busy, run } = useAction(reload);
  const [action, setAction] = useState("suspend");
  const [minutes, setMinutes] = useState("45");
  const labels: Record<string, string> = {
    lock: "Lock the screen",
    suspend: "Sleep",
    poweroff: "Shut down",
    awake_off: "Stop keeping awake",
  };

  return (
    <section className="panel">
      <h2>Timers</h2>
      {data?.countdown ? (
        <div className="control-row">
          <label>
            <strong>{labels[data.countdown.action]}</strong> at {formatClock(data.countdown.at)} (
            {Math.max(0, Math.round((data.countdown.at - Date.now() / 1000) / 60))} min left)
          </label>
          <button className="ctl-btn" disabled={busy} onClick={() => run("set_countdown", { action: null, minutes: 0 })}>
            Cancel
          </button>
        </div>
      ) : (
        <div className="control-row">
          <label>In</label>
          <input
            className="port-input number-box"
            value={minutes}
            onChange={(e) => setMinutes(e.target.value.replace(/\D/g, ""))}
            aria-label="Minutes"
          />
          <label style={{ flex: "none", minWidth: 0 }}>minutes</label>
          <select className="port-input small-select" value={action} onChange={(e) => setAction(e.target.value)}>
            {Object.entries(labels).map(([key, label]) => (
              <option key={key} value={key}>
                {label}
              </option>
            ))}
          </select>
          <button
            className="ctl-btn"
            disabled={busy || !(parseInt(minutes, 10) > 0)}
            onClick={() => run("set_countdown", { action, minutes: parseInt(minutes, 10) })}
          >
            Start
          </button>
        </div>
      )}
      {(data?.watches ?? []).map((w) => (
        <div key={w.pid} className="meta-row">
          <span>
            Waiting for {w.name} (PID {w.pid}) to finish
          </span>
          <span>
            since {formatDay(w.since)} {formatClock(w.since)}
          </span>
        </div>
      ))}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        You get a warning a minute before the machine sleeps or shuts down. To be told when a build
        or download finishes, pick its process under Apps &amp; Processes and press "Tell me when it's
        done". Timers are lost if Trace quits.
      </p>
    </section>
  );
}

/** Every scheduled job on the system. */
export function ScheduledJobs() {
  const { data } = usePoll<{ unit: string; activates: string; user: boolean; next: number; last: number }[]>(
    "list_timers"
  );
  const when = (ts: number) => (ts > 0 ? `${formatDay(ts)} ${formatClock(ts)}` : "—");
  return (
    <section className="panel">
      <h2>Scheduled Jobs ({data?.length ?? 0})</h2>
      <div className="unit-list">
        {(data ?? []).map((j) => (
          <div key={j.unit + j.user} className="unit-row" title={`Runs ${j.activates}`}>
            <span className="unit-name">
              {j.unit.replace(".timer", "")}
              {j.user && <span className="mem-muted"> (yours)</span>}
            </span>
            <span className="unit-state">next {when(j.next)}</span>
            <span className="unit-state">last {when(j.last)}</span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">
        Everything the system runs on a schedule, soonest first. The common ones can be switched off
        under Tweaks → Background Services.
      </p>
    </section>
  );
}
