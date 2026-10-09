import { useState } from "react";
import DiskExplorer from "./DiskExplorer";
import { TimeChart, bytesToHuman, usePoll } from "./ui";

interface HistoryPoint {
  ts: number;
  cpu_percent: number;
  cpu_peak: number;
  ram_percent: number;
  ram_bytes: number;
  swap_bytes: number;
  gpu_percent: number | null;
}

interface PressurePoint {
  ts: number;
  cpu_some: number;
  io_some: number;
  mem_some: number;
}

interface DiskPoint {
  ts: number;
  mount_point: string;
  used_bytes: number;
  total_bytes: number;
}

interface EventRow {
  ts: number;
  event_type: string;
  severity: string;
  message: string;
}

interface HistoryData {
  resources: HistoryPoint[];
  pressure: PressurePoint[];
  disks: DiskPoint[];
  events: EventRow[];
}

const RANGES = [
  { label: "1h", secs: 3600 },
  { label: "6h", secs: 6 * 3600 },
  { label: "24h", secs: 24 * 3600 },
  { label: "7d", secs: 7 * 24 * 3600 },
  { label: "30d", secs: 30 * 24 * 3600 },
];

const DISK_COLORS = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300"];

const GB = 1024 * 1024 * 1024;

function StorageBreakdown({ disks }: { disks: DiskPoint[] }) {
  const latestByMount = new Map<string, DiskPoint>();
  for (const d of disks) latestByMount.set(d.mount_point, d);
  const entries = [...latestByMount.values()].filter((d) => d.total_bytes > 0);
  if (entries.length === 0) return null;

  const totalUsed = entries.reduce((sum, d) => sum + d.used_bytes, 0);
  const radius = 40;
  const circumference = 2 * Math.PI * radius;
  let cumulative = 0;
  const slices = entries.map((d, i) => {
    const share = totalUsed > 0 ? d.used_bytes / totalUsed : 0;
    const slice = {
      disk: d,
      color: DISK_COLORS[i % DISK_COLORS.length],
      dashArray: `${circumference * share} ${circumference * (1 - share)}`,
      dashOffset: -circumference * cumulative,
    };
    cumulative += share;
    return slice;
  });

  return (
    <section className="panel">
      <h2>Storage Distribution</h2>
      <div className="pie-layout">
        <svg viewBox="0 0 100 100" className="pie-svg">
          {slices.map((s) => (
            <circle
              key={s.disk.mount_point}
              cx="50"
              cy="50"
              r={radius}
              fill="none"
              stroke={s.color}
              strokeWidth="16"
              strokeDasharray={s.dashArray}
              strokeDashoffset={s.dashOffset}
              transform="rotate(-90 50 50)"
            />
          ))}
          <text x="50" y="47" textAnchor="middle" className="pie-center-label">
            {bytesToHuman(totalUsed)}
          </text>
          <text x="50" y="58" textAnchor="middle" className="pie-center-sublabel">
            used total
          </text>
        </svg>
        <div className="pie-legend">
          {slices.map((s) => (
            <div key={s.disk.mount_point} className="pie-legend-row">
              <span className="pie-swatch" style={{ background: s.color }} />
              <span className="pie-legend-mount">{s.disk.mount_point}</span>
              <span className="pie-legend-value">
                {bytesToHuman(s.disk.used_bytes)} / {bytesToHuman(s.disk.total_bytes)} (
                {((s.disk.used_bytes / s.disk.total_bytes) * 100).toFixed(0)}%)
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

export default function History() {
  const [rangeSecs, setRangeSecs] = useState(RANGES[1].secs);
  const { data } = usePoll<HistoryData>("get_history", { sinceSecsAgo: rangeSecs }, 15000);

  const formatTime = (ts: number) => {
    const d = new Date(ts * 1000);
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return rangeSecs > 24 * 3600
      ? `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`
      : time;
  };
  const percent = (v: number) => `${v.toFixed(0)}%`;
  const stall = (v: number) => `${v.toFixed(1)}%`;
  const gigabytes = (v: number) => `${v.toFixed(1)} GB`;

  const picker = (
    <div className="range-picker">
      {RANGES.map((r) => (
        <button
          key={r.label}
          className={r.secs === rangeSecs ? "range-btn active" : "range-btn"}
          onClick={() => setRangeSecs(r.secs)}
        >
          {r.label}
        </button>
      ))}
    </div>
  );

  if (!data) {
    return (
      <>
        {picker}
        <p className="loading">Loading history…</p>
      </>
    );
  }

  const { resources, pressure, events } = data;
  const markers = events.filter((e) => e.severity !== "info").map((e) => e.ts);
  const diskByMount = new Map<string, DiskPoint[]>();
  for (const d of data.disks) {
    const arr = diskByMount.get(d.mount_point) ?? [];
    arr.push(d);
    diskByMount.set(d.mount_point, arr);
  }
  const swapMax = Math.max(1, ...resources.map((p) => p.swap_bytes / GB));
  const chart = { formatTime, markers };

  return (
    <>
      {picker}

      <section className="panel">
        <h2>CPU</h2>
        <TimeChart
          points={resources.map((p) => ({ ts: p.ts, v: p.cpu_percent }))}
          max={100}
          color="#3987e5"
          format={percent}
          {...chart}
        />
      </section>

      <section className="panel">
        <h2>RAM</h2>
        <TimeChart
          points={resources.map((p) => ({ ts: p.ts, v: p.ram_percent }))}
          max={100}
          color="#199e70"
          format={percent}
          {...chart}
        />
      </section>

      <section className="panel">
        <h2>Swap</h2>
        <TimeChart
          points={resources.map((p) => ({ ts: p.ts, v: p.swap_bytes / GB }))}
          max={swapMax}
          color="#d95926"
          format={gigabytes}
          {...chart}
        />
      </section>

      {resources.some((p) => p.gpu_percent !== null) && (
        <section className="panel">
          <h2>GPU</h2>
          <TimeChart
            points={resources.map((p) => ({ ts: p.ts, v: p.gpu_percent ?? 0 }))}
            max={100}
            color="#c98500"
            format={percent}
            {...chart}
          />
        </section>
      )}

      <section className="panel">
        <h2>Stalled on Memory</h2>
        <TimeChart
          points={pressure.map((p) => ({ ts: p.ts, v: p.mem_some }))}
          max={Math.max(5, ...pressure.map((p) => p.mem_some))}
          color="#d55181"
          format={stall}
          {...chart}
        />
        <p className="diagnose-hint">
          Share of time some task was waiting for memory. Unlike RAM usage,
          anything above zero here is slowdown you could feel.
        </p>
      </section>

      <section className="panel">
        <h2>Stalled on Disk</h2>
        <TimeChart
          points={pressure.map((p) => ({ ts: p.ts, v: p.io_some }))}
          max={Math.max(5, ...pressure.map((p) => p.io_some))}
          color="#9085e9"
          format={stall}
          {...chart}
        />
        <p className="diagnose-hint">
          Share of time some task was waiting on reads or writes.
        </p>
      </section>

      {[...diskByMount.entries()].map(([mount, points], i) => (
        <section key={mount} className="panel">
          <h2>Disk — {mount}</h2>
          <TimeChart
            points={points.map((p) => ({ ts: p.ts, v: p.used_bytes / GB }))}
            max={points[points.length - 1].total_bytes / GB}
            color={DISK_COLORS[i % DISK_COLORS.length]}
            format={gigabytes}
            formatTime={formatTime}
          />
        </section>
      ))}

      <StorageBreakdown disks={data.disks} />

      <section className="panel">
        <h2>What Happened ({events.length})</h2>
        {events.length === 0 ? (
          <p className="status-ok">✓ No spikes, stalls or swap activity in this range.</p>
        ) : (
          <div className="event-scroll">
            {[...events].reverse().map((e, i) => (
              <div key={i} className="event-row">
                <span className="event-time">{formatTime(e.ts)}</span>
                <span className="event-message">{e.message}</span>
              </div>
            ))}
          </div>
        )}
        <p className="diagnose-hint">
          Each warning is also marked as a tick along the bottom of the charts
          above, so you can see what else was happening at that moment.
        </p>
      </section>

      <DiskExplorer />
    </>
  );
}
