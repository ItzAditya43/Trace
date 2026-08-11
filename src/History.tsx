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

function LineChart({
  points,
  max,
  color,
  height = 90,
}: {
  points: { ts: number; v: number }[];
  max: number;
  color: string;
  height?: number;
}) {
  if (points.length < 2) {
    return (
      <div className="chart-empty" style={{ height }}>
        Not enough data yet — history builds up every 30s
      </div>
    );
  }
  const width = 100;
  const minTs = points[0].ts;
  const maxTs = points[points.length - 1].ts;
  const spanTs = Math.max(maxTs - minTs, 1);
  const path = points
    .map((p, i) => {
      const x = ((p.ts - minTs) / spanTs) * width;
      const y = height - (Math.min(p.v, max) / max) * height;
      return `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
  const areaPath = `${path} L${width},${height} L0,${height} Z`;

  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      className="chart-svg"
      style={{ height }}
    >
      <path d={areaPath} fill={color} opacity={0.15} />
      <path d={path} fill="none" stroke={color} strokeWidth={0.8} />
    </svg>
  );
}

function bytesToGB(bytes: number): string {
  return (bytes / 1024 / 1024 / 1024).toFixed(1);
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
          />
        </section>
      )}

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
            />
          </section>
        );
      })}
    </div>
  );
}
