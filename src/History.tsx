import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ResourcePoint {
  ts: number;
  cpu_percent: number;
  used_memory_bytes: number;
  total_memory_bytes: number;
  used_swap_bytes: number;
  gpu_usage_percent: number | null;
}

interface DiskPoint {
  ts: number;
  mount_point: string;
  used_bytes: number;
  total_bytes: number;
}

const RANGES = [
  { label: "1h", secs: 3600 },
  { label: "6h", secs: 6 * 3600 },
  { label: "24h", secs: 24 * 3600 },
  { label: "7d", secs: 7 * 24 * 3600 },
];

function formatTime(ts: number, rangeSecs: number): string {
  const d = new Date(ts * 1000);
  if (rangeSecs > 24 * 3600) {
    return d.toLocaleDateString([], { month: "short", day: "numeric" }) + " " +
      d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function LineChart({
  points,
  max,
  color,
  unit = "%",
  rangeSecs,
  height = 170,
}: {
  points: { ts: number; v: number }[];
  max: number;
  color: string;
  unit?: string;
  rangeSecs: number;
  height?: number;
}) {
  if (points.length < 2) {
    return (
      <div className="chart-empty" style={{ height }}>
        Not enough data yet — history builds up every 10s
      </div>
    );
  }
  const width = 100;
  const minTs = points[0].ts;
  const maxTs = points[points.length - 1].ts;
  const spanTs = Math.max(maxTs - minTs, 1);
  const values = points.map((p) => p.v);
  const dataMax = Math.max(...values);
  const dataMin = Math.min(...values);
  const current = values[values.length - 1];
  const avg = values.reduce((a, b) => a + b, 0) / values.length;

  const path = points
    .map((p, i) => {
      const x = ((p.ts - minTs) / spanTs) * width;
      const y = height - (Math.min(p.v, max) / max) * height;
      return `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
  const areaPath = `${path} L${width},${height} L0,${height} Z`;
  const fmt = (v: number) => (unit === "%" ? v.toFixed(0) : v.toFixed(1));

  return (
    <div>
      <div className="chart-stats">
        <span className="chart-stat">
          <strong style={{ color }}>{fmt(current)}{unit}</strong> now
        </span>
        <span className="chart-stat">avg {fmt(avg)}{unit}</span>
        <span className="chart-stat">min {fmt(dataMin)}{unit}</span>
        <span className="chart-stat">max {fmt(dataMax)}{unit}</span>
      </div>
      <div className="chart-with-axis">
        <div className="chart-y-axis">
          <span>{fmt(max)}{unit}</span>
          <span>{fmt(max / 2)}{unit}</span>
          <span>0{unit}</span>
        </div>
        <svg
          viewBox={`0 0 ${width} ${height}`}
          preserveAspectRatio="none"
          className="chart-svg"
          style={{ height }}
        >
          <line x1="0" y1={height / 2} x2={width} y2={height / 2} className="chart-grid" />
          <line x1="0" y1="0" x2={width} y2="0" className="chart-grid" />
          <path d={areaPath} fill={color} opacity={0.15} />
          <path d={path} fill="none" stroke={color} strokeWidth={0.8} />
        </svg>
      </div>
      <div className="chart-x-axis">
        <span>{formatTime(minTs, rangeSecs)}</span>
        <span>{formatTime(maxTs, rangeSecs)}</span>
      </div>
    </div>
  );
}

function bytesToGB(bytes: number): string {
  return (bytes / 1024 / 1024 / 1024).toFixed(1);
}

const DISK_COLORS = ["#c26be0", "#4a7fe0", "#3fb968", "#e0a83e", "#e0503e", "#5be0c2"];

function StorageBreakdown({ disks }: { disks: DiskPoint[] }) {
  const latestByMount = new Map<string, DiskPoint>();
  for (const d of disks) {
    latestByMount.set(d.mount_point, d);
  }
  const entries = [...latestByMount.values()].filter((d) => d.total_bytes > 0);
  if (entries.length === 0) return null;

  const totalUsed = entries.reduce((sum, d) => sum + d.used_bytes, 0);
  let cumulativePercent = 0;
  const radius = 40;
  const circumference = 2 * Math.PI * radius;

  const slices = entries.map((d, i) => {
    const percent = totalUsed > 0 ? (d.used_bytes / totalUsed) * 100 : 0;
    const offset = circumference * (1 - cumulativePercent / 100);
    const dash = circumference * (percent / 100);
    cumulativePercent += percent;
    return {
      mount: d.mount_point,
      used: d.used_bytes,
      total: d.total_bytes,
      percent,
      color: DISK_COLORS[i % DISK_COLORS.length],
      dashArray: `${dash} ${circumference - dash}`,
      dashOffset: offset,
    };
  });

  return (
    <section className="panel">
      <h2>Storage Distribution</h2>
      <div className="pie-layout">
        <svg viewBox="0 0 100 100" className="pie-svg">
          {slices.map((s, i) => (
            <circle
              key={i}
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
            {bytesToGB(totalUsed)} GB
          </text>
          <text x="50" y="58" textAnchor="middle" className="pie-center-sublabel">
            used total
          </text>
        </svg>
        <div className="pie-legend">
          {slices.map((s, i) => (
            <div key={i} className="pie-legend-row">
              <span className="pie-swatch" style={{ background: s.color }} />
              <span className="pie-legend-mount">{s.mount}</span>
              <span className="pie-legend-value">
                {bytesToGB(s.used)} / {bytesToGB(s.total)} GB ({s.percent.toFixed(0)}%)
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
  const [resources, setResources] = useState<ResourcePoint[]>([]);
  const [disks, setDisks] = useState<DiskPoint[]>([]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const [r, d] = await Promise.all([
          invoke<ResourcePoint[]>("get_resource_history", {
            sinceSecsAgo: rangeSecs,
          }),
          invoke<DiskPoint[]>("get_disk_history", { sinceSecsAgo: rangeSecs }),
        ]);
        if (!cancelled) {
          setResources(r);
          setDisks(d);
        }
      } catch (e) {
        console.error(e);
      }
    }
    load();
    const id = setInterval(load, 15000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [rangeSecs]);

  const diskByMount = new Map<string, DiskPoint[]>();
  for (const d of disks) {
    const arr = diskByMount.get(d.mount_point) ?? [];
    arr.push(d);
    diskByMount.set(d.mount_point, arr);
  }

  return (
    <div>
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

      <section className="panel">
        <h2>CPU %</h2>
        <LineChart
          points={resources.map((p) => ({ ts: p.ts, v: p.cpu_percent }))}
          max={100}
          color="#4a7fe0"
          rangeSecs={rangeSecs}
        />
      </section>

      <section className="panel">
        <h2>RAM Usage</h2>
        <LineChart
          points={resources.map((p) => ({
            ts: p.ts,
            v: (p.used_memory_bytes / p.total_memory_bytes) * 100,
          }))}
          max={100}
          color="#3fb968"
          rangeSecs={rangeSecs}
        />
      </section>

      {resources.some((p) => p.gpu_usage_percent !== null) && (
        <section className="panel">
          <h2>GPU Utilization %</h2>
          <LineChart
            points={resources.map((p) => ({
              ts: p.ts,
              v: p.gpu_usage_percent ?? 0,
            }))}
            max={100}
            color="#e0a83e"
            rangeSecs={rangeSecs}
          />
        </section>
      )}

      <StorageBreakdown disks={disks} />

      {[...diskByMount.entries()].map(([mount, pts]) => {
        const totalGB = pts.length
          ? bytesToGB(pts[pts.length - 1].total_bytes)
          : "0";
        return (
          <section className="panel" key={mount}>
            <h2>
              Disk — {mount} ({totalGB} GB total)
            </h2>
            <LineChart
              points={pts.map((p) => ({ ts: p.ts, v: p.used_bytes / 1e9 }))}
              max={Math.max(...pts.map((p) => p.total_bytes / 1e9), 1)}
              color="#c26be0"
              unit=" GB"
              rangeSecs={rangeSecs}
            />
          </section>
        );
      })}
    </div>
  );
}
