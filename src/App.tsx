import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import History from "./History";
import Diagnose from "./Diagnose";
import Projects from "./Projects";
import Control from "./Control";
import UpdateBanner from "./UpdateBanner";
import "./App.css";

interface ProcessInfo {
  pid: number;
  name: string;
  cpu_usage: number;
  memory_bytes: number;
}

interface DiskInfo {
  name: string;
  mount_point: string;
  total_bytes: number;
  available_bytes: number;
}

interface GpuInfo {
  name: string;
  vendor: string;
  usage_percent: number;
  vram_used_bytes: number;
  vram_total_bytes: number;
  temperature_c: number | null;
  power_draw_watts: number | null;
}

interface TempSensor {
  label: string;
  celsius: number;
}

interface NetworkInfo {
  interface: string;
  rx_bytes_per_sec: number;
  tx_bytes_per_sec: number;
}

interface BatteryInfo {
  percent: number;
  status: string;
}

interface Snapshot {
  cpu_usage_percent: number;
  per_core_usage: number[];
  total_memory_bytes: number;
  used_memory_bytes: number;
  total_swap_bytes: number;
  used_swap_bytes: number;
  load_average: [number, number, number];
  disks: DiskInfo[];
  top_processes: ProcessInfo[];
  uptime_secs: number;
  gpus: GpuInfo[];
  temperatures: TempSensor[];
  networks: NetworkInfo[];
  battery: BatteryInfo | null;
}

function bytesToGB(bytes: number): string {
  return (bytes / 1024 / 1024 / 1024).toFixed(1);
}

function formatUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

function formatRate(bytesPerSec: number): string {
  if (bytesPerSec > 1024 * 1024) {
    return `${(bytesPerSec / 1024 / 1024).toFixed(1)} MB/s`;
  }
  return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
}

function Bar({ percent, label }: { percent: number; label: string }) {
  const level = percent > 85 ? "danger" : percent > 60 ? "warn" : "ok";
  return (
    <div className="bar-row">
      <span className="bar-label">{label}</span>
      <div className="bar-track">
        <div
          className={`bar-fill bar-${level}`}
          style={{ width: `${Math.min(percent, 100)}%` }}
        />
      </div>
      <span className="bar-value">{percent.toFixed(0)}%</span>
    </div>
  );
}

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [tab, setTab] = useState<
    "live" | "history" | "diagnose" | "projects" | "control"
  >("live");

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const s = await invoke<Snapshot>("get_snapshot");
        if (!cancelled) setSnapshot(s);
      } catch (e) {
        console.error(e);
      }
    }
    poll();
    const id = setInterval(poll, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  if (!snapshot) {
    return (
      <main className="container">
        <UpdateBanner />
        <header className="header">
          <h1>Trace</h1>
          <span className="subtitle">Machine Observatory</span>
        </header>
        <p className="loading">Reading machine state…</p>
      </main>
    );
  }

  const memPercent =
    (snapshot.used_memory_bytes / snapshot.total_memory_bytes) * 100;
  const swapPercent = snapshot.total_swap_bytes
    ? (snapshot.used_swap_bytes / snapshot.total_swap_bytes) * 100
    : 0;

  return (
    <main className="container">
      <UpdateBanner />
      <header className="header">
        <h1>Trace</h1>
        <span className="subtitle">
          Machine Observatory · uptime {formatUptime(snapshot.uptime_secs)}
        </span>
      </header>

      <nav className="tab-bar">
        <button
          className={tab === "live" ? "tab-btn active" : "tab-btn"}
          onClick={() => setTab("live")}
        >
          Live
        </button>
        <button
          className={tab === "history" ? "tab-btn active" : "tab-btn"}
          onClick={() => setTab("history")}
        >
          History
        </button>
        <button
          className={tab === "diagnose" ? "tab-btn active" : "tab-btn"}
          onClick={() => setTab("diagnose")}
        >
          Investigate
        </button>
        <button
          className={tab === "projects" ? "tab-btn active" : "tab-btn"}
          onClick={() => setTab("projects")}
        >
          Projects
        </button>
        <button
          className={tab === "control" ? "tab-btn active" : "tab-btn"}
          onClick={() => setTab("control")}
        >
          Control
        </button>
      </nav>

      {tab === "history" && <History />}
      {tab === "diagnose" && <Diagnose />}
      {tab === "projects" && <Projects />}
      {tab === "control" && <Control />}
      {tab === "live" && (
        <>

      <section className="panel">
        <h2>System Health</h2>
        <Bar percent={snapshot.cpu_usage_percent} label="CPU" />
        <Bar percent={memPercent} label="RAM" />
        {snapshot.total_swap_bytes > 0 && (
          <Bar percent={swapPercent} label="Swap" />
        )}
        <div className="meta-row">
          <span>
            RAM: {bytesToGB(snapshot.used_memory_bytes)} /{" "}
            {bytesToGB(snapshot.total_memory_bytes)} GB
          </span>
          <span>
            Load avg: {snapshot.load_average[0].toFixed(2)}{" "}
            {snapshot.load_average[1].toFixed(2)}{" "}
            {snapshot.load_average[2].toFixed(2)}
          </span>
        </div>
      </section>

      <section className="panel">
        <h2>Cores</h2>
        <div className="core-grid">
          {snapshot.per_core_usage.map((usage, i) => (
            <div key={i} className="core-cell">
              <div
                className="core-fill"
                style={{ height: `${Math.min(usage, 100)}%` }}
              />
              <span className="core-label">{i}</span>
            </div>
          ))}
        </div>
      </section>

      {snapshot.gpus.length > 0 && (
        <section className="panel">
          <h2>GPUs</h2>
          {snapshot.gpus.map((g, i) => {
            const vramPct = g.vram_total_bytes
              ? (g.vram_used_bytes / g.vram_total_bytes) * 100
              : 0;
            return (
              <div key={i} className="gpu-block">
                <div className="disk-info">
                  <span className="disk-name">
                    {g.vendor} — {g.name}
                  </span>
                  <span className="disk-size">
                    {g.temperature_c !== null && `${g.temperature_c.toFixed(0)}°C `}
                    {g.power_draw_watts !== null &&
                      `· ${g.power_draw_watts.toFixed(1)} W`}
                  </span>
                </div>
                <Bar percent={g.usage_percent} label="Util" />
                {g.vram_total_bytes > 0 && (
                  <>
                    <Bar percent={vramPct} label="VRAM" />
                    <div className="meta-row">
                      <span>
                        {bytesToGB(g.vram_used_bytes)} /{" "}
                        {bytesToGB(g.vram_total_bytes)} GB
                      </span>
                    </div>
                  </>
                )}
              </div>
            );
          })}
        </section>
      )}

      {(snapshot.temperatures.length > 0 || snapshot.battery) && (
        <section className="panel">
          <h2>Temperatures &amp; Power</h2>
          {snapshot.battery && (
            <div className="meta-row">
              <span>
                Battery: {snapshot.battery.percent}% ({snapshot.battery.status})
              </span>
            </div>
          )}
          <div className="temp-grid">
            {snapshot.temperatures
              .sort((a, b) => b.celsius - a.celsius)
              .slice(0, 12)
              .map((t, i) => (
                <div key={i} className="temp-cell">
                  <span className="temp-label">{t.label}</span>
                  <span
                    className={
                      "temp-value " +
                      (t.celsius > 85
                        ? "temp-danger"
                        : t.celsius > 65
                        ? "temp-warn"
                        : "temp-ok")
                    }
                  >
                    {t.celsius.toFixed(0)}°C
                  </span>
                </div>
              ))}
          </div>
        </section>
      )}

      {snapshot.networks.length > 0 && (
        <section className="panel">
          <h2>Network</h2>
          {snapshot.networks.map((n) => (
            <div key={n.interface} className="meta-row">
              <span>{n.interface}</span>
              <span>
                ↓ {formatRate(n.rx_bytes_per_sec)} · ↑{" "}
                {formatRate(n.tx_bytes_per_sec)}
              </span>
            </div>
          ))}
        </section>
      )}

      <section className="panel">
        <h2>Disks</h2>
        {snapshot.disks.map((d) => {
          const usedPct =
            ((d.total_bytes - d.available_bytes) / d.total_bytes) * 100;
          return (
            <div key={d.mount_point} className="disk-row">
              <div className="disk-info">
                <span className="disk-name">
                  {d.name || d.mount_point} — {d.mount_point}
                </span>
                <span className="disk-size">
                  {bytesToGB(d.total_bytes - d.available_bytes)} /{" "}
                  {bytesToGB(d.total_bytes)} GB
                </span>
              </div>
              <Bar percent={usedPct} label="" />
            </div>
          );
        })}
      </section>

      <section className="panel">
        <h2>Top Processes (by memory)</h2>
        <table className="proc-table">
          <thead>
            <tr>
              <th>PID</th>
              <th>Name</th>
              <th>CPU%</th>
              <th>Memory</th>
            </tr>
          </thead>
          <tbody>
            {snapshot.top_processes.map((p) => (
              <tr key={p.pid}>
                <td>{p.pid}</td>
                <td>{p.name}</td>
                <td>{p.cpu_usage.toFixed(1)}</td>
                <td>{bytesToGB(p.memory_bytes)} GB</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
        </>
      )}
    </main>
  );
}

export default App;
