import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CpuPowerControl, FanControl } from "./ControlTuning";
import {
  ConfirmButton,
  Segmented,
  Sparkline,
  StatusLine,
  bytesToHuman,
  useAction,
  usePoll,
} from "./ui";

interface Sensor {
  chip: string;
  label: string;
  kind: "temp" | "fan" | "voltage" | "power" | "current" | "frequency";
  value: number;
  unit: string;
}

interface GpuDevice {
  address: string;
  vendor: string;
  driver: string;
  primary: boolean;
  power_state: string;
  apps: { app: string; processes: number; memory_bytes: number; busy_percent: number }[];
}

interface HardwareView {
  sensors: Sensor[];
  gpus: GpuDevice[];
  peripherals: { name: string; kind: string; percent: number; state: string }[];
  backlight_percent: number | null;
}

interface MonitorSetup {
  name: string;
  mode: string;
  x: number;
  y: number;
  scale: number;
  transform: number;
  vrr: boolean;
  mirror_of: string;
  enabled: boolean;
}

interface MonitorInfo {
  setup: MonitorSetup;
  description: string;
  available_modes: string[];
  width_mm: number;
  height_mm: number;
}

interface NightLight {
  enabled: boolean;
  kelvin: number;
  from: string;
  until: string;
}

interface DisplaysView {
  monitors: MonitorInfo[];
  config: { layouts: { name: string; auto: boolean; monitors: MonitorSetup[] }[]; night_light: NightLight };
  night_light_kelvin: number | null;
}

interface ExternalDisplay {
  number: number;
  connector: string;
  model: string;
  brightness: number | null;
}

const SENSOR_COLOR: Record<Sensor["kind"], string> = {
  temp: "#d95926",
  fan: "#3987e5",
  voltage: "#199e70",
  power: "#c98500",
  current: "#d55181",
  frequency: "#9085e9",
};

const ROTATIONS = ["Normal", "90°", "180°", "270°"];

function format(value: number, unit: string): string {
  const digits = unit === "RPM" || unit === "MHz" || unit === "°C" ? 0 : 2;
  return `${value.toFixed(digits)} ${unit}`;
}

/** Live readings with the range seen and a trend since the page was opened. */
function Sensors({ sensors }: { sensors: Sensor[] }) {
  const history = useRef(new Map<string, number[]>());
  for (const s of sensors) {
    const key = `${s.chip}/${s.label}/${s.kind}`;
    const values = history.current.get(key) ?? [];
    values.push(s.value);
    if (values.length > 90) values.shift();
    history.current.set(key, values);
  }
  const chips = [...new Set(sensors.map((s) => s.chip))];

  return (
    <section className="panel wide">
      <h2>Sensors ({sensors.length})</h2>
      <div className="sensor-grid">
        {chips.map((chip) => (
          <div key={chip} className="sensor-chip">
            <h3 className="sub-heading">{chip}</h3>
            {sensors
              .filter((s) => s.chip === chip)
              .map((s) => {
                const values = history.current.get(`${s.chip}/${s.label}/${s.kind}`) ?? [];
                return (
                  <div key={s.label + s.kind} className="sensor-row">
                    <span className="folder-name">{s.label}</span>
                    <Sparkline values={values} color={SENSOR_COLOR[s.kind]} width={90} height={22} />
                    <span className="sensor-value">{format(s.value, s.unit)}</span>
                    <span className="mem-muted sensor-range">
                      {values.length > 1
                        ? `${format(Math.min(...values), "")}– ${format(Math.max(...values), s.unit)}`
                        : ""}
                    </span>
                  </div>
                );
              })}
          </div>
        ))}
      </div>
      <p className="diagnose-hint">
        Every reading the hardware reports to Linux, refreshed every 2 seconds. The range and the
        trend line cover the time this page has been open.
      </p>
    </section>
  );
}

function Graphics({ gpus }: { gpus: GpuDevice[] }) {
  return (
    <section className="panel">
      <h2>Graphics Cards</h2>
      {gpus.map((g) => (
        <div key={g.address} className="gpu-card">
          <div className="disk-info">
            <span className="disk-name">
              {g.vendor} <span className="mem-muted">{g.primary ? "drives the screen" : "dedicated"} · driver {g.driver}</span>
            </span>
            <span className={g.power_state === "suspended" ? "status-ok" : "mem-muted"}>
              {g.power_state === "suspended" ? "✓ powered down" : g.power_state || "unknown"}
            </span>
          </div>
          {g.apps.length === 0 ? (
            <p className="diagnose-hint">
              {g.power_state === "suspended"
                ? "Nothing is using it, so it is switched off and costing no battery."
                : "No program of yours is using it right now."}
            </p>
          ) : (
            <table className="proc-table">
              <thead>
                <tr>
                  <th>Using it</th>
                  <th>Busy</th>
                  <th>Video memory</th>
                </tr>
              </thead>
              <tbody>
                {g.apps.slice(0, 8).map((a) => (
                  <tr key={a.app}>
                    <td>
                      {a.app}
                      {a.processes > 1 && <span className="mem-muted"> ({a.processes})</span>}
                    </td>
                    <td>{a.busy_percent >= 0.5 ? `${a.busy_percent.toFixed(0)}%` : "idle"}</td>
                    <td>{bytesToHuman(a.memory_bytes)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {!g.primary && g.power_state !== "suspended" && g.apps.length > 0 && (
            <p className="diagnose-hint">
              These programs are keeping the dedicated card awake, which costs several watts on
              battery. Closing them lets it power down.
            </p>
          )}
        </div>
      ))}
      {gpus.length === 0 && <p className="diagnose-hint">No graphics card found.</p>}
    </section>
  );
}

function Brightness({ builtin }: { builtin: number | null }) {
  const [level, setLevel] = useState<number | null>(null);
  const [external, setExternal] = useState<ExternalDisplay[] | null>(null);
  const [looking, setLooking] = useState(false);
  const { result, run } = useAction();
  const shown = level ?? builtin;

  async function detect() {
    setLooking(true);
    try {
      setExternal(await invoke<ExternalDisplay[]>("get_external_displays"));
    } finally {
      setLooking(false);
    }
  }

  return (
    <section className="panel">
      <h2>Brightness</h2>
      {shown != null && (
        <div className="control-row">
          <label>Built-in screen ({shown}%)</label>
          <input
            type="range"
            min={1}
            max={100}
            value={shown}
            onChange={(e) => setLevel(parseInt(e.target.value, 10))}
            onMouseUp={() => level != null && run("set_brightness", { percent: level })}
            onKeyUp={() => level != null && run("set_brightness", { percent: level })}
          />
        </div>
      )}
      {(external ?? []).map((d) => (
        <div key={d.number} className="control-row">
          <label>
            {d.connector} <span className="mem-muted">{d.model}</span>
            {d.brightness != null && ` (${d.brightness}%)`}
          </label>
          {d.brightness == null ? (
            <span className="mem-muted">doesn't accept brightness commands</span>
          ) : (
            <input
              type="range"
              min={0}
              max={100}
              defaultValue={d.brightness}
              onMouseUp={(e) =>
                run("set_external_brightness", {
                  number: d.number,
                  percent: parseInt((e.target as HTMLInputElement).value, 10),
                }).then(detect)
              }
            />
          )}
        </div>
      ))}
      <div className="control-actions">
        <button className="ctl-btn" onClick={detect} disabled={looking}>
          {looking ? "Asking the monitors…" : external ? "Look again" : "Find external monitors"}
        </button>
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        External monitors are dimmed through the cable (DDC/CI), the same as pressing their own
        buttons. It takes a second or two per monitor, and a few monitors don't support it.
      </p>
    </section>
  );
}

function MonitorEditor({
  info,
  others,
  draft,
  onChange,
}: {
  info: MonitorInfo;
  others: string[];
  draft: MonitorSetup;
  onChange: (next: MonitorSetup) => void;
}) {
  const set = (patch: Partial<MonitorSetup>) => onChange({ ...draft, ...patch });
  return (
    <div className="monitor-card">
      <div className="disk-info">
        <span className="disk-name">
          {draft.name} <span className="mem-muted">{info.description}</span>
        </span>
        <Segmented
          options={["on", "off"]}
          value={draft.enabled ? "on" : "off"}
          onChange={(v) => set({ enabled: v === "on" })}
        />
      </div>
      {draft.enabled && (
        <>
          <div className="control-row">
            <label>Resolution and refresh rate</label>
            <select className="port-input" value={draft.mode} onChange={(e) => set({ mode: e.target.value })}>
              {info.available_modes.map((m) => (
                <option key={m} value={m}>
                  {m.replace("@", " @ ")}
                </option>
              ))}
            </select>
          </div>
          <div className="control-row">
            <label>Scale</label>
            <Segmented
              options={["1", "1.25", "1.5", "2"]}
              value={String(draft.scale)}
              onChange={(v) => set({ scale: parseFloat(v) })}
            />
          </div>
          <div className="control-row">
            <label>Rotation</label>
            <Segmented
              options={ROTATIONS}
              value={ROTATIONS[draft.transform] ?? null}
              onChange={(v) => set({ transform: ROTATIONS.indexOf(v) })}
            />
          </div>
          <div className="control-row">
            <label>Position (pixels from the top-left of the desktop)</label>
            <input
              className="port-input number-box"
              value={draft.x}
              aria-label="Horizontal position"
              onChange={(e) => set({ x: parseInt(e.target.value, 10) || 0 })}
            />
            <input
              className="port-input number-box"
              value={draft.y}
              aria-label="Vertical position"
              onChange={(e) => set({ y: parseInt(e.target.value, 10) || 0 })}
            />
          </div>
          <div className="control-row">
            <label>Adaptive sync on this monitor</label>
            <Segmented
              options={["on", "off"]}
              value={draft.vrr ? "on" : "off"}
              onChange={(v) => set({ vrr: v === "on" })}
            />
          </div>
          {others.length > 0 && (
            <div className="control-row">
              <label>Show the same picture as</label>
              <select
                className="port-input small-select"
                value={draft.mirror_of}
                onChange={(e) => set({ mirror_of: e.target.value })}
              >
                <option value="">Nothing — its own desktop</option>
                {others.map((o) => (
                  <option key={o} value={o}>
                    {o}
                  </option>
                ))}
              </select>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function Displays() {
  const { data, reload } = usePoll<DisplaysView>("get_displays");
  const { result, busy, run } = useAction(reload);
  const [drafts, setDrafts] = useState<Record<string, MonitorSetup>>({});
  const [layoutName, setLayoutName] = useState("");
  const [auto, setAuto] = useState(true);
  const [night, setNight] = useState<NightLight | null>(null);

  // Start each edit from what the monitors are actually doing.
  useEffect(() => {
    if (data) {
      setDrafts(Object.fromEntries(data.monitors.map((m) => [m.setup.name, m.setup])));
      setNight((n) => n ?? { ...data.config.night_light, kelvin: data.config.night_light.kelvin || 4000, from: data.config.night_light.from || "20:00", until: data.config.night_light.until || "07:00" });
    }
  }, [data]);

  if (!data) return null;
  if (data.monitors.length === 0) {
    return (
      <section className="panel">
        <h2>Displays</h2>
        <p className="diagnose-hint">Arranging displays needs Hyprland — no monitors were reported.</p>
      </section>
    );
  }
  const changed = data.monitors.filter((m) => JSON.stringify(m.setup) !== JSON.stringify(drafts[m.setup.name] ?? m.setup));
  // Side-by-side helper: lays enabled monitors out left to right in the listed order.
  const sideBySide = () => {
    let x = 0;
    const next = { ...drafts };
    for (const m of data.monitors) {
      const d = next[m.setup.name];
      if (!d || !d.enabled) continue;
      const [w, h] = d.mode.split("@")[0].split("x").map((n) => parseInt(n, 10));
      const width = Math.round((d.transform % 2 === 1 ? h : w) / d.scale);
      next[m.setup.name] = { ...d, x, y: 0, mirror_of: "" };
      x += width;
    }
    setDrafts(next);
  };

  return (
    <section className="panel wide">
      <h2>Displays ({data.monitors.length})</h2>
      <div className="monitor-grid">
        {data.monitors.map((m) => (
          <MonitorEditor
            key={m.setup.name}
            info={m}
            others={data.monitors.filter((o) => o.setup.name !== m.setup.name).map((o) => o.setup.name)}
            draft={drafts[m.setup.name] ?? m.setup}
            onChange={(next) => setDrafts({ ...drafts, [m.setup.name]: next })}
          />
        ))}
      </div>
      <div className="control-actions">
        <ConfirmButton
          label={changed.length > 0 ? `Apply changes to ${changed.length} monitor(s)` : "No changes"}
          confirmLabel="Apply — a wrong mode can blank a screen. Confirm"
          className="ctl-btn ctl-primary"
          disabled={busy || changed.length === 0}
          onConfirm={() => run("apply_monitors", { setups: changed.map((m) => drafts[m.setup.name]) })}
        />
        {data.monitors.length > 1 && (
          <button className="ctl-btn" onClick={sideBySide}>
            Arrange side by side
          </button>
        )}
        <button className="ctl-btn" disabled={changed.length === 0} onClick={reload}>
          Discard
        </button>
      </div>
      <StatusLine result={result} />

      <h3 className="sub-heading">Saved layouts</h3>
      {data.config.layouts.length === 0 && (
        <p className="diagnose-hint">None yet. Arrange your monitors, then save the arrangement under a name.</p>
      )}
      {data.config.layouts.map((l) => (
        <div key={l.name} className="control-row">
          <label>
            {l.name}{" "}
            <span className="mem-muted">
              {l.monitors.map((m) => m.name).join(" + ")}
              {l.auto && " · applies by itself when exactly these are connected"}
            </span>
          </label>
          <button className="ctl-btn" disabled={busy} onClick={() => run("apply_display_layout", { name: l.name })}>
            Apply
          </button>
          <ConfirmButton label="Delete" disabled={busy} onConfirm={() => run("delete_display_layout", { name: l.name })} />
        </div>
      ))}
      <div className="control-row">
        <label>Save the current arrangement as</label>
        <input
          className="port-input"
          placeholder="Docked, Presentation, On the sofa…"
          value={layoutName}
          onChange={(e) => setLayoutName(e.target.value)}
        />
        <label className="check-row">
          <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} />
          apply automatically
        </label>
        <button
          className="ctl-btn"
          disabled={busy || !layoutName.trim()}
          onClick={() => {
            run("save_display_layout", { name: layoutName, auto });
            setLayoutName("");
          }}
        >
          Save
        </button>
      </div>

      {night && data.night_light_kelvin != null && (
        <>
          <h3 className="sub-heading">Night light schedule</h3>
          <div className="control-row">
            <label>Warm the screen every evening ({night.kelvin} K)</label>
            <Segmented
              options={["on", "off"]}
              value={night.enabled ? "on" : "off"}
              onChange={(v) => setNight({ ...night, enabled: v === "on" })}
            />
            <input type="range" min={2500} max={6000} step={100} value={night.kelvin} onChange={(e) => setNight({ ...night, kelvin: parseInt(e.target.value, 10) })} />
            <input className="port-input small" type="time" value={night.from} aria-label="From" onChange={(e) => setNight({ ...night, from: e.target.value })} />
            <input className="port-input small" type="time" value={night.until} aria-label="Until" onChange={(e) => setNight({ ...night, until: e.target.value })} />
            <button className="ctl-btn" disabled={busy} onClick={() => run("set_night_schedule", { night })}>
              Save
            </button>
          </div>
        </>
      )}
      <p className="diagnose-hint">
        Changes apply to this session; your Hyprland config is not rewritten, so logging out restores
        it. Saved layouts and the night-light schedule are applied by Trace while it is running.
      </p>
    </section>
  );
}

export default function Hardware() {
  const { data } = usePoll<HardwareView>("get_hardware", undefined, 2000);
  if (!data) return <p className="loading">Reading the hardware…</p>;

  return (
    <>
      <Sensors sensors={data.sensors} />
      <FanControl />
      <CpuPowerControl />
      <Graphics gpus={data.gpus} />
      <Brightness builtin={data.backlight_percent} />
      {data.peripherals.length > 0 && (
        <section className="panel">
          <h2>Wireless Devices</h2>
          {data.peripherals.map((p, i) => (
            <div key={i} className="bar-row">
              <span className="bar-label" title={p.name}>
                {p.name}
              </span>
              <div className="bar-track">
                <div
                  className={`bar-fill ${p.percent < 15 ? "bar-danger" : p.percent < 30 ? "bar-warn" : "bar-ok"}`}
                  style={{ width: `${p.percent}%` }}
                />
              </div>
              <span className="bar-value">{p.percent}%</span>
            </div>
          ))}
          <p className="diagnose-hint">Battery levels of wireless mice, keyboards and headsets that report one.</p>
        </section>
      )}
      <Displays />
    </>
  );
}
