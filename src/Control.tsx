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
  const [ioClass, setIoClass] = useState("2");
  const [ioLevel, setIoLevel] = useState("4");
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

  return (
    <section className="panel">
      <h2>Startup Impact</h2>
      {entries.slice(0, 15).map((e) => (
        <div key={e.unit} className="bar-row">
          <span className="bar-label" title={e.unit}>
            {e.unit.length > 22 ? e.unit.slice(0, 22) + "…" : e.unit}
          </span>
          <div className="bar-track">
            <div
              className="bar-fill bar-ok"
              style={{ width: `${(e.time_ms / maxMs) * 100}%` }}
            />
          </div>
          <span className="bar-value">{e.time_ms}ms</span>
        </div>
      ))}
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
              {w.class} — ws {w.workspace.name}
            </span>
            <div className="unit-buttons">
              <input
                className="port-input small"
                style={{ width: 40 }}
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
      <SystemControl />
      <NetworkControl />
      <ServiceControl />
      <DockerControl />
      <GpuControl />
      <UsbControl />
      <WindowControl />
      <NamespaceControl />
      <AutostartControl />
      <StartupImpact />
      <ClipboardHistory />
      <ProfilesControl />
      <ActionLog />
    </div>
  );
}
