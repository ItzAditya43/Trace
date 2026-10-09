import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import History from "./History";
import Diagnose from "./Diagnose";
import Projects from "./Projects";
import Control from "./Control";
import MemoryMap from "./MemoryMap";
import Activity from "./Activity";
import Health, { HealthScore } from "./Health";
import Logs from "./Logs";
import Tweaks from "./Tweaks";
import Hardware from "./Hardware";
import { ExplainNow, PinnedSwitches } from "./Quick";
import { listen } from "@tauri-apps/api/event";
import SearchPalette from "./SearchPalette";
import { NavContext, NavTarget } from "./nav";
import { Tile, isWindowVisible, onWindowShown } from "./ui";
import UpdateBanner from "./UpdateBanner";
import "./App.css";
import { CardChooser, CardPrefs, SetupCard, loadCardPrefs } from "./Batch6";

const CARD_LABELS: Record<string, string> = { health: "System Health", cores: "Cores", gpus: "GPUs", temps: "Temperatures & Power", network: "Network", disks: "Disks", processes: "Top Processes (by memory)" };

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
  per_core_freq_mhz: number[];
  total_memory_bytes: number;
  used_memory_bytes: number;
  available_memory_bytes: number;
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
  pressure: {
    cpu_some: number;
    io_some: number;
    io_full: number;
    mem_some: number;
    mem_full: number;
  };
}

const PAGES = [
  { key: "live", label: "Overview", hint: "Live state of the machine" },
  { key: "memory", label: "Memory", hint: "Where RAM and swap are going" },
  { key: "activity", label: "Activity", hint: "Disk, network and wakeups by app" },
  { key: "hardware", label: "Hardware", hint: "Sensors, fans, graphics, displays and devices" },
  { key: "health", label: "Health", hint: "Score, restarts, errors, battery, drives" },
  { key: "history", label: "History", hint: "Charts over time and disk usage" },
  { key: "logs", label: "Logs", hint: "Daily, weekly, monthly and yearly record" },
  { key: "diagnose", label: "Investigate", hint: "What is slowing me down?" },
  { key: "projects", label: "Projects", hint: "Git and dependency status" },
  { key: "tweaks", label: "Tweaks", hint: "Every system setting worth changing, with presets" },
  { key: "control", label: "Control", hint: "Act on the machine" },
] as const;

type PageKey = (typeof PAGES)[number]["key"];

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

function MiniBar({ label, percent }: { label: string; percent: number }) {
  const level = percent > 85 ? "danger" : percent > 60 ? "warn" : "ok";
  return (
    <div className="mini-row" title={`${label} ${percent.toFixed(0)}%`}>
      <span>{label}</span>
      <div className="mini-track">
        <div className={`bar-fill bar-${level}`} style={{ width: `${Math.min(percent, 100)}%` }} />
      </div>
      <span className="mini-value">{percent.toFixed(0)}%</span>
    </div>
  );
}

function App() {
  const [cardPrefs, setCardPrefs] = useState<CardPrefs>(() => loadCardPrefs(Object.keys(CARD_LABELS)));
  // Cards keep their place in the page's code; the chosen order is applied as a layout order.
  const cardStyle = (id: string) => ({ order: 10 + cardPrefs.order.indexOf(id), display: cardPrefs.hidden.includes(id) ? "none" : undefined });
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [target, setTarget] = useState<NavTarget>({ page: "live", nonce: 0 });
  const [searching, setSearching] = useState(false);
  const tab = (PAGES.some((p) => p.key === target.page) ? target.page : "live") as PageKey;
  const go = useCallback(
    (to: Omit<NavTarget, "nonce">) => setTarget((t) => ({ ...to, nonce: t.nonce + 1 })),
    []
  );
  const setTab = (page: PageKey) => go({ page });

  // `trace page <name>` and `trace panic` from a terminal or a key binding.
  useEffect(() => {
    const stop = listen<{ page: string; section?: string }>("navigate", (event) =>
      go({ page: event.payload.page, ...(event.payload.section ? { section: event.payload.section } : {}) })
    );
    return () => {
      stop.then((off) => off());
    };
  }, [go]);

  // Ctrl+K (or /) opens search from anywhere.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const typing = ["INPUT", "TEXTAREA", "SELECT"].includes((e.target as HTMLElement)?.tagName);
      if ((e.ctrlKey && e.key.toLowerCase() === "k") || (e.key === "/" && !typing)) {
        e.preventDefault();
        setSearching(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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
    const id = setInterval(() => {
      if (isWindowVisible()) poll();
    }, 2000);
    const stop = onWindowShown(poll);
    return () => {
      cancelled = true;
      clearInterval(id);
      stop();
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

  const page = PAGES.find((p) => p.key === tab)!;
  const hottest = snapshot.temperatures.reduce((max, t) => Math.max(max, t.celsius), 0);
  const netDown = snapshot.networks.reduce((sum, n) => sum + n.rx_bytes_per_sec, 0);
  const netUp = snapshot.networks.reduce((sum, n) => sum + n.tx_bytes_per_sec, 0);
  const fullest = snapshot.disks.reduce(
    (max, d) => Math.max(max, d.total_bytes ? (1 - d.available_bytes / d.total_bytes) * 100 : 0),
    0
  );
  const stalled = Math.max(
    snapshot.pressure.cpu_some,
    snapshot.pressure.mem_some,
    snapshot.pressure.io_some
  );
  const tone = (value: number, warn: number, danger: number) =>
    value > danger ? "danger" : value > warn ? "warn" : undefined;
  const memPercent =
    (snapshot.used_memory_bytes / snapshot.total_memory_bytes) * 100;
  const swapPercent = snapshot.total_swap_bytes
    ? (snapshot.used_swap_bytes / snapshot.total_swap_bytes) * 100
    : 0;

  return (
    <NavContext.Provider value={{ target, go }}>
    <main className="container shell">
      {searching && <SearchPalette onClose={() => setSearching(false)} />}
      <header className="header">
        <h1>Trace</h1>
        <span className="subtitle">
          Machine Observatory
          <br />
          up {formatUptime(snapshot.uptime_secs)}
        </span>
      </header>

      <nav className="side-nav">
        <button className="nav-search" onClick={() => setSearching(true)} title="Search everything (Ctrl+K)">
          <span>Search…</span>
          <kbd>Ctrl K</kbd>
        </button>
        {PAGES.map((p) => (
          <button
            key={p.key}
            className={tab === p.key ? "nav-btn active" : "nav-btn"}
            title={p.hint}
            onClick={() => setTab(p.key)}
          >
            {p.label}
          </button>
        ))}
        <div className="side-vitals">
          <MiniBar label="CPU" percent={snapshot.cpu_usage_percent} />
          <MiniBar label="RAM" percent={memPercent} />
          {snapshot.total_swap_bytes > 0 && <MiniBar label="Swap" percent={swapPercent} />}
          {hottest > 0 && (
            <div className="mini-row">
              <span>Temp</span>
              <span className="mini-value">{hottest.toFixed(0)}°C</span>
            </div>
          )}
          <div className="mini-row">
            <span>Net</span>
            <span className="mini-value">
              ↓{formatRate(netDown)} ↑{formatRate(netUp)}
            </span>
          </div>
        </div>
      </nav>

      <div className="page">
        <h2 className="page-title">
          {page.label} <span className="subtitle">{page.hint}</span>
        </h2>
      <div className="page-body">
      {tab === "activity" && <Activity />}
      {tab === "logs" && <Logs />}
      {tab === "tweaks" && <Tweaks />}
      {tab === "hardware" && <Hardware />}
      {tab === "health" && <Health />}
      {tab === "memory" && <MemoryMap />}
      {tab === "history" && <History />}
      {tab === "diagnose" && <Diagnose />}
      {tab === "projects" && <Projects />}
      {tab === "control" && <Control />}
      {tab === "live" && (
        <>
      <UpdateBanner />
      <SetupCard />
      <div className="tile-grid">
        <Tile
          label="CPU"
          value={`${snapshot.cpu_usage_percent.toFixed(0)}%`}
          detail={`load ${snapshot.load_average[0].toFixed(2)} on ${snapshot.per_core_usage.length} cores`}
          tone={tone(snapshot.cpu_usage_percent, 60, 85)}
        />
        <Tile
          label="RAM available"
          value={`${bytesToGB(snapshot.available_memory_bytes)} GB`}
          detail={`of ${bytesToGB(snapshot.total_memory_bytes)} GB`}
          tone={tone(memPercent, 75, 90)}
        />
        <Tile
          label="Swap used"
          value={`${bytesToGB(snapshot.used_swap_bytes)} GB`}
          detail={`${swapPercent.toFixed(0)}% of swap`}
          tone={tone(swapPercent, 50, 80)}
        />
        <Tile
          label="Stalled"
          value={`${stalled.toFixed(0)}%`}
          detail="worst of CPU, memory, disk"
          tone={tone(stalled, 5, 20)}
        />
        <Tile
          label="Hottest"
          value={hottest > 0 ? `${hottest.toFixed(0)}°C` : "—"}
          detail="highest sensor"
          tone={tone(hottest, 80, 92)}
        />
        <Tile
          label="Fullest disk"
          value={`${fullest.toFixed(0)}%`}
          detail="of its capacity"
          tone={tone(fullest, 82, 92)}
        />
        <Tile
          label="Network"
          value={`↓ ${formatRate(netDown)}`}
          detail={`↑ ${formatRate(netUp)}`}
        />
        {snapshot.battery && (
          <Tile
            label="Battery"
            value={`${snapshot.battery.percent}%`}
            detail={snapshot.battery.status.toLowerCase()}
            tone={snapshot.battery.percent < 15 ? "danger" : undefined}
          />
        )}
      </div>
      <PinnedSwitches />
      <HealthScore compact />

      <section className="panel" style={cardStyle("health")}>
        <h2>System Health</h2>
        <Bar percent={snapshot.cpu_usage_percent} label="CPU" />
        <Bar percent={memPercent} label="RAM" />
        {snapshot.total_swap_bytes > 0 && (
          <Bar percent={swapPercent} label="Swap" />
        )}
        <div className="meta-row">
          <span>
            RAM: {bytesToGB(snapshot.used_memory_bytes)} /{" "}
            {bytesToGB(snapshot.total_memory_bytes)} GB ·{" "}
            {bytesToGB(snapshot.available_memory_bytes)} GB available
          </span>
          <span>
            Load avg: {snapshot.load_average[0].toFixed(2)}{" "}
            {snapshot.load_average[1].toFixed(2)}{" "}
            {snapshot.load_average[2].toFixed(2)}
          </span>
        </div>
        <div className="meta-row">
          <span title="Share of the last 10 seconds in which work was held up waiting for each resource">
            Stalled waiting for — CPU {snapshot.pressure.cpu_some.toFixed(0)}% ·
            memory {snapshot.pressure.mem_some.toFixed(0)}% · disk{" "}
            {snapshot.pressure.io_some.toFixed(0)}%
          </span>
        </div>
        <ExplainNow />
      </section>

      <section className="panel" style={cardStyle("cores")}>
        <h2>Cores</h2>
        <div className="core-grid">
          {snapshot.per_core_usage.map((usage, i) => {
            const level = usage > 85 ? "danger" : usage > 60 ? "warn" : "ok";
            const freq = snapshot.per_core_freq_mhz[i];
            return (
              <div key={i} className={`core-cell core-cell-${level}`} title={`Core ${i}: ${usage.toFixed(0)}%${freq ? ` @ ${(freq / 1000).toFixed(2)} GHz` : ""}`}>
                <div
                  className="core-fill"
                  style={{ height: `${Math.min(usage, 100)}%` }}
                />
                <span className="core-percent">{usage.toFixed(0)}</span>
                <span className="core-label">{i}</span>
                {freq > 0 && (
                  <span className="core-freq">{(freq / 1000).toFixed(1)}GHz</span>
                )}
              </div>
            );
          })}
        </div>
        <p className="diagnose-hint">
          Hover a core for exact load and clock speed.
        </p>
      </section>

      {snapshot.gpus.length > 0 && (
        <section className="panel" style={cardStyle("gpus")}>
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
        <section className="panel" style={cardStyle("temps")}>
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
        <section className="panel" style={cardStyle("network")}>
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

      <section className="panel" style={cardStyle("disks")}>
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

      <section className="panel wide" style={cardStyle("processes")}>
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
      <CardChooser prefs={cardPrefs} labels={CARD_LABELS} onChange={setCardPrefs} />
        </>
      )}
      </div>
      </div>
    </main>
    </NavContext.Provider>
  );
}

export default App;
