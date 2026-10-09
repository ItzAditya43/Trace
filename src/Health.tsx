import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkline, bytesToHuman, formatDay, usePoll } from "./ui";
import { BatteryCareCard } from "./More";
import { MediaUseCard, SecurityCheckup } from "./Batch3";
import { DiskGrowth, DriveWear, PowerPerApp } from "./Batch4";
import { SecureErase } from "./Batch6";

interface Issue {
  severity: "critical" | "warn" | "info";
  title: string;
  detail: string;
}

interface HealthOverview {
  report: { score: number; grade: string; issues: Issue[] };
  reboot: {
    reboot_needed: boolean;
    reasons: string[];
    running_kernel: string;
    stale_apps: { name: string; process_count: number }[];
  };
  failed_units: { name: string; description: string; user: boolean }[];
  batteries: {
    name: string;
    status: string;
    percent: number;
    power_watts: number;
    hours_remaining: number | null;
    full_wh: number;
    design_wh: number;
    wear_percent: number;
    cycle_count: number | null;
    charge_limit_percent: number | null;
  }[];
  boots: { ts: number; total_ms: number; userspace_ms: number }[];
  drives: { device: string; model: string; size_bytes: number; rotational: boolean }[];
}

interface DriveHealth {
  device: string;
  passed: boolean | null;
  temperature_c: number | null;
  power_on_hours: number | null;
  wear_percent: number | null;
  available_spare_percent: number | null;
  media_errors: number | null;
}

interface LogLine {
  time: string;
  source: string;
  message: string;
}

const SEVERITY_LABEL = { critical: "Critical", warn: "Warning", info: "Note" };
const SEVERITY_ICON = { critical: "✗", warn: "!", info: "i" };

function gradeClass(score: number): string {
  if (score >= 90) return "score-good";
  if (score >= 70) return "score-fair";
  if (score >= 45) return "score-strained";
  return "score-critical";
}

export function HealthScore({ compact }: { compact?: boolean }) {
  const { data } = usePoll<HealthOverview>("get_health", undefined, 30000);
  if (!data) return null;
  const { report } = data;
  const issues = compact ? report.issues.slice(0, 3) : report.issues;
  return (
    <section className="panel wide">
      <h2>Machine Health</h2>
      <div className="score-layout">
        <div className={`score-badge ${gradeClass(report.score)}`}>
          <span className="score-number">{report.score}</span>
          <span className="score-grade">{report.grade}</span>
        </div>
        <div className="issue-list">
          {issues.length === 0 && (
            <p className="diagnose-hint">Nothing needs your attention.</p>
          )}
          {issues.map((issue, i) => (
            <div key={i} className="issue-row">
              <span className={`issue-tag issue-${issue.severity}`}>
                {SEVERITY_ICON[issue.severity]} {SEVERITY_LABEL[issue.severity]}
              </span>
              <div>
                <div className="issue-title">{issue.title}</div>
                <div className="issue-detail">{issue.detail}</div>
              </div>
            </div>
          ))}
          {compact && report.issues.length > issues.length && (
            <p className="diagnose-hint">
              +{report.issues.length - issues.length} more on the Health page
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

function LogPanel({
  title,
  command,
  empty,
  hint,
  wide,
}: {
  wide?: boolean;
  title: string;
  command: string;
  empty: string;
  hint: string;
}) {
  const { data, loading } = usePoll<LogLine[]>(command);
  const [showAll, setShowAll] = useState(false);
  const lines = showAll ? data ?? [] : (data ?? []).slice(0, 8);
  return (
    <section className={wide ? "panel wide" : "panel"}>
      <h2>{title}</h2>
      {loading && !data && <p className="loading">Reading the system journal…</p>}
      {data && data.length === 0 && <p className="status-ok">✓ {empty}</p>}
      {lines.map((l, i) => (
        <div key={i} className="event-row">
          <span className="event-time">
            {new Date(l.time).toLocaleString([], {
              month: "short",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            })}
          </span>
          <span className="event-message">
            {l.source && <span className="mem-muted">{l.source}: </span>}
            {l.message}
          </span>
        </div>
      ))}
      {data && data.length > lines.length && (
        <button className="ctl-btn" onClick={() => setShowAll(true)}>
          Show all {data.length}
        </button>
      )}
      <p className="diagnose-hint">{hint}</p>
    </section>
  );
}

function Drives({ drives }: { drives: HealthOverview["drives"] }) {
  const [health, setHealth] = useState<DriveHealth[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function check() {
    setBusy(true);
    setError(null);
    try {
      setHealth(await invoke<DriveHealth[]>("check_drive_health"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel">
      <h2>Drives</h2>
      <table className="proc-table">
        <thead>
          <tr>
            <th>Drive</th>
            <th>Size</th>
            <th>Health</th>
            <th>Wear</th>
            <th>Temp</th>
            <th>Powered on</th>
          </tr>
        </thead>
        <tbody>
          {drives.map((d) => {
            const h = health?.find((x) => x.device === d.device);
            return (
              <tr key={d.device}>
                <td>
                  {d.device} <span className="mem-muted">{d.model}</span>
                </td>
                <td>{bytesToHuman(d.size_bytes)}</td>
                <td>
                  {h?.passed == null ? (
                    "—"
                  ) : h.passed ? (
                    <span className="status-ok">✓ Passed</span>
                  ) : (
                    <span className="status-fail">✗ Failing</span>
                  )}
                </td>
                <td>{h?.wear_percent != null ? `${h.wear_percent}% used` : "—"}</td>
                <td>{h?.temperature_c != null ? `${h.temperature_c}°C` : "—"}</td>
                <td>{h?.power_on_hours != null ? `${h.power_on_hours} h` : "—"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="control-actions">
        <button className="ctl-btn" onClick={check} disabled={busy}>
          {busy ? "Waiting for authorization…" : "Check drive health"}
        </button>
        {drives.map((d) => (
          <button
            key={d.device}
            className="ctl-btn"
            disabled={busy}
            onClick={async () => {
              const r = await invoke<{ ok: boolean; message: string }>("start_drive_self_test", { device: d.device });
              setError(r.ok ? null : r.message);
              setNote(r.ok ? r.message : null);
            }}
          >
            Self-test {d.device}
          </button>
        ))}
      </div>
      {error && <p className="status-fail">✗ {error}</p>}
      {note && <p className="status-ok">✓ {note}</p>}
      <DriveWear />
      <p className="diagnose-hint">
        Drive self-test data (SMART) is only readable by root, so checking it
        asks for your password. "Wear" is how much of an SSD's rated lifetime
        has been used.
      </p>
    </section>
  );
}

export default function Health() {
  const { data } = usePoll<HealthOverview>("get_health", undefined, 30000);
  if (!data) return <p className="loading">Checking the machine…</p>;
  const { reboot, failed_units, batteries, boots } = data;
  const bootsOldestFirst = [...boots].reverse();

  return (
    <>
      <HealthScore />

      <section className="panel">
        <h2>Restart Needed?</h2>
        {reboot.reasons.length === 0 ? (
          <p className="status-ok">
            ✓ No reboot needed — running kernel {reboot.running_kernel} matches
            what is installed.
          </p>
        ) : (
          reboot.reasons.map((r, i) => (
            <p key={i} className="status-fail">
              ! {r}
            </p>
          ))
        )}
        {reboot.stale_apps.length > 0 && (
          <>
            <p className="diagnose-hint">
              These apps are still running a version that an update has
              replaced — restarting just the app picks up the new one:
            </p>
            <div className="chip-row">
              {reboot.stale_apps.slice(0, 24).map((a) => (
                <span key={a.name} className="chip">
                  {a.name}
                  {a.process_count > 1 && ` ×${a.process_count}`}
                </span>
              ))}
            </div>
          </>
        )}
      </section>

      <section className="panel">
        <h2>Failed Services</h2>
        {failed_units.length === 0 ? (
          <p className="status-ok">✓ Every service started cleanly.</p>
        ) : (
          failed_units.map((u) => (
            <div key={u.name + u.user} className="issue-row failed-unit">
              <span className="issue-tag issue-critical">✗ Failed</span>
              <div>
                <div className="issue-title">{u.name}</div>
                <div className="issue-detail">
                  {u.description} · {u.user ? "your session" : "system"}
                </div>
              </div>
            </div>
          ))
        )}
      </section>

      {batteries.map((b) => (
        <section key={b.name} className="panel">
          <h2>Battery — {b.name}</h2>
          <div className="mem-stats">
            <div className="mem-stat">
              <span className="mem-stat-value">{b.percent.toFixed(0)}%</span>
              <span className="mem-stat-label">{b.status.toLowerCase()}</span>
            </div>
            <div className="mem-stat">
              <span className="mem-stat-value">{b.power_watts.toFixed(1)} W</span>
              <span className="mem-stat-label">
                {b.status === "Discharging" ? "being drawn" : "flowing in"}
                {b.hours_remaining != null &&
                  ` · about ${b.hours_remaining.toFixed(1)} h left`}
              </span>
            </div>
            <div className="mem-stat">
              <span className="mem-stat-value">{(100 - b.wear_percent).toFixed(0)}%</span>
              <span className="mem-stat-label">
                of original capacity ({b.full_wh.toFixed(0)} of{" "}
                {b.design_wh.toFixed(0)} Wh)
              </span>
            </div>
            {b.cycle_count != null && (
              <div className="mem-stat">
                <span className="mem-stat-value">{b.cycle_count}</span>
                <span className="mem-stat-label">charge cycles</span>
              </div>
            )}
          </div>
          {b.charge_limit_percent != null && b.charge_limit_percent < 100 && (
            <p className="diagnose-hint">
              Charging stops at {b.charge_limit_percent}% to slow battery ageing.
            </p>
          )}
        </section>
      ))}

      <BatteryCareCard />
      <PowerPerApp />
      <DiskGrowth />
      <SecureErase />
      <SecurityCheckup />
      <MediaUseCard />

      <section className="panel">
        <h2>Boot Time</h2>
        {boots.length === 0 ? (
          <p className="diagnose-hint">No boots recorded yet.</p>
        ) : (
          <>
            <div className="mem-stats">
              <div className="mem-stat">
                <span className="mem-stat-value">
                  {(boots[0].total_ms / 1000).toFixed(1)} s
                </span>
                <span className="mem-stat-label">
                  this boot, power button to desktop (
                  {(boots[0].userspace_ms / 1000).toFixed(1)} s of it starting
                  services)
                </span>
              </div>
              {boots.length > 1 && (
                <div className="mem-stat">
                  <Sparkline
                    values={bootsOldestFirst.map((b) => b.total_ms)}
                    color="#3987e5"
                    width={180}
                  />
                  <span className="mem-stat-label">
                    last {boots.length} boots since {formatDay(bootsOldestFirst[0].ts)}
                  </span>
                </div>
              )}
            </div>
            {boots.length === 1 && (
              <p className="diagnose-hint">
                A trend appears here once Trace has seen more than one boot.
              </p>
            )}
          </>
        )}
      </section>

      <Drives drives={data.drives} />

      <LogPanel
        title="Out-of-Memory Kills (last 7 days)"
        command="get_oom_kills"
        empty="Nothing was killed for lack of memory."
        hint="When RAM and swap run out the kernel kills a process to survive. Each one listed here was an app that vanished without warning."
      />
      <LogPanel
        wide
        title="Recent Errors (this boot)"
        command="get_recent_errors"
        empty="No errors logged since boot."
        hint="Error-level messages from the system journal, newest first."
      />
    </>
  );
}
