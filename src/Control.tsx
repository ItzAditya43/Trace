import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ProcessInfo {
  pid: number;
  name: string;
  cpu_usage: number;
  memory_bytes: number;
}

interface ActionResult {
  ok: boolean;
  message: string;
}

interface UnitInfo {
  name: string;
  description: string;
  load_state: string;
  active_state: string;
  sub_state: string;
}

interface ContainerInfo {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
}

type ProfileAction =
  | { type: "FreezeProcess"; pid: number }
  | { type: "ResumeProcess"; pid: number }
  | { type: "KillProcess"; pid: number; force: boolean }
  | { type: "SetCpuLimit"; pid: number; percent: number }
  | { type: "SetMemoryLimitMb"; pid: number; mb: number }
  | { type: "StartService"; name: string }
  | { type: "StopService"; name: string }
  | { type: "RestartService"; name: string }
  | { type: "StartContainer"; id: string }
  | { type: "StopContainer"; id: string };

interface Profile {
  name: string;
  description: string;
  actions: ProfileAction[];
}

interface ActionOutcome {
  description: string;
  ok: boolean;
  message: string;
}

interface ActionRow {
  ts: number;
  action: string;
  ok: boolean;
  message: string;
}

function StatusLine({ result }: { result: ActionResult | string | null }) {
  if (!result) return null;
  const ok = typeof result === "string" ? true : result.ok;
  const message = typeof result === "string" ? result : result.message;
  return (
    <p className={ok ? "status-ok" : "status-fail"}>
      {ok ? "✓" : "✗"} {message}
    </p>
  );
}

function ProcessControl() {
  const [processes, setProcesses] = useState<ProcessInfo[]>([]);
  const [pid, setPid] = useState("");
  const [cores, setCores] = useState("");
  const [percent, setPercent] = useState("50");
  const [mb, setMb] = useState("512");
  const [nice, setNice] = useState("0");
  const [result, setResult] = useState<ActionResult | null>(null);

  useEffect(() => {
    invoke<{ top_processes: ProcessInfo[] }>("get_snapshot").then((s) =>
      setProcesses(s.top_processes)
    );
  }, []);

  function targetPid(): number | null {
    const n = parseInt(pid, 10);
    return Number.isFinite(n) && n > 0 ? n : null;
  }

  async function act(cmd: string, args: Record<string, unknown>) {
    const p = targetPid();
    if (!p) return;
    try {
      const r = await invoke<ActionResult>(cmd, { pid: p, ...args });
      setResult(r);
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>Processes</h2>
      <div className="proc-picker">
        <select
          className="port-input"
          value={pid}
          onChange={(e) => setPid(e.target.value)}
        >
          <option value="">Select a process…</option>
          {processes.map((p) => (
            <option key={p.pid} value={p.pid}>
              {p.name} (PID {p.pid}) — {(p.memory_bytes / 1e9).toFixed(2)} GB
            </option>
          ))}
        </select>
      </div>

      <div className="control-actions">
        <button className="ctl-btn" onClick={() => act("freeze_process", {})}>
          Freeze
        </button>
        <button className="ctl-btn" onClick={() => act("resume_process", {})}>
          Resume
        </button>
        <button
          className="ctl-btn ctl-danger"
          onClick={() => act("kill_process", { force: false })}
        >
          Kill (SIGTERM)
        </button>
        <button
          className="ctl-btn ctl-danger"
          onClick={() => act("kill_process", { force: true })}
        >
          Force Kill
        </button>
      </div>

      <div className="control-row">
        <label>Priority (nice, -20 to 19)</label>
        <input
          className="port-input small"
          value={nice}
          onChange={(e) => setNice(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() => act("set_process_priority", { nice: parseInt(nice, 10) })}
        >
          Apply
        </button>
      </div>

      <div className="control-row">
        <label>CPU affinity (comma-separated cores)</label>
        <input
          className="port-input small"
          placeholder="0,1,2"
          value={cores}
          onChange={(e) => setCores(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() =>
            act("set_process_affinity", {
              cores: cores
                .split(",")
                .map((c) => parseInt(c.trim(), 10))
                .filter((n) => Number.isFinite(n)),
            })
          }
        >
          Apply
        </button>
      </div>

      <div className="control-row">
        <label>CPU limit (%)</label>
        <input
          className="port-input small"
          value={percent}
          onChange={(e) => setPercent(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() =>
            act("set_process_cpu_limit", { percent: parseInt(percent, 10) })
          }
        >
          Apply
        </button>
      </div>

      <div className="control-row">
        <label>Memory limit (MB)</label>
        <input
          className="port-input small"
          value={mb}
          onChange={(e) => setMb(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() => act("set_process_memory_limit", { mb: parseInt(mb, 10) })}
        >
          Apply
        </button>
      </div>

      <StatusLine result={result} />
      <p className="diagnose-hint">
        CPU/memory limits act on the process's current cgroup and often need
        elevated permissions — a clear error will show if the cgroup isn't
        writable by your user.
      </p>
    </section>
  );
}

function ServiceControl() {
  const [units, setUnits] = useState<UnitInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    setLoading(true);
    try {
      const u = await invoke<UnitInfo[]>("list_services", {
        runningOrFailedOnly: true,
      });
      setUnits(u);
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function act(cmd: string, name: string) {
    try {
      const message = await invoke<string>(cmd, { name });
      setResult({ ok: true, message });
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>systemd Services ({units.length})</h2>
      <button className="ctl-btn" onClick={refresh} disabled={loading}>
        {loading ? "Refreshing…" : "Refresh"}
      </button>
      <div className="unit-list">
        {units.map((u) => (
          <div key={u.name} className="unit-row">
            <span className="unit-name">{u.name}</span>
            <span
              className={
                "unit-state " +
                (u.active_state === "active" ? "unit-active" : "unit-inactive")
              }
            >
              {u.active_state}/{u.sub_state}
            </span>
            <div className="unit-buttons">
              <button className="ctl-btn-sm" onClick={() => act("start_service", u.name)}>
                Start
              </button>
              <button className="ctl-btn-sm" onClick={() => act("stop_service", u.name)}>
                Stop
              </button>
              <button className="ctl-btn-sm" onClick={() => act("restart_service", u.name)}>
                Restart
              </button>
            </div>
          </div>
        ))}
      </div>
      <StatusLine result={result} />
    </section>
  );
}

function DockerControl() {
  const [containers, setContainers] = useState<ContainerInfo[] | null>(null);
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    try {
      const c = await invoke<ContainerInfo[]>("list_containers");
      setContainers(c);
    } catch (e) {
      setResult({ ok: false, message: String(e) });
      setContainers([]);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function act(cmd: string, id: string) {
    try {
      const message = await invoke<string>(cmd, { id });
      setResult({ ok: true, message });
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>Docker Containers</h2>
      {containers === null ? (
        <p className="diagnose-hint">Loading…</p>
      ) : containers.length === 0 ? (
        <p className="diagnose-hint">
          No containers found (or Docker isn't reachable).
        </p>
      ) : (
        <div className="unit-list">
          {containers.map((c) => (
            <div key={c.id} className="unit-row">
              <span className="unit-name">{c.name || c.id}</span>
              <span className="unit-state">{c.status}</span>
              <div className="unit-buttons">
                <button className="ctl-btn-sm" onClick={() => act("start_container", c.id)}>
                  Start
                </button>
                <button className="ctl-btn-sm" onClick={() => act("stop_container", c.id)}>
                  Stop
                </button>
                <button className="ctl-btn-sm" onClick={() => act("restart_container", c.id)}>
                  Restart
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
      <StatusLine result={result} />
    </section>
  );
}

function GpuControl() {
  const [watts, setWatts] = useState("60");
  const [result, setResult] = useState<ActionResult | null>(null);

  async function act(cmd: string, args: Record<string, unknown>) {
    try {
      const message = await invoke<string>(cmd, args);
      setResult({ ok: true, message });
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>GPU</h2>
      <div className="control-actions">
        <button
          className="ctl-btn"
          onClick={() => act("set_gpu_persistence", { enabled: true })}
        >
          Enable Persistence Mode
        </button>
        <button
          className="ctl-btn"
          onClick={() => act("set_gpu_persistence", { enabled: false })}
        >
          Disable Persistence Mode
        </button>
      </div>
      <div className="control-row">
        <label>Power limit (W)</label>
        <input
          className="port-input small"
          value={watts}
          onChange={(e) => setWatts(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() => act("set_gpu_power_limit", { watts: parseInt(watts, 10) })}
        >
          Apply
        </button>
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Most GPU controls require root and a supported driver — expect
        permission errors unless running with elevated privileges.
      </p>
    </section>
  );
}

function ProfilesControl() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [outcomes, setOutcomes] = useState<ActionOutcome[] | null>(null);
  const [activeProfile, setActiveProfile] = useState<string | null>(null);

  async function refresh() {
    const p = await invoke<Profile[]>("list_profiles");
    setProfiles(p);
  }

  useEffect(() => {
    refresh();
  }, []);

  async function run(profile: Profile, dryRun: boolean, undo: boolean) {
    setActiveProfile(profile.name);
    setOutcomes(null);
    try {
      const result = await invoke<ActionOutcome[]>(
        undo ? "undo_profile" : "apply_profile",
        undo ? { profile } : { profile, dryRun }
      );
      setOutcomes(result);
    } catch (e) {
      setOutcomes([{ description: "Error", ok: false, message: String(e) }]);
    }
  }

  return (
    <section className="panel">
      <h2>Profiles</h2>
      <p className="diagnose-hint">
        Profiles are TOML files under your Trace data directory — hand-edit
        them to build "Gaming Mode", "Focus Mode", etc. as ordered action
        lists.
      </p>
      {profiles.length === 0 && (
        <p className="diagnose-hint">
          No profiles yet. A starter "Focus Mode" template was created empty
          — edit its TOML file to add actions.
        </p>
      )}
      {profiles.map((p) => (
        <div key={p.name} className="profile-card">
          <div className="profile-header">
            <span className="profile-name">{p.name}</span>
            <span className="profile-desc">{p.description}</span>
          </div>
          <ol className="profile-actions">
            {p.actions.map((a, i) => (
              <li key={i}>{describeAction(a)}</li>
            ))}
          </ol>
          <div className="control-actions">
            <button className="ctl-btn" onClick={() => run(p, true, false)}>
              Dry Run
            </button>
            <button className="ctl-btn ctl-primary" onClick={() => run(p, false, false)}>
              Apply
            </button>
            <button className="ctl-btn" onClick={() => run(p, false, true)}>
              Undo
            </button>
          </div>
          {activeProfile === p.name && outcomes && (
            <div className="outcome-list">
              {outcomes.map((o, i) => (
                <div key={i} className={o.ok ? "status-ok" : "status-fail"}>
                  {o.ok ? "✓" : "✗"} {o.description} — {o.message}
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
    </section>
  );
}

function describeAction(a: ProfileAction): string {
  switch (a.type) {
    case "FreezeProcess":
      return `Freeze PID ${a.pid}`;
    case "ResumeProcess":
      return `Resume PID ${a.pid}`;
    case "KillProcess":
      return `Kill PID ${a.pid}${a.force ? " (force)" : ""}`;
    case "SetCpuLimit":
      return `Limit PID ${a.pid} to ${a.percent}% CPU`;
    case "SetMemoryLimitMb":
      return `Limit PID ${a.pid} to ${a.mb} MB RAM`;
    case "StartService":
      return `Start service ${a.name}`;
    case "StopService":
      return `Stop service ${a.name}`;
    case "RestartService":
      return `Restart service ${a.name}`;
    case "StartContainer":
      return `Start container ${a.id}`;
    case "StopContainer":
      return `Stop container ${a.id}`;
  }
}

function ActionLog() {
  const [rows, setRows] = useState<ActionRow[]>([]);

  useEffect(() => {
    invoke<ActionRow[]>("get_action_log", { sinceSecsAgo: 24 * 3600 }).then(
      setRows
    );
  }, []);

  if (rows.length === 0) return null;

  return (
    <section className="panel">
      <h2>Action Log (last 24h)</h2>
      <div className="event-timeline">
        {rows.map((r, i) => (
          <div key={i} className={"event-row " + (r.ok ? "event-info" : "event-warn")}>
            <span className="event-time">
              {new Date(r.ts * 1000).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
            <span className="event-message">
              {r.action}: {r.message}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

export default function Control() {
  return (
    <div>
      <ProcessControl />
      <ServiceControl />
      <DockerControl />
      <GpuControl />
      <ProfilesControl />
      <ActionLog />
    </div>
  );
}
