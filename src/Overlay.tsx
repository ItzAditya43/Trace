import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Segmented, usePoll } from "./ui";

interface OverlayConfig {
  open_at_start: boolean;
  layout: string;
  corner: string;
  monitor: string;
  backdrop: number;
  text_scale: number;
  items: string[];
}

interface OverlayView {
  config: OverlayConfig;
  open: boolean;
  layout: string;
  width: number;
  height: number;
  anchor: string;
  monitors: string[];
  desktop_support: boolean;
  mangohud: boolean;
  fps_launch_option: string;
  fps_now: number | null;
}

interface OverlayData {
  cpu_percent: number;
  ram_percent: number;
  ram_used_gb: number;
  temp_c: number | null;
  gpu_percent: number | null;
  fan_rpm: number | null;
  net_down: number;
  net_up: number;
  battery_percent: number | null;
  charging: boolean;
  fps: number | null;
  frametime_ms: number | null;
  game: string | null;
}

const ITEM_LABEL: Record<string, string> = {
  fps: "Frame rate",
  cpu: "CPU",
  gpu: "GPU",
  ram: "Memory",
  temp: "Temperature",
  fan: "Fan",
  net: "Network",
  battery: "Battery",
  clock: "Clock",
};

function rate(bytes: number): string {
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)}M`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)}K`;
  return "0K";
}

/** One reading as (short label, value, how alarming it is). Null when there is nothing to show. */
function cell(item: string, d: OverlayData): [string, string, "" | "warn" | "bad"] | null {
  switch (item) {
    case "fps":
      return d.fps == null ? null : ["FPS", d.fps.toFixed(0), d.fps < 30 ? "bad" : d.fps < 50 ? "warn" : ""];
    case "cpu":
      return ["CPU", `${d.cpu_percent.toFixed(0)}%`, d.cpu_percent > 90 ? "warn" : ""];
    case "gpu":
      return d.gpu_percent == null ? null : ["GPU", `${d.gpu_percent.toFixed(0)}%`, ""];
    case "ram":
      return ["RAM", `${d.ram_percent.toFixed(0)}%`, d.ram_percent > 92 ? "bad" : d.ram_percent > 82 ? "warn" : ""];
    case "temp":
      return d.temp_c == null ? null : ["TEMP", `${d.temp_c.toFixed(0)}°`, d.temp_c >= 92 ? "bad" : d.temp_c >= 82 ? "warn" : ""];
    case "fan":
      return d.fan_rpm == null ? null : ["FAN", d.fan_rpm >= 1000 ? `${(d.fan_rpm / 1000).toFixed(1)}k` : `${d.fan_rpm}`, ""];
    case "net":
      return ["NET", `↓${rate(d.net_down)} ↑${rate(d.net_up)}`, ""];
    case "battery":
      return d.battery_percent == null ? null : ["BAT", `${d.battery_percent}%${d.charging ? "+" : ""}`, !d.charging && d.battery_percent <= 15 ? "bad" : ""];
    case "clock":
      return ["TIME", new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }), ""];
    default:
      return null;
  }
}

/** The overlay window itself: see-through, never clickable, readable over anything. */
export function Overlay() {
  const [view, setView] = useState<OverlayView | null>(null);
  const [data, setData] = useState<OverlayData | null>(null);
  useEffect(() => {
    document.documentElement.classList.add("overlay-root");
    let alive = true;
    const tick = async () => {
      const d = await invoke<OverlayData>("get_overlay_data").catch(() => null);
      if (alive && d) setData(d);
    };
    invoke<OverlayView>("get_overlay").then((v) => alive && setView(v));
    tick();
    const fast = setInterval(tick, 1000);
    // Settings can change while it is up; the layout comes from where it was placed.
    const slow = setInterval(() => invoke<OverlayView>("get_overlay").then((v) => alive && setView(v)), 5000);
    return () => {
      alive = false;
      clearInterval(fast);
      clearInterval(slow);
    };
  }, []);
  if (!view || !data) return null;
  const cells = view.config.items.map((item) => cell(item, data)).filter((c): c is [string, string, "" | "warn" | "bad"] => c !== null);
  const style = {
    "--overlay-backdrop": view.config.backdrop,
    "--overlay-scale": view.config.text_scale,
  } as React.CSSProperties;
  // The window can be larger than the numbers need (the toolkit has a smallest size),
  // so they are pinned to the chosen corner inside it; the rest stays invisible.
  const [vertical, horizontal] = (view.anchor || "top-right").split("-");
  return (
    <div className={`overlay-anchor overlay-${vertical} overlay-${horizontal}`}>
      <div className={`overlay overlay-${view.layout}`} style={{ ...style, width: view.width || undefined, height: view.height || undefined }}>
        {cells.map(([label, value, level]) => (
          <div key={label} className="overlay-cell">
            <span className="overlay-label">{label}</span>
            <span className={`overlay-value${level ? ` overlay-${level}` : ""}`}>{value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

const CORNERS: [string, string][] = [
  ["top-left", "↖"],
  ["top-center", "↑"],
  ["top-right", "↗"],
  ["bottom-left", "↙"],
  ["bottom-center", "↓"],
  ["bottom-right", "↘"],
];

/** Settings for the overlay, since the overlay itself ignores clicks. */
export function OverlaySettings() {
  const view = usePoll<OverlayView>("get_overlay", undefined, 3000);
  const [copied, setCopied] = useState(false);
  if (!view.data) return null;
  const v = view.data;
  const c = v.config;
  async function save(change: Partial<OverlayConfig>) {
    await invoke("save_overlay", { config: { ...c, ...change } });
    view.reload();
  }
  async function toggle() {
    await invoke("show_overlay", { show: !v.open });
    setTimeout(view.reload, 600);
  }
  return (
    <section className="panel wide">
      <h2>Stats Overlay</h2>
      <div className="control-row">
        <label>
          A see-through panel of live numbers on top of whatever you are doing
          <span className="mem-muted"> — clicks go straight through it; also in the tray menu, or run <code>trace overlay</code></span>
        </label>
        <button className={`ctl-btn${v.open ? "" : " ctl-primary"}`} onClick={toggle}>
          {v.open ? "Hide it" : "Show it"}
        </button>
      </div>
      {!v.desktop_support && (
        <p className="status-warn">
          ! Staying above other windows and letting clicks through are done by Hyprland. On this desktop the overlay
          opens as an ordinary small window.
        </p>
      )}
      <div className="control-row">
        <label>
          Shape
          <span className="mem-muted"> — automatic is a slim strip while a game runs and a small panel otherwise</span>
        </label>
        <Segmented options={["auto", "strip", "panel"]} value={c.layout} onChange={(layout) => save({ layout })} />
      </div>
      <div className="control-row">
        <label>Where</label>
        <div className="corner-grid">
          {CORNERS.map(([id, arrow]) => (
            <button key={id} className={`chip chip-button${c.corner === id ? " chip-on" : ""}`} title={id.replace("-", " ")} aria-label={id.replace("-", " ")} onClick={() => save({ corner: id })}>
              {arrow}
            </button>
          ))}
        </div>
        <select className="port-input" value={c.monitor} aria-label="Monitor" onChange={(e) => save({ monitor: e.target.value })}>
          <option value="auto">on the screen in use</option>
          {v.monitors.map((m) => (
            <option key={m} value={m}>
              always on {m}
            </option>
          ))}
        </select>
      </div>
      <div className="mixer-row">
        <span className="mem-muted">Tint behind the numbers</span>
        <input type="range" min={0} max={0.9} step={0.05} value={c.backdrop} aria-label="Tint behind the numbers" onChange={(e) => save({ backdrop: parseFloat(e.target.value) })} />
        <span className="bar-value">{Math.round(c.backdrop * 100)}%</span>
        <span className="mem-muted">Text size</span>
        <input type="range" min={0.8} max={1.8} step={0.1} value={c.text_scale} aria-label="Text size" onChange={(e) => save({ text_scale: parseFloat(e.target.value) })} />
        <span className="bar-value">{Math.round(c.text_scale * 100)}%</span>
      </div>
      <div className="chip-row">
        <span className="mem-muted">Show</span>
        {Object.keys(ITEM_LABEL).map((item) => {
          const on = c.items.includes(item);
          return (
            <button
              key={item}
              className={`chip chip-button${on ? " chip-on" : ""}`}
              onClick={() => save({ items: on ? c.items.filter((i) => i !== item) : Object.keys(ITEM_LABEL).filter((i) => i === item || c.items.includes(i)) })}
            >
              {ITEM_LABEL[item]}
            </button>
          );
        })}
      </div>
      <div className="control-row">
        <label>
          Start with Trace
          <span className="mem-muted"> — while it is showing, Trace keeps its window engine loaded (about 130 MB)</span>
        </label>
        <Segmented options={["on", "off"]} value={c.open_at_start ? "on" : "off"} onChange={(x) => save({ open_at_start: x === "on" })} />
      </div>

      <h3 className="sub-heading">Frame rate in games</h3>
      {v.mangohud ? (
        <>
          <p className="diagnose-hint">
            {v.fps_now != null ? (
              <span className="status-ok">✓ Receiving a frame rate right now: {v.fps_now.toFixed(0)} FPS. </span>
            ) : (
              "No game is reporting a frame rate at the moment. "
            )}
            Only the game itself knows its frame rate, so it has to report it. In Steam, open the game's Properties and
            paste this into Launch Options; other launchers have an equivalent field. MangoHud then runs inside the
            game, draws nothing, and Trace shows the number.
          </p>
          <div className="control-row">
            <code className="launch-option">{v.fps_launch_option}</code>
            <button
              className="ctl-btn"
              onClick={async () => {
                await navigator.clipboard.writeText(v.fps_launch_option).catch(() => {});
                setCopied(true);
                setTimeout(() => setCopied(false), 2000);
              }}
            >
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
        </>
      ) : (
        <p className="diagnose-hint">
          Only a game itself knows its frame rate. Install MangoHud and a launch option appears here that makes games
          report it to Trace.
        </p>
      )}
    </section>
  );
}
