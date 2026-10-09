import { useState } from "react";
import { formatRate, usePoll } from "./ui";

interface AppActivity {
  name: string;
  process_count: number;
  read_bytes_per_sec: number;
  write_bytes_per_sec: number;
  net_rx_bytes_per_sec: number;
  net_tx_bytes_per_sec: number;
  wakeups_per_sec: number;
}

interface DriveActivity {
  device: string;
  read_bytes_per_sec: number;
  write_bytes_per_sec: number;
}

interface ActivityData {
  ready: boolean;
  apps: AppActivity[];
  drives: DriveActivity[];
}

type SortKey = "disk" | "network" | "wakeups";

const SORTS: { key: SortKey; label: string; value: (a: AppActivity) => number }[] = [
  { key: "disk", label: "Disk", value: (a) => a.read_bytes_per_sec + a.write_bytes_per_sec },
  { key: "network", label: "Network", value: (a) => a.net_rx_bytes_per_sec + a.net_tx_bytes_per_sec },
  { key: "wakeups", label: "Wakeups", value: (a) => a.wakeups_per_sec },
];

export default function Activity() {
  const { data } = usePoll<ActivityData>("get_activity", undefined, 3000);
  const [sortKey, setSortKey] = useState<SortKey>("disk");
  const sort = SORTS.find((s) => s.key === sortKey)!;

  if (!data || !data.ready) {
    return <p className="loading">Measuring activity…</p>;
  }

  const rows = data.apps
    .filter((a) => sort.value(a) > 0)
    .sort((a, b) => sort.value(b) - sort.value(a))
    .slice(0, 20);
  const max = Math.max(1, ...rows.map(sort.value));

  return (
    <>
      <section className="panel wide">
        <h2>Drives Right Now</h2>
        {data.drives.map((d) => (
          <div key={d.device} className="meta-row">
            <span>{d.device}</span>
            <span>
              read {formatRate(d.read_bytes_per_sec)} · write{" "}
              {formatRate(d.write_bytes_per_sec)}
            </span>
          </div>
        ))}
      </section>

      <section className="panel wide">
        <h2>Who Is Busy</h2>
        <div className="range-picker">
          {SORTS.map((s) => (
            <button
              key={s.key}
              className={s.key === sortKey ? "range-btn active" : "range-btn"}
              onClick={() => setSortKey(s.key)}
            >
              {s.label}
            </button>
          ))}
        </div>
        {rows.length === 0 ? (
          <p className="diagnose-hint">Nothing is using the {sort.label.toLowerCase()} right now.</p>
        ) : (
          <table className="proc-table activity-table">
            <thead>
              <tr>
                <th>App</th>
                <th></th>
                <th>Disk read</th>
                <th>Disk write</th>
                <th>Net ↓</th>
                <th>Net ↑</th>
                <th>Wakeups/s</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((a) => (
                <tr key={a.name}>
                  <td>
                    {a.name}
                    {a.process_count > 1 && (
                      <span className="mem-muted"> ({a.process_count})</span>
                    )}
                  </td>
                  <td className="activity-bar-cell">
                    <div className="folder-bar-track">
                      <div
                        className="mem-bar-seg"
                        style={{
                          width: `${(sort.value(a) / max) * 100}%`,
                          background: "#3987e5",
                        }}
                      />
                    </div>
                  </td>
                  <td>{formatRate(a.read_bytes_per_sec)}</td>
                  <td>{formatRate(a.write_bytes_per_sec)}</td>
                  <td>{formatRate(a.net_rx_bytes_per_sec)}</td>
                  <td>{formatRate(a.net_tx_bytes_per_sec)}</td>
                  <td>{a.wakeups_per_sec >= 1 ? a.wakeups_per_sec.toFixed(0) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="diagnose-hint">
          Rates over the last few seconds. Network covers TCP traffic of your
          own apps and is approximate when connections open and close quickly.
          Wakeups (how often an app's main thread wakes the CPU) are what
          drain a battery while the machine looks idle. Apps owned by other
          users only show wakeups.
        </p>
      </section>
    </>
  );
}
