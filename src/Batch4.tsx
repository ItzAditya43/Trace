import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ConfirmButton, StatusLine, bytesToHuman, formatClock, formatDay, useAction, usePoll } from "./ui";

interface PowerReport {
  on_battery: boolean;
  total_watts: number;
  idle_watts: number;
  measured: boolean;
  apps: { app: string; cpu_percent: number; watts: number; minutes_gained: number | null }[];
}

/** Which apps are costing battery, measured over three seconds. */
export function PowerPerApp() {
  const [report, setReport] = useState<PowerReport | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    try {
      setReport(await invoke("get_power_report"));
    } finally {
      setBusy(false);
    }
  }
  const top = report ? Math.max(...report.apps.map((a) => a.watts), 0.1) : 1;
  return (
    <section className="panel">
      <h2>Battery Drain Per App</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? "Watching for 3 seconds…" : report ? "Measure again" : "What is using the battery?"}
        </button>
      </div>
      {report && (
        <>
          {report.on_battery ? (
            <div className="meta-row">
              <span>Drawing now</span>
              <span>
                {report.total_watts.toFixed(1)} W, of which about {report.idle_watts.toFixed(1)} W is the screen and the
                machine just being on
              </span>
            </div>
          ) : (
            <p className="diagnose-hint">Plugged in, so these are what each app would cost on battery.</p>
          )}
          {report.apps.length === 0 && <p className="diagnose-hint">Nothing is using the processor right now.</p>}
          {report.apps.map((a) => (
            <div key={a.app} className="bar-row">
              <span className="bar-label" title={a.app}>
                {a.app}
              </span>
              <div className="bar-track">
                <div className="bar-fill" style={{ width: `${(a.watts / top) * 100}%` }} />
              </div>
              <span className="bar-value">
                {a.watts.toFixed(1)} W{a.minutes_gained != null && a.minutes_gained >= 1 ? ` · +${a.minutes_gained.toFixed(0)} min` : ""}
              </span>
            </div>
          ))}
          <p className="diagnose-hint">
            {report.measured
              ? "The split uses this machine's own history of power draw against processor use."
              : "An estimate from a typical laptop until Trace has seen enough time on battery to learn this machine (about five minutes of varied use)."}{" "}
            It counts processor work only; the screen, the graphics card and radios are in the machine's share. “+min” is
            the battery time closing that app would buy.
          </p>
        </>
      )}
    </section>
  );
}

interface GrowthReport {
  running: boolean;
  newest: number;
  compared_to: number;
  snapshots: number[];
  folders: { path: string; before: number; now: number }[];
}

/** Folders that grew or shrank between two measurements of the disk. */
export function DiskGrowth() {
  const [against, setAgainst] = useState<number | null>(null);
  const report = usePoll<GrowthReport>("get_folder_growth", { against }, 15000);
  const { result, busy, run } = useAction(() => report.reload());
  const r = report.data;
  if (!r) return null;
  const home = r.folders[0]?.path.match(/^\/home\/[^/]+/)?.[0];
  return (
    <section className="panel">
      <h2>What Grew On The Disk</h2>
      <div className="control-actions">
        <button className="ctl-btn" disabled={busy || r.running} onClick={() => run("snapshot_folders")}>
          {r.running ? "Measuring folders…" : "Measure now"}
        </button>
        {r.snapshots.length > 2 && (
          <select className="port-input" value={r.compared_to} onChange={(e) => setAgainst(parseInt(e.target.value))}>
            {r.snapshots.slice(1).map((ts) => (
              <option key={ts} value={ts}>
                compared with {formatDay(ts)}
              </option>
            ))}
          </select>
        )}
      </div>
      <StatusLine result={result} />
      {r.newest === 0 && <p className="diagnose-hint">No measurement yet. Trace takes one a week while plugged in; the first comparison needs two.</p>}
      {r.newest !== 0 && r.compared_to === 0 && (
        <p className="diagnose-hint">One measurement so far ({formatDay(r.newest)}). Changes show up after the next one.</p>
      )}
      {r.compared_to !== 0 && r.folders.length === 0 && (
        <p className="diagnose-hint">
          Nothing changed by more than 100 MB between {formatDay(r.compared_to)} and {formatDay(r.newest)}.
        </p>
      )}
      {r.folders.map((f) => {
        const delta = f.now - f.before;
        return (
          <div key={f.path} className="meta-row">
            <span className="folder-name" title={f.path}>
              {home ? f.path.replace(home, "~") : f.path}
            </span>
            <span className={delta > 0 ? "status-warn" : "status-ok"}>
              {delta > 0 ? "+" : "−"}
              {bytesToHuman(Math.abs(delta))} <span className="mem-muted">now {bytesToHuman(f.now)}</span>
            </span>
          </div>
        );
      })}
      {r.compared_to !== 0 && r.folders.length > 0 && (
        <p className="diagnose-hint">
          {formatDay(r.compared_to)} → {formatDay(r.newest)}. Only the deepest folder responsible is listed.
        </p>
      )}
    </section>
  );
}

interface Measurement {
  ts: number;
  label: string;
  seconds: number;
  cpu_percent: number;
  temp_c: number;
  watts: number | null;
  memory_stall: number;
  io_stall: number;
}

function delta(now: number, before: number | null | undefined, unit: string, digits = 1) {
  if (before == null) return null;
  const d = now - before;
  if (Math.abs(d) < 0.05) return null;
  // Lower is better for everything measured here.
  return (
    <span className={d < 0 ? "status-ok" : "status-warn"}>
      {" "}
      {d > 0 ? "+" : "−"}
      {Math.abs(d).toFixed(digits)}
      {unit}
    </span>
  );
}

/** Watch the machine for a while, change a setting, watch again, compare. */
export function MeasureEffect() {
  const list = usePoll<Measurement[]>("get_measurements");
  const [rows, setRows] = useState<Measurement[] | null>(null);
  const [label, setLabel] = useState("");
  const [seconds, setSeconds] = useState(60);
  const [left, setLeft] = useState(0);
  useEffect(() => {
    if (left <= 0) return;
    const t = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(t);
  }, [left]);
  const data = rows ?? list.data ?? [];
  async function run() {
    setLeft(seconds);
    try {
      setRows(await invoke("run_measurement", { label: label || (data.length ? "after" : "before"), seconds }));
      setLabel("");
    } finally {
      setLeft(0);
    }
  }
  return (
    <section className="panel wide">
      <h2>Did That Setting Help?</h2>
      <div className="control-actions">
        <input className="port-input" placeholder={data.length ? "name, e.g. after" : "name, e.g. before"} value={label} onChange={(e) => setLabel(e.target.value)} />
        <select className="port-input" value={seconds} disabled={left > 0} onChange={(e) => setSeconds(parseInt(e.target.value))}>
          <option value={30}>30 seconds</option>
          <option value={60}>1 minute</option>
          <option value={180}>3 minutes</option>
          <option value={300}>5 minutes</option>
        </select>
        <button className="ctl-btn ctl-primary" onClick={run} disabled={left > 0}>
          {left > 0 ? `Measuring… ${left}s` : "Take a measurement"}
        </button>
        {data.length > 0 && left === 0 && (
          <button
            className="ctl-btn"
            onClick={async () => {
              await invoke("clear_measurements");
              setRows([]);
            }}
          >
            Clear
          </button>
        )}
      </div>
      {data.length > 0 && (
        <table className="proc-table">
          <thead>
            <tr>
              <th>Measurement</th>
              <th>Processor</th>
              <th>Temperature</th>
              <th>Battery draw</th>
              <th>Waiting on memory</th>
              <th>Waiting on disk</th>
            </tr>
          </thead>
          <tbody>
            {data.map((m, i) => {
              const prev = data[i + 1];
              return (
                <tr key={m.ts}>
                  <td>
                    {m.label} <span className="mem-muted">{formatClock(m.ts)}</span>
                  </td>
                  <td>
                    {m.cpu_percent.toFixed(1)}%{delta(m.cpu_percent, prev?.cpu_percent, "")}
                  </td>
                  <td>
                    {m.temp_c.toFixed(1)}°C{delta(m.temp_c, prev?.temp_c, "°")}
                  </td>
                  <td>{m.watts == null ? "plugged in" : <>{m.watts.toFixed(1)} W{delta(m.watts, prev?.watts, " W")}</>}</td>
                  <td>
                    {m.memory_stall.toFixed(2)}%{delta(m.memory_stall, prev?.memory_stall, "", 2)}
                  </td>
                  <td>
                    {m.io_stall.toFixed(2)}%{delta(m.io_stall, prev?.io_stall, "", 2)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <p className="diagnose-hint">
        Take one measurement, change a setting, do the same thing on the machine again and take another. The coloured
        figure is the change from the row below (green is lower). Only a fair comparison if you were doing the same
        work both times.
      </p>
    </section>
  );
}

const ACCESS_ICON: Record<string, string> = { usb: "⌁", login: "✗", lid: "▭" };

/** Devices plugged in, wrong passwords and the lid, for the last week. */
export function AccessLog() {
  const { data } = usePoll<{ when: string; kind: string; text: string }[]>("get_access_log", undefined, 120000);
  const [kind, setKind] = useState("all");
  if (!data) return null;
  const shown = data.filter((e) => kind === "all" || e.kind === kind);
  return (
    <section className="panel">
      <h2>At The Machine, Last 7 Days</h2>
      <div className="chip-row">
        {[
          ["all", "Everything"],
          ["usb", "USB plugged in"],
          ["login", "Wrong passwords"],
          ["lid", "Lid"],
        ].map(([k, label]) => (
          <button key={k} className={`chip chip-button${kind === k ? " chip-on" : ""}`} onClick={() => setKind(k)}>
            {label} {k !== "all" && <span className="mem-muted">{data.filter((e) => e.kind === k).length}</span>}
          </button>
        ))}
      </div>
      {shown.length === 0 && <p className="diagnose-hint">Nothing recorded.</p>}
      <div className="event-scroll">
        {shown.map((e, i) => (
          <div key={i} className="plain-row">
            <span className="mem-muted">{e.when}</span> {ACCESS_ICON[e.kind]} <span className={e.kind === "login" ? "status-warn" : ""}>{e.text}</span>
          </div>
        ))}
      </div>
      <p className="diagnose-hint">A USB device or a wrong password at a time you weren't there is worth a second look.</p>
    </section>
  );
}

interface AppPeers {
  app: string;
  known_hosts: number;
  peers: { host: string; port: number; new: boolean; first_seen: number }[];
}

/** Which addresses each app is connected to, with newly seen ones marked. */
export function Connections() {
  const { data, reload } = usePoll<AppPeers[]>("get_connections", undefined, 20000);
  const [names, setNames] = useState<Record<string, string>>({});
  const [open, setOpen] = useState<string | null>(null);
  async function name(ip: string) {
    setNames((n) => ({ ...n, [ip]: "…" }));
    const found = await invoke<string>("lookup_host", { ip });
    setNames((n) => ({ ...n, [ip]: found || "no name" }));
  }
  if (!data) return null;
  return (
    <section className="panel">
      <h2>
        Who Each App Is Talking To{" "}
        <button className="chip chip-button" onClick={reload}>
          Refresh
        </button>
      </h2>
      {data.length === 0 && <p className="diagnose-hint">No app has a connection open right now.</p>}
      {data.map((a) => {
        const fresh = a.peers.filter((p) => p.new).length;
        return (
          <div key={a.app} className="finding">
            <button className="row-toggle" onClick={() => setOpen(open === a.app ? null : a.app)}>
              <strong>{a.app}</strong>
              <span className="mem-muted">
                {a.peers.length} open now · {a.known_hosts} seen in total
                {fresh > 0 && <span className="status-warn"> · {fresh} new today</span>}
              </span>
            </button>
            {open === a.app &&
              a.peers.map((p) => (
                <div key={`${p.host}:${p.port}`} className="plain-row">
                  {p.host}:{p.port}{" "}
                  {names[p.host] ? (
                    <span className="mem-muted">{names[p.host]}</span>
                  ) : (
                    <button className="chip chip-button" onClick={() => name(p.host)}>
                      Whose is it?
                    </button>
                  )}
                  {p.new && <span className="status-warn"> new</span>}
                </div>
              ))}
          </div>
        );
      })}
      <p className="diagnose-hint">
        Trace notes these once a minute and keeps the list on this machine. “New” means the app had not been seen
        talking to that address before today. Addresses, not countries: Trace ships no location database.
      </p>
    </section>
  );
}

interface WhatIsResult {
  name: string;
  what: string;
  safe_to_stop: string;
  note: string;
  source: string;
}

const SAFE_LABEL: Record<string, [string, string]> = {
  yes: ["Safe to stop", "status-ok"],
  careful: ["Stop with care", "status-warn"],
  no: ["Don't stop it", "status-fail"],
};

/** A plain explanation of a process or service name. */
export function WhatIs({ apps }: { apps: string[] }) {
  const [name, setName] = useState("");
  const [answer, setAnswer] = useState<WhatIsResult | null>(null);
  async function ask(n: string) {
    if (n.trim()) setAnswer(await invoke("what_is", { name: n.trim() }));
  }
  return (
    <section className="panel">
      <h2>What Is This?</h2>
      <div className="control-actions">
        <input
          className="port-input grow-input"
          list="whatis-names"
          placeholder="a process or service name, e.g. kswapd0"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && ask(name)}
        />
        <datalist id="whatis-names">
          {apps.map((a) => (
            <option key={a} value={a} />
          ))}
        </datalist>
        <button className="ctl-btn ctl-primary" disabled={!name.trim()} onClick={() => ask(name)}>
          Explain
        </button>
      </div>
      {answer &&
        (answer.what ? (
          <div className="explain">
            <strong>{answer.name}</strong> — {answer.what}
            {answer.safe_to_stop && (
              <div>
                <span className={SAFE_LABEL[answer.safe_to_stop][1]}>{SAFE_LABEL[answer.safe_to_stop][0]}.</span> {answer.note}
              </div>
            )}
            <div className="mem-muted">From {answer.source}.</div>
          </div>
        ) : (
          <p className="diagnose-hint">
            Trace doesn't know “{answer.name}” and no installed package claims it. It may be a script, or part of a larger
            app.
          </p>
        ))}
    </section>
  );
}

interface SpeedTest {
  ts: number;
  network: string;
  down_mbps: number;
  latency_ms: number;
}

/** A download speed test with a history per network. */
export function SpeedHistory() {
  const history = usePoll<SpeedTest[]>("get_speed_history");
  const [rows, setRows] = useState<SpeedTest[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const data = rows ?? history.data ?? [];
  async function run() {
    setBusy(true);
    setError(null);
    try {
      setRows(await invoke("run_speed_test"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const networks = [...new Set(data.map((d) => d.network))];
  const top = Math.max(...data.map((d) => d.down_mbps), 1);
  return (
    <section className="panel">
      <h2>Internet Speed</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? "Downloading 25 MB…" : "Test download speed"}
        </button>
      </div>
      {error && <p className="status-fail">✗ {error}</p>}
      {networks.map((n) => {
        const tests = data.filter((d) => d.network === n);
        const mean = tests.reduce((s, t) => s + t.down_mbps, 0) / tests.length;
        return (
          <div key={n}>
            <div className="meta-row">
              <strong>{n}</strong>
              <span>
                {tests[0].down_mbps.toFixed(0)} Mbit/s last · {mean.toFixed(0)} average of {tests.length} · {tests[0].latency_ms.toFixed(0)} ms to connect
              </span>
            </div>
            <div className="mini-bars" title="Each bar is one test, newest on the right">
              {[...tests].reverse().map((t) => (
                <span key={t.ts} style={{ height: `${Math.max(6, (t.down_mbps / top) * 100)}%` }} title={`${formatDay(t.ts)} ${formatClock(t.ts)} — ${t.down_mbps.toFixed(0)} Mbit/s`} />
              ))}
            </div>
          </div>
        );
      })}
      <p className="diagnose-hint">
        Each test downloads 25 MB, so think twice on a phone hotspot. Trace never runs it by itself.
      </p>
    </section>
  );
}

interface WifiAir {
  signal: number | null;
  channels: { channel: number; networks: number; strongest: number; yours: boolean }[];
  advice: string;
}

/** How crowded the Wi-Fi channels around you are. */
export function WifiChannels() {
  const [air, setAir] = useState<WifiAir | null>(null);
  const [busy, setBusy] = useState(false);
  async function load(rescan: boolean) {
    setBusy(true);
    try {
      setAir(await invoke("get_wifi_air", { rescan }));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    load(false);
  }, []);
  if (!air || air.channels.length === 0) return null;
  const top = Math.max(...air.channels.map((c) => c.networks));
  return (
    <section className="panel">
      <h2>Wi-Fi Around You</h2>
      <div className="control-actions">
        <button className="ctl-btn" onClick={() => load(true)} disabled={busy}>
          {busy ? "Scanning…" : "Scan again"}
        </button>
        {air.signal != null && air.signal > 0 && <span className="mem-muted">Your signal: {air.signal}%</span>}
      </div>
      {air.channels.map((c) => (
        <div key={c.channel} className="bar-row">
          <span className="bar-label">
            Channel {c.channel}
            {c.yours ? " (yours)" : ""}
          </span>
          <div className="bar-track">
            <div className={`bar-fill${c.yours ? " bar-own" : ""}`} style={{ width: `${(c.networks / top) * 100}%` }} />
          </div>
          <span className="bar-value">
            {c.networks} network{c.networks === 1 ? "" : "s"}
          </span>
        </div>
      ))}
      {air.advice && <p className="diagnose-hint">{air.advice}</p>}
    </section>
  );
}

/** A QR code a phone can scan to join a saved network. */
export function WifiQr({ uuid, name, onClose }: { uuid: string; name: string; onClose: () => void }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    invoke<string>("get_wifi_qr", { uuid })
      .then(setSvg)
      .catch((e) => setError(String(e)));
  }, [uuid]);
  return (
    <div className="qr-box">
      {error && <p className="status-fail">✗ {error}</p>}
      {svg && <img alt={`QR code to join ${name}`} src={`data:image/svg+xml;utf8,${encodeURIComponent(svg)}`} />}
      <span className="mem-muted">Point a phone camera at this to join {name}.</span>
      <button className="chip chip-button" onClick={onClose}>
        Hide
      </button>
    </div>
  );
}

/** How a drive's wear has moved since Trace first read it. */
export function DriveWear() {
  const { data } = usePoll<{ ts: number; device: string; wear_percent: number | null; power_on_hours: number | null }[]>("get_drive_wear");
  if (!data || data.length === 0) return null;
  const devices = [...new Set(data.map((d) => d.device))];
  return (
    <>
      {devices.map((dev) => {
        const points = data.filter((d) => d.device === dev && d.wear_percent != null);
        if (points.length === 0) return null;
        const [first, last] = [points[0], points[points.length - 1]];
        const days = (last.ts - first.ts) / 86400;
        const gained = last.wear_percent! - first.wear_percent!;
        return (
          <p key={dev} className="diagnose-hint">
            {dev}: {last.wear_percent}% of its rated life used
            {days >= 1
              ? `, ${gained === 0 ? "unchanged" : `up ${gained} point${gained === 1 ? "" : "s"}`} since ${formatDay(first.ts)}.` +
                (gained > 0 && days >= 30 ? ` At this rate it reaches 100% in about ${(((100 - last.wear_percent!) / gained) * days / 365).toFixed(0)} years.` : "")
              : ". Trace keeps a reading each time you check, so the trend shows up over the coming months."}
          </p>
        );
      })}
    </>
  );
}

/** Small buttons that remove confirm friction for trusted, reversible things. */
export function RowConfirm(props: { label: string; confirm: string; onConfirm: () => void; disabled?: boolean }) {
  return <ConfirmButton className="chip chip-button" label={props.label} confirmLabel={props.confirm} disabled={props.disabled} onConfirm={props.onConfirm} />;
}
