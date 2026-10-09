import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ConfirmButton, StatusLine, bytesToHuman, formatClock, formatDay, useAction, usePoll } from "./ui";
import { WifiQr } from "./Batch4";
import { GpuLoad, MediaHistory, VulnerablePackages } from "./Batch6";

const LEVEL_MARK: Record<string, [string, string]> = {
  good: ["✓", "status-ok"],
  warn: ["!", "status-warn"],
  bad: ["✗", "status-fail"],
  info: ["i", "mem-muted"],
};

interface SecurityReport {
  findings: { level: string; title: string; detail: string; fix: string; fix_label: string }[];
  failed_logins: string[];
}

/** A read-only look at what most often leaves a laptop exposed. */
export function SecurityCheckup() {
  const [report, setReport] = useState<SecurityReport | null>(null);
  const [busy, setBusy] = useState(false);
  const fix = useAction(() => run());
  async function run() {
    setBusy(true);
    try {
      setReport(await invoke("get_security_report"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel">
      <h2>Security Check-up</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? "Checking…" : report ? "Check again" : "Check this machine"}
        </button>
      </div>
      {!report && (
        <p className="diagnose-hint">
          Looks at the firewall, programs listening on the network, SSH, disk encryption, Secure Boot and failed
          logins. Checking changes nothing; where Trace can fix something, a button appears next to it.
        </p>
      )}
      {report?.findings.map((f) => {
        const [mark, cls] = LEVEL_MARK[f.level] ?? LEVEL_MARK.info;
        return (
          <div key={f.title} className="finding">
            <strong className={cls}>
              {mark} {f.title}
            </strong>
            {f.detail && <span className="mem-muted">{f.detail}</span>}
            {f.fix && (
              <div className="control-actions">
                <ConfirmButton className="ctl-btn ctl-btn-sm" label={f.fix_label} confirmLabel="Asks for your password — go ahead?" disabled={fix.busy} onConfirm={() => fix.run("apply_security_fix", { id: f.fix })} />
              </div>
            )}
          </div>
        );
      })}
      <StatusLine result={fix.result} />
      {report && <VulnerablePackages />}
      {report && report.failed_logins.length > 0 && (
        <>
          <h3 className="sub-heading">Failed logins, last 7 days</h3>
          <div className="event-scroll">
            {report.failed_logins.map((l, i) => (
              <div key={i} className="plain-row">
                {l}
              </div>
            ))}
          </div>
        </>
      )}
    </section>
  );
}

/** Which programs are using the microphone or camera right now. */
export function MediaUseCard() {
  const { data } = usePoll<{ microphone: string[]; camera: string[] }>("get_media_use", undefined, 5000);
  if (!data) return null;
  const row = (label: string, users: string[]) => (
    <div className="meta-row">
      <span>{label}</span>
      {users.length === 0 ? (
        <span className="mem-muted">not in use</span>
      ) : (
        <span className="status-warn">in use by {users.join(", ")}</span>
      )}
    </div>
  );
  return (
    <section className="panel">
      <h2>Microphone & Camera</h2>
      {row("Microphone", data.microphone)}
      {row("Camera", data.camera)}
      <MediaHistory />
    </section>
  );
}

interface SavedWifi {
  name: string;
  uuid: string;
  autoconnect: boolean;
  last_used: number;
  metered: boolean;
}

/** Saved Wi-Fi networks: show the password, stop joining automatically, forget. */
export function SavedWifiCard() {
  const list = usePoll<SavedWifi[]>("list_saved_wifi", undefined, 60000);
  const { result, busy, run } = useAction(() => list.reload());
  const [qr, setQr] = useState<SavedWifi | null>(null);
  if (!list.data || list.data.length === 0) return null;
  return (
    <section className="panel wide">
      <h2>Saved Wi-Fi Networks</h2>
      <div className="event-scroll">
        {list.data.map((w) => (
          <div key={w.uuid} className="wifi-row">
            <span className="wifi-name" title={w.name}>
              <strong>{w.name}</strong>
              <span className="mem-muted">
                {" "}
                {w.last_used ? formatDay(w.last_used) : "never used"}
                {w.autoconnect ? "" : " · manual"}
              </span>
            </span>
            <button className="chip chip-button" disabled={busy} onClick={() => run("saved_wifi_action", { uuid: w.uuid, action: "password" })}>
              Password
            </button>
            <button className="chip chip-button" onClick={() => setQr(w)}>
              QR
            </button>
            <button
              className={`chip chip-button${w.metered ? " chip-on" : ""}`}
              disabled={busy}
              title="Mark as a connection where data costs money, such as a phone hotspot"
              onClick={() => run("saved_wifi_action", { uuid: w.uuid, action: w.metered ? "metered_off" : "metered_on" })}
            >
              Limited data: {w.metered ? "on" : "off"}
            </button>
            <button
              className="chip chip-button"
              disabled={busy}
              title={w.autoconnect ? "Stop joining this network by itself" : "Join this network by itself when in range"}
              onClick={() => run("saved_wifi_action", { uuid: w.uuid, action: w.autoconnect ? "auto_off" : "auto_on" })}
            >
              {w.autoconnect ? "Auto-join" : "Manual"}
            </button>
            <ConfirmButton
              className="chip chip-button"
              label="Forget"
              confirmLabel="Sure?"
              disabled={busy}
              onConfirm={() => run("saved_wifi_action", { uuid: w.uuid, action: "forget" })}
            />
          </div>
        ))}
      </div>
      {qr && <WifiQr uuid={qr.uuid} name={qr.name} onClose={() => setQr(null)} />}
      <StatusLine result={result} />
    </section>
  );
}

interface PrivacyItem {
  id: string;
  title: string;
  detail: string;
  bytes: number;
}

/** Traces of what you did: recent-file lists, browser caches, clipboard history. */
export function PrivacyCleaner() {
  const [items, setItems] = useState<PrivacyItem[] | null>(null);
  const [busy, setBusy] = useState(false);
  async function load() {
    setBusy(true);
    try {
      setItems(await invoke("get_privacy_items"));
    } finally {
      setBusy(false);
    }
  }
  const action = useAction(load);
  return (
    <section className="panel">
      <h2>Privacy Cleaner</h2>
      {!items && (
        <div className="control-actions">
          <button className="ctl-btn ctl-primary" onClick={load} disabled={busy}>
            {busy ? "Looking…" : "Show what's kept about my activity"}
          </button>
        </div>
      )}
      {items?.length === 0 && <p className="diagnose-hint">Nothing found to clear.</p>}
      {items?.map((item) => (
        <div key={item.id} className="finding">
          <div className="control-row">
            <label>
              <strong>{item.title}</strong>
              {item.bytes >= 1048576 && <span className="mem-muted"> — {bytesToHuman(item.bytes)}</span>}
            </label>
            <ConfirmButton
              label="Clear"
              confirmLabel="Clear it?"
              disabled={action.busy}
              onConfirm={() => action.run("clean_privacy_item", { id: item.id })}
            />
          </div>
          <span className="mem-muted">{item.detail}</span>
        </div>
      ))}
      <StatusLine result={action.result} />
    </section>
  );
}

/** Start a program cut off from the network and/or capped in memory and CPU. */
export function RunRestricted() {
  const [command, setCommand] = useState("");
  const [noNetwork, setNoNetwork] = useState(true);
  const [readOnly, setReadOnly] = useState(false);
  const [memory, setMemory] = useState("");
  const [cpu, setCpu] = useState("");
  const { result, busy, run } = useAction();
  return (
    <section className="panel">
      <h2>Run With Restrictions</h2>
      <div className="control-row">
        <input
          className="port-input grow-input"
          placeholder="command, e.g. ./some-tool --flag"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
        />
      </div>
      <div className="control-row">
        <label>
          <input type="checkbox" checked={noNetwork} onChange={(e) => setNoNetwork(e.target.checked)} /> No network at all
        </label>
        <label>
          <input type="checkbox" checked={readOnly} onChange={(e) => setReadOnly(e.target.checked)} /> Can't change any file
        </label>
        <input
          className="port-input number-box"
          placeholder="MB"
          aria-label="Memory limit in megabytes"
          value={memory}
          onChange={(e) => setMemory(e.target.value.replace(/\D/g, ""))}
        />
        <input
          className="port-input number-box"
          placeholder="CPU %"
          aria-label="CPU limit in percent of one core"
          value={cpu}
          onChange={(e) => setCpu(e.target.value.replace(/\D/g, ""))}
        />
        <button
          className="ctl-btn ctl-primary"
          disabled={busy || !command.trim()}
          onClick={() =>
            run("run_restricted", {
              command,
              noNetwork,
              readOnly,
              memoryMb: memory ? parseInt(memory) : null,
              cpuPercent: cpu ? parseInt(cpu) : null,
            })
          }
        >
          Start
        </button>
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        For tools you don't fully trust. "No network" gives the program an empty network of its own, so it can't reach
        the internet or your local machines. The memory limit is a hard stop (the program is ended if it goes over);
        CPU is in percent of one core, so 200 means two cores. “Can't change any file” lets it read everything but write only to a throwaway /tmp of its own; programs that need to save settings may refuse to start that way.
      </p>
    </section>
  );
}

interface StressResult {
  seconds: number;
  start_c: number;
  peak_c: number;
  end_c: number;
  lowest_speed_percent: number;
  peak_fan_rpm: number;
  verdict: string;
}

/** Loads every core for a short time and reports how the cooling coped. */
export function StressTest() {
  const [seconds, setSeconds] = useState(30);
  const [result, setResult] = useState<StressResult | null>(null);
  const [left, setLeft] = useState(0);
  const [graphics, setGraphics] = useState(false);
  useEffect(() => {
    if (left <= 0) return;
    const t = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(t);
  }, [left]);
  async function run() {
    setLeft(seconds);
    setResult(null);
    try {
      setResult(await invoke("run_stress_test", { seconds }));
    } finally {
      setLeft(0);
    }
  }
  const busy = left > 0;
  return (
    <section className="panel">
      <h2>Cooling Test</h2>
      <div className="control-actions">
        <select className="port-input" value={seconds} disabled={busy} onChange={(e) => setSeconds(parseInt(e.target.value))}>
          <option value={30}>30 seconds</option>
          <option value={60}>1 minute</option>
          <option value={120}>2 minutes</option>
        </select>
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? `Running at full load… ${left}s` : result ? "Run again" : "Run every core flat out"}
        </button>
        <label className="mem-muted">
          <input type="checkbox" checked={graphics} disabled={busy} onChange={(e) => setGraphics(e.target.checked)} /> load the graphics chip too
        </label>
      </div>
      {busy && graphics && <GpuLoad />}
      {result && (
        <>
          <div className="explain">
            <strong>{result.verdict}</strong>
          </div>
          <div className="meta-row">
            <span>Temperature</span>
            <span>
              {result.start_c.toFixed(0)}°C → peak {result.peak_c.toFixed(0)}°C
            </span>
          </div>
          <div className="meta-row">
            <span>Lowest speed under load</span>
            <span>{result.lowest_speed_percent.toFixed(0)}% of its top speed</span>
          </div>
          {result.peak_fan_rpm > 0 && (
            <div className="meta-row">
              <span>Fan at most</span>
              <span>{result.peak_fan_rpm} RPM</span>
            </div>
          )}
        </>
      )}
      <p className="diagnose-hint">
        The fans will get loud and the machine will be sluggish while it runs. It stops by itself at 98°C. Run it
        before and after cleaning the vents to see whether it helped.
      </p>
    </section>
  );
}

/** One file with everything Trace manages, for a new install or a second machine. */
export function SettingsTransfer() {
  const { result, busy, run } = useAction();
  async function load() {
    const path = await openDialog({ multiple: false, filters: [{ name: "Trace settings", extensions: ["json"] }] });
    if (typeof path === "string") run("import_settings", { path });
  }
  return (
    <>
      <h3 className="sub-heading">Move to another machine</h3>
      <div className="control-actions">
        <button className="ctl-btn" disabled={busy} onClick={() => run("export_settings")}>
          Save everything to a file
        </button>
        <button className="ctl-btn" disabled={busy} onClick={load}>
          Load from a file…
        </button>
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        The file holds your rules, schedules, profiles, saved presets, display layouts and the settings you chose to
        keep. Loading replaces rules and schedules; restart Trace afterwards.
      </p>
    </>
  );
}

interface Leak {
  app: string;
  hours: number;
  start_bytes: number;
  end_bytes: number;
  per_hour_bytes: number;
}

/** Apps whose memory has only gone up over the last day. */
export function Leaks() {
  const leaks = usePoll<Leak[]>("get_leaks", undefined, 300000);
  const { result, busy, run } = useAction(() => leaks.reload());
  if (!leaks.data || leaks.data.length === 0) return null;
  return (
    <section className="panel">
      <h2>Possible Memory Leaks</h2>
      {leaks.data.map((l) => (
        <div key={l.app} className="control-row">
          <label>
            <strong>{l.app}</strong>
            <span className="mem-muted">
              {" "}
              — {bytesToHuman(l.start_bytes)} → {bytesToHuman(l.end_bytes)} over {l.hours.toFixed(0)} h, about{" "}
              {bytesToHuman(l.per_hour_bytes)} more each hour
            </span>
          </label>
          <ConfirmButton label="Restart it" confirmLabel={`Restart ${l.app}?`} disabled={busy} onConfirm={() => run("app_action", { name: l.app, action: "restart" })} />
        </div>
      ))}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        These kept growing without ever shrinking. Restarting gives the memory back; if one shows up here every day,
        it is the app's fault, not yours.
      </p>
    </section>
  );
}

/** Freeze chosen apps for a set time, then let them run again. */
export function FocusMode({ apps }: { apps: string[] }) {
  const focus = usePoll<{ apps: string[]; until: number } | null>("get_focus", undefined, 15000);
  const { result, busy, run } = useAction(() => focus.reload());
  const [chosen, setChosen] = useState<string[]>([]);
  const [minutes, setMinutes] = useState(45);
  const active = focus.data;
  return (
    <section className="panel">
      <h2>Focus Time</h2>
      {active ? (
        <>
          <div className="explain">
            <strong>{active.apps.join(", ")}</strong> frozen until {formatClock(active.until)}.
          </div>
          <div className="control-actions">
            <button className="ctl-btn ctl-primary" disabled={busy} onClick={() => run("set_focus", { apps: [], minutes: 0 })}>
              End now
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="chip-row">
            {apps.slice(0, 16).map((a) => (
              <button
                key={a}
                className={`chip chip-button${chosen.includes(a) ? " chip-on" : ""}`}
                onClick={() => setChosen(chosen.includes(a) ? chosen.filter((c) => c !== a) : [...chosen, a])}
              >
                {a}
              </button>
            ))}
          </div>
          <div className="control-actions">
            <select className="port-input" value={minutes} onChange={(e) => setMinutes(parseInt(e.target.value))}>
              {[15, 25, 45, 60, 90, 120].map((m) => (
                <option key={m} value={m}>
                  {m} minutes
                </option>
              ))}
            </select>
            <button className="ctl-btn ctl-primary" disabled={busy || chosen.length === 0} onClick={() => run("set_focus", { apps: chosen, minutes })}>
              Freeze {chosen.length || ""} app{chosen.length === 1 ? "" : "s"}
            </button>
          </div>
        </>
      )}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Frozen apps stay open but can't run, use the processor or make noise until the time is up. They wake by
        themselves, also if Trace is quit or restarted.
      </p>
    </section>
  );
}
