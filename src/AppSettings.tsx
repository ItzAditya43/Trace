import { invoke } from "@tauri-apps/api/core";
import { SettingsTransfer } from "./Batch3";
import { Segmented, StatusLine, formatClock, formatDay, useAction, usePoll } from "./ui";

interface Config {
  alerts_enabled: boolean;
  close_to_tray: boolean;
  rules: unknown[];
  schedules: unknown[];
}

interface AutostartStatus {
  enabled: boolean;
  file: string;
  method: string;
}

interface Notification {
  ts: number;
  title: string;
  body: string;
  critical: boolean;
}

/** Settings for Trace itself: background running, start at login, alert history. */
export default function AppSettings() {
  const info = usePoll<{ version: string; tray: boolean }>("get_app_info");
  const config = usePoll<Config>("get_automation");
  const autostart = usePoll<AutostartStatus>("get_autostart");
  const sent = usePoll<Notification[]>("get_notifications", undefined, 30000);
  const { result, busy, run } = useAction(() => autostart.reload());

  async function setCloseToTray(on: boolean) {
    if (!config.data) return;
    await invoke("save_automation", { config: { ...config.data, close_to_tray: on } });
    config.reload();
  }

  return (
    <section className="panel">
      <h2>Trace Itself {info.data && <span className="mem-muted">v{info.data.version}</span>}</h2>
      <div className="control-row">
        <label>
          Keep running when the window is closed
          <span className="mem-muted">
            {" "}
            — Trace keeps recording and running rules, schedules and alerts
          </span>
        </label>
        <Segmented
          options={["on", "off"]}
          value={config.data?.close_to_tray ? "on" : "off"}
          disabled={!info.data?.tray}
          onChange={(v) => setCloseToTray(v === "on")}
        />
      </div>
      <p className="diagnose-hint">
        {info.data && !info.data.tray
          ? "This desktop has no tray support, so closing the window quits Trace."
          : "With this on, the tray icon's menu shows live CPU and RAM and has Open and Quit. If your bar has no tray area, opening Trace again from the launcher brings the window back."}
      </p>

      <div className="control-row">
        <label>
          Start at login
          <span className="mem-muted"> — hidden, in the tray</span>
        </label>
        <Segmented
          options={["on", "off"]}
          value={autostart.data?.enabled ? "on" : "off"}
          disabled={busy || !autostart.data}
          onChange={(v) => run("set_autostart", { enabled: v === "on" })}
        />
      </div>
      {autostart.data && (
        <p className="diagnose-hint">
          Done through your {autostart.data.method}: Trace adds (or removes) a clearly marked block in{" "}
          {autostart.data.file}. Nothing else in that file is touched.
        </p>
      )}
      <StatusLine result={result} />

      <SettingsTransfer />

      <h3 className="sub-heading">Alerts Trace has sent</h3>
      {(sent.data ?? []).length === 0 && <p className="diagnose-hint">None yet.</p>}
      <div className="event-scroll">
        {(sent.data ?? []).map((n, i) => (
          <div key={i} className="event-row">
            <span className="event-time">
              {formatDay(n.ts)} {formatClock(n.ts)}
            </span>
            <span className="event-message">
              <strong>{n.title}</strong> <span className="mem-muted">{n.body}</span>
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}
