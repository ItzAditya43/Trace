import { useContext, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { NavContext } from "./nav";
import AppSettings from "./AppSettings";
import { ScheduledJobs, TimersControl } from "./Quick";
import { ConnectionDoctor, DataBudgets, ListeningPorts } from "./More";
import { PrivacyCleaner, RunRestricted, SavedWifiCard } from "./Batch3";
import { invoke } from "@tauri-apps/api/core";
import Automation from "./Automation";
import Packages from "./Packages";
import { CpuPowerControl, FanControl, MemoryTuningControl } from "./ControlTuning";
import { AudioControl, CleanupControl, SessionControl } from "./ControlDesktop";
import { VpnControl } from "./ControlVpn";
import { DisplayControl, FirewallControl, WirelessControl } from "./ControlConnect";

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
  | { type: "StopContainer"; id: string }
  | { type: "FreezeApp"; app: string }
  | { type: "ResumeApp"; app: string }
  | { type: "QuitApp"; app: string }
  | { type: "SetPowerProfile"; profile: string }
  | { type: "ApplyPreset"; preset: string }
  | { type: "RunCommand"; command: string };

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
  const [ioClass, setIoClass] = useState("2");
  const [ioLevel, setIoLevel] = useState("4");
  const [result, setResult] = useState<ActionResult | null>(null);

  const { target } = useContext(NavContext);
  const [search, setSearch] = useState("");

  // Arriving from global search with a program name.
  useEffect(() => {
    if (target.page === "control" && target.section === "apps" && target.query) {
      setSearch(target.query);
    }
  }, [target]);

  useEffect(() => {
    let cancelled = false;
    invoke<ProcessInfo[]>("search_processes", { query: search }).then(
      (found) => !cancelled && setProcesses(found)
    );
    return () => {
      cancelled = true;
    };
  }, [search]);

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
        <input
          className="port-input"
          placeholder="Search by name or PID…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select
          className="port-input"
          value={pid}
          onChange={(e) => setPid(e.target.value)}
        >
          <option value="">
            {processes.length} match{processes.length === 1 ? "" : "es"} — select a process…
          </option>
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
        <button className="ctl-btn" onClick={() => act("watch_process", {})}>
          Tell me when it's done
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
        <label>IO class (1 realtime / 2 best-effort / 3 idle) + level (0-7)</label>
        <input
          className="port-input small"
          value={ioClass}
          onChange={(e) => setIoClass(e.target.value)}
        />
        <input
          className="port-input small"
          value={ioLevel}
          onChange={(e) => setIoLevel(e.target.value)}
        />
        <button
          className="ctl-btn"
          onClick={() =>
            act("set_ionice", {
              class: parseInt(ioClass, 10),
              level: parseInt(ioLevel, 10),
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
  const { target } = useContext(NavContext);
  const [units, setUnits] = useState<UnitInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<ActionResult | null>(null);
  const [filter, setFilter] = useState("");
  const [logsFor, setLogsFor] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[] | null>(null);

  useEffect(() => {
    if (target.page === "control" && target.section === "services" && target.query) {
      setFilter(target.query);
    }
  }, [target]);

  async function showLogs(name: string) {
    if (logsFor === name) return setLogsFor(null);
    setLogsFor(name);
    setLogs(null);
    try {
      setLogs(await invoke<string[]>("get_service_logs", { name, user: false }));
    } catch (e) {
      setLogs([String(e)]);
    }
  }

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
      <div className="proc-picker">
        <input
          className="port-input"
          placeholder="Filter services…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className="ctl-btn" onClick={refresh} disabled={loading} style={{ flex: "none" }}>
          {loading ? "Refreshing…" : "Refresh"}
        </button>
      </div>
      <div className="unit-list">
        {units
          .filter((u) => `${u.name} ${u.description}`.toLowerCase().includes(filter.toLowerCase()))
          .map((u) => (
          <div key={u.name}>
          <div className="unit-row">
            <span className="unit-name" title={`${u.name} — ${u.description}`}>
              {u.name}
            </span>
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
              <button className="ctl-btn-sm" onClick={() => showLogs(u.name)}>
                {logsFor === u.name ? "Hide log" : "Log"}
              </button>
            </div>
          </div>
          {logsFor === u.name && (
            <pre className="log-view">
              {logs === null ? "Reading the journal…" : logs.length === 0 ? "No log lines for this unit." : logs.join("\n")}
            </pre>
          )}
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
  const [detail, setDetail] = useState<{ id: string; kind: "logs" | "stats"; text: string } | null>(null);

  async function show(id: string, kind: "logs" | "stats") {
    if (detail?.id === id && detail.kind === kind) return setDetail(null);
    setDetail({ id, kind, text: kind === "logs" ? "Reading the container's output…" : "Measuring…" });
    try {
      if (kind === "logs") {
        const text = await invoke<string>("get_container_logs", { id });
        setDetail({ id, kind, text: text.trim() || "This container hasn't printed anything." });
      } else {
        const s = await invoke<Record<string, string>>("get_container_stats", { id });
        setDetail({
          id,
          kind,
          text: `CPU        ${s.cpu}\nMemory     ${s.memory} (${s.memory_percent})\nNetwork    ${s.network}  (received / sent)\nDisk       ${s.disk}  (read / written)\nProcesses  ${s.processes}`,
        });
      }
    } catch (e) {
      setDetail({ id, kind, text: String(e) });
    }
  }

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
            <div key={c.id}>
            <div className="unit-row">
              <span className="unit-name" title={`${c.name || c.id} — ${c.image}`}>
                {c.name || c.id}
              </span>
              <span className="unit-state" title={c.status}>
                {c.status}
              </span>
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
                <button className="ctl-btn-sm" onClick={() => show(c.id, "logs")}>
                  Log
                </button>
                <button className="ctl-btn-sm" onClick={() => show(c.id, "stats")}>
                  Stats
                </button>
              </div>
            </div>
            {detail?.id === c.id && <pre className="log-view">{detail.text}</pre>}
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
  const [minClock, setMinClock] = useState("300");
  const [maxClock, setMaxClock] = useState("1800");
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
      <div className="control-row">
        <label>Lock GPU clock range (MHz, NVIDIA only)</label>
        <input className="port-input small" value={minClock} onChange={(e) => setMinClock(e.target.value)} />
        <input className="port-input small" value={maxClock} onChange={(e) => setMaxClock(e.target.value)} />
        <button
          className="ctl-btn"
          onClick={() =>
            act("set_gpu_clock_lock", {
              minMhz: parseInt(minClock, 10),
              maxMhz: parseInt(maxClock, 10),
            })
          }
        >
          Lock
        </button>
        <button className="ctl-btn" onClick={() => act("reset_gpu_clock_lock", {})}>
          Reset to default
        </button>
      </div>

      <StatusLine result={result} />
      <p className="diagnose-hint">
        Most GPU controls require root and a supported driver — expect
        permission errors unless running with elevated privileges. Clock
        locking only accepts values within the driver's reported supported
        range (see clocks.max.graphics) — out-of-range requests are rejected,
        not clamped silently, and the result shows the driver's read-back of
        what was actually applied.
      </p>
    </section>
  );
}

function SystemControl() {
  const [brightness, setBrightnessState] = useState<number | null>(null);
  const [volume, setVolumeState] = useState<{ percent: number; muted: boolean } | null>(
    null
  );
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    const b = await invoke<{ percent: number } | null>("get_brightness");
    setBrightnessState(b?.percent ?? null);
    const v = await invoke<{ percent: number; muted: boolean } | null>("get_volume");
    setVolumeState(v);
  }

  useEffect(() => {
    refresh();
  }, []);

  async function act(cmd: string, args: Record<string, unknown>) {
    try {
      const r = await invoke<ActionResult>(cmd, args);
      setResult(r);
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>Brightness &amp; Volume</h2>
      {brightness !== null && (
        <div className="control-row">
          <label>Brightness</label>
          <input
            type="range"
            min={1}
            max={100}
            value={brightness}
            onChange={(e) => act("set_brightness", { percent: parseInt(e.target.value, 10) })}
          />
          <span>{brightness}%</span>
        </div>
      )}
      {volume && (
        <div className="control-row">
          <label>Volume</label>
          <input
            type="range"
            min={0}
            max={150}
            value={volume.percent}
            onChange={(e) => act("set_volume", { percent: parseInt(e.target.value, 10) })}
          />
          <span>{volume.percent}%</span>
          <button className="ctl-btn-sm" onClick={() => act("toggle_mute", {})}>
            {volume.muted ? "Unmute" : "Mute"}
          </button>
        </div>
      )}
      <StatusLine result={result} />
    </section>
  );
}

function StartupImpact() {
  const [entries, setEntries] = useState<{ unit: string; time_ms: number }[]>([]);

  useEffect(() => {
    invoke<{ unit: string; time_ms: number }[]>("startup_impact").then(setEntries);
  }, []);

  if (entries.length === 0) return null;

  const maxMs = Math.max(...entries.map((e) => e.time_ms), 1);
  const seconds = (ms: number) =>
    ms >= 60000
      ? `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`
      : `${(ms / 1000).toFixed(1)}s`;

  return (
    <section className="panel">
      <h2>Startup Impact</h2>
      <div className="unit-list">
        {entries.map((e) => (
          <div key={e.unit} className="startup-row">
            <span className="unit-name" title={e.unit}>
              {e.unit}
            </span>
            <div className="bar-track">
              <div
                className="bar-fill bar-ok"
                style={{ width: `${(e.time_ms / maxMs) * 100}%` }}
              />
            </div>
            <span className="bar-value">{seconds(e.time_ms)}</span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">
        How long each unit took to start at the last boot, slowest first.
        Units start in parallel, so these don't add up to the boot time.
      </p>
    </section>
  );
}

function AutostartControl() {
  const [entries, setEntries] = useState<
    { filename: string; name: string; enabled: boolean; system_wide: boolean }[]
  >([]);
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    const e = await invoke<typeof entries>("list_autostart");
    setEntries(e);
  }

  useEffect(() => {
    refresh();
  }, []);

  async function toggle(filename: string, enabled: boolean) {
    try {
      const r = await invoke<ActionResult>("set_autostart_enabled", { filename, enabled });
      setResult(r);
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  if (entries.length === 0) return null;

  return (
    <section className="panel">
      <h2>Autostart Apps</h2>
      <div className="unit-list">
        {entries.map((e) => (
          <div key={e.filename} className="unit-row">
            <span className="unit-name">{e.name}</span>
            <span className={"unit-state " + (e.enabled ? "unit-active" : "unit-inactive")}>
              {e.enabled ? "enabled" : "disabled"}
            </span>
            <div className="unit-buttons">
              <button
                className="ctl-btn-sm"
                onClick={() => toggle(e.filename, !e.enabled)}
              >
                {e.enabled ? "Disable" : "Enable"}
              </button>
            </div>
          </div>
        ))}
      </div>
      <StatusLine result={result} />
    </section>
  );
}

function ClipboardHistory() {
  const [items, setItems] = useState<string[]>([]);

  useEffect(() => {
    const load = () => invoke<string[]>("get_clipboard_history").then(setItems);
    load();
    const id = setInterval(load, 10000);
    return () => clearInterval(id);
  }, []);

  if (items.length === 0) return null;

  return (
    <section className="panel">
      <h2>Clipboard History</h2>
      <div className="unit-list">
        {items.map((text, i) => (
          <div key={i} className="unit-row">
            <span className="unit-name" title={text}>
              {text.length > 80 ? text.slice(0, 80) + "…" : text}
            </span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">Polled every 10s, last 50 entries kept in memory.</p>
    </section>
  );
}

interface ConnectionInfo {
  pid: number;
  process_name: string;
  local_addr: string;
  remote_addr: string;
  state: string;
}

function NetworkControl() {
  const [connections, setConnections] = useState<ConnectionInfo[]>([]);
  const [interfaces, setInterfaces] = useState<string[]>([]);
  const [iface, setIface] = useState("");
  const [rate, setRate] = useState("2000");
  const [blockPid, setBlockPid] = useState("");
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    const [conns, ifaces] = await Promise.all([
      invoke<ConnectionInfo[]>("list_connections", { pid: null }),
      invoke<string[]>("list_network_interfaces"),
    ]);
    setConnections(conns.filter((c) => c.state === "ESTABLISHED").slice(0, 30));
    setInterfaces(ifaces);
    if (!iface && ifaces.length > 0) setIface(ifaces[0]);
  }

  useEffect(() => {
    refresh();
  }, []);

  async function act(cmd: string, args: Record<string, unknown>) {
    try {
      const r = await invoke<ActionResult>(cmd, args);
      setResult(r);
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>Network</h2>
      <div className="unit-list">
        {connections.map((c, i) => (
          <div key={i} className="unit-row">
            <span className="unit-name">
              {c.process_name} (PID {c.pid})
            </span>
            <span className="unit-state">{c.remote_addr}</span>
          </div>
        ))}
      </div>

      <div className="control-row">
        <label>Limit interface bandwidth</label>
        <select className="port-input small" value={iface} onChange={(e) => setIface(e.target.value)}>
          {interfaces.map((i) => (
            <option key={i} value={i}>{i}</option>
          ))}
        </select>
        <input className="port-input small" value={rate} onChange={(e) => setRate(e.target.value)} placeholder="kbit/s" />
        <button className="ctl-btn" onClick={() => act("limit_interface_bandwidth", { iface, rateKbit: parseInt(rate, 10) })}>
          Apply
        </button>
        <button className="ctl-btn" onClick={() => act("clear_interface_bandwidth_limit", { iface })}>
          Clear
        </button>
      </div>

      <div className="control-row">
        <label>Block network for PID (whole cgroup)</label>
        <input className="port-input small" value={blockPid} onChange={(e) => setBlockPid(e.target.value)} placeholder="PID" />
        <button className="ctl-btn ctl-danger" onClick={() => act("block_process_network", { pid: parseInt(blockPid, 10) })}>
          Block
        </button>
        <button className="ctl-btn" onClick={() => act("unblock_all_network", {})}>
          Unblock All
        </button>
      </div>

      <StatusLine result={result} />
      <p className="diagnose-hint">
        Bandwidth limiting and network blocking need CAP_NET_ADMIN (usually root) —
        you'll get a clear error otherwise. Blocking affects the process's whole
        cgroup, not just one PID.
      </p>
    </section>
  );
}

interface UsbDevice {
  device_id: string;
  vendor_id: string;
  product_id: string;
  manufacturer: string;
  product: string;
  authorized: boolean;
}

function UsbControl() {
  const [devices, setDevices] = useState<UsbDevice[]>([]);
  const [result, setResult] = useState<ActionResult | null>(null);

  async function refresh() {
    setDevices(await invoke<UsbDevice[]>("list_usb_devices"));
  }

  useEffect(() => {
    refresh();
  }, []);

  async function toggle(deviceId: string, authorized: boolean) {
    try {
      const r = await invoke<ActionResult>("set_usb_authorized", { deviceId, authorized });
      setResult(r);
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  if (devices.length === 0) return null;

  return (
    <section className="panel">
      <h2>USB Devices</h2>
      <div className="unit-list">
        {devices.map((d) => (
          <div key={d.device_id} className="unit-row">
            <span className="unit-name">
              {d.manufacturer || d.vendor_id} {d.product} ({d.device_id})
            </span>
            <span className={"unit-state " + (d.authorized ? "unit-active" : "unit-inactive")}>
              {d.authorized ? "authorized" : "blocked"}
            </span>
            <div className="unit-buttons">
              <button className="ctl-btn-sm ctl-danger" onClick={() => toggle(d.device_id, !d.authorized)}>
                {d.authorized ? "Deauthorize" : "Authorize"}
              </button>
            </div>
          </div>
        ))}
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Deauthorizing your keyboard/mouse this way can lock you out until you
        unplug and replug it — double-check the device first.
      </p>
    </section>
  );
}

interface WindowInfo {
  address: string;
  class: string;
  title: string;
  pid: number;
  workspace: { id: number; name: string };
}

function WindowControl() {
  const [windows, setWindows] = useState<WindowInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ActionResult | null>(null);
  const [targetWs, setTargetWs] = useState("1");

  async function refresh() {
    try {
      setWindows(await invoke<WindowInfo[]>("list_windows"));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function act(cmd: string, args: Record<string, unknown>) {
    try {
      const r = await invoke<ActionResult>(cmd, args);
      setResult(r);
      refresh();
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    }
  }

  if (error) {
    return (
      <section className="panel">
        <h2>Windows &amp; Workspaces</h2>
        <p className="diagnose-hint">{error}</p>
      </section>
    );
  }

  if (!windows) return null;

  return (
    <section className="panel">
      <h2>Windows &amp; Workspaces</h2>
      <div className="unit-list">
        {windows.map((w) => (
          <div key={w.address} className="unit-row">
            <span className="unit-name" title={w.title}>
              {w.class} <span className="mem-muted">{w.title}</span>
            </span>
            <span className="unit-state">ws {w.workspace.name}</span>
            <div className="unit-buttons">
              <input
                className="port-input small"
                style={{ width: 44 }}
                aria-label="Target workspace"
                value={targetWs}
                onChange={(e) => setTargetWs(e.target.value)}
              />
              <button
                className="ctl-btn-sm"
                onClick={() =>
                  act("move_window_to_workspace", {
                    address: w.address,
                    workspace: parseInt(targetWs, 10),
                  })
                }
              >
                Move
              </button>
              <button className="ctl-btn-sm ctl-danger" onClick={() => act("close_window", { address: w.address })}>
                Close
              </button>
            </div>
          </div>
        ))}
      </div>
      <StatusLine result={result} />
    </section>
  );
}

interface NamespaceGroup {
  ns_type: string;
  inode: string;
  pids: number[];
  process_names: string[];
}

function NamespaceControl() {
  const [groups, setGroups] = useState<NamespaceGroup[]>([]);
  const [filter, setFilter] = useState<string>("net");

  useEffect(() => {
    invoke<NamespaceGroup[]>("list_namespaces").then(setGroups);
  }, []);

  const types = [...new Set(groups.map((g) => g.ns_type))];
  const shown = groups.filter((g) => g.ns_type === filter);

  return (
    <section className="panel">
      <h2>Namespaces</h2>
      <div className="range-picker">
        {types.map((t) => (
          <button
            key={t}
            className={t === filter ? "range-btn active" : "range-btn"}
            onClick={() => setFilter(t)}
          >
            {t}
          </button>
        ))}
      </div>
      <div className="unit-list">
        {shown.map((g) => (
          <div key={g.inode} className="unit-row">
            <span className="unit-name">
              {g.ns_type}:[{g.inode}] — {g.pids.length} process(es)
            </span>
            <span className="unit-state" title={g.process_names.join(", ")}>
              {g.process_names.slice(0, 3).join(", ")}
              {g.process_names.length > 3 ? "…" : ""}
            </span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">
        Live view of which processes share a kernel namespace — same idea as
        `lsns`, but grouped and filterable. A namespace with only one or two
        processes is usually a container or sandbox boundary worth a closer
        look.
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
        Ordered lists of actions you can dry-run, apply and undo. Build one
        below, or hand-edit the TOML files under your Trace data directory.
      </p>
      <div className="control-actions">
        <button
          className="ctl-btn"
          onClick={async () => {
            const path = await openDialog({
              multiple: false,
              filters: [{ name: "Trace profile", extensions: ["toml"] }],
            });
            if (typeof path !== "string") return;
            const r = await invoke<ActionResult>("import_profile_file", { path });
            setOutcomes([{ description: "Import", ok: r.ok, message: r.message }]);
            setActiveProfile(null);
            refresh();
          }}
        >
          Import a profile file…
        </button>
      </div>
      {activeProfile === null && outcomes && (
        <div className="outcome-list">
          {outcomes.map((o, i) => (
            <div key={i} className={o.ok ? "status-ok" : "status-fail"}>
              {o.ok ? "✓" : "✗"} {o.message}
            </div>
          ))}
        </div>
      )}
      {profiles.length === 0 && (
        <p className="diagnose-hint">
          No profiles yet — build one below.
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
    case "FreezeApp":
      return `Freeze every ${a.app} process`;
    case "ResumeApp":
      return `Resume every ${a.app} process`;
    case "QuitApp":
      return `Quit ${a.app}`;
    case "SetPowerProfile":
      return `Switch power profile to ${a.profile}`;
    case "ApplyPreset":
      return `Apply the “${a.preset}” settings preset`;
    case "RunCommand":
      return `Run: ${a.command}`;
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

const SECTIONS = [
  { key: "apps", label: "Apps & Processes" },
  { key: "power", label: "Power & Memory" },
  { key: "network", label: "Network" },
  { key: "devices", label: "Display & Devices" },
  { key: "services", label: "Services" },
  { key: "automation", label: "Automation" },
  { key: "packages", label: "Packages & Cleanup" },
] as const;

type SectionKey = (typeof SECTIONS)[number]["key"];

export default function Control() {
  const { target } = useContext(NavContext);
  const [section, setSection] = useState<SectionKey>("apps");

  // Global search and other pages can land on a specific section.
  useEffect(() => {
    if (target.page === "control" && SECTIONS.some((s) => s.key === target.section)) {
      setSection(target.section as SectionKey);
    }
  }, [target]);
  // Bumped when the builder saves a profile, so the list above it reloads.
  const [profilesVersion, setProfilesVersion] = useState(0);

  return (
    <div className="page-flow control-flow">
      <div className="range-picker section-picker">
        {SECTIONS.map((s) => (
          <button
            key={s.key}
            className={s.key === section ? "range-btn active" : "range-btn"}
            onClick={() => setSection(s.key)}
          >
            {s.label}
          </button>
        ))}
      </div>

      {section === "apps" && (
        <>
          <ProcessControl />
          <ListeningPorts />
          <RunRestricted />
          <WindowControl />
          <NamespaceControl />
        </>
      )}
      {section === "power" && (
        <>
          <SessionControl />
          <TimersControl />
          <CpuPowerControl />
          <FanControl />
          <MemoryTuningControl />
          <SystemControl />
          <GpuControl />
        </>
      )}
      {section === "network" && (
        <>
          <VpnControl />
          <ConnectionDoctor />
          <DataBudgets />
          <SavedWifiCard />
          <WirelessControl />
          <FirewallControl />
          <NetworkControl />
        </>
      )}
      {section === "devices" && (
        <>
          <DisplayControl />
          <AudioControl />
          <UsbControl />
          <ClipboardHistory />
        </>
      )}
      {section === "services" && (
        <>
          <ServiceControl />
          <DockerControl />
          <ScheduledJobs />
          <AutostartControl />
          <StartupImpact />
        </>
      )}
      {section === "automation" && (
        <>
          <ProfilesControl key={profilesVersion} />
          <Automation onProfilesChanged={() => setProfilesVersion((v) => v + 1)} />
          <AppSettings />
          <ActionLog />
        </>
      )}
      {section === "packages" && (
        <>
          <Packages />
          <CleanupControl />
          <PrivacyCleaner />
        </>
      )}
    </div>
  );
}
