import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Segmented, usePoll } from "./ui";

interface Rule {
  id: string;
  app: string;
  threshold_mb: number;
  include_swap: boolean;
  action: string;
  enabled: boolean;
}

type Trigger =
  | { type: "Time"; hour: number; minute: number; days: number[] }
  | { type: "AppLaunch"; app: string };

interface Schedule {
  id: string;
  profile: string;
  trigger: Trigger;
  enabled: boolean;
}

interface Config {
  alerts_enabled: boolean;
  close_to_tray: boolean;
  rules: Rule[];
  schedules: Schedule[];
}

type BuilderAction =
  | { type: "FreezeApp" | "ResumeApp" | "QuitApp"; app: string }
  | { type: "SetPowerProfile"; profile: string }
  | { type: "StartService" | "StopService" | "RestartService"; name: string }
  | { type: "ApplyPreset"; preset: string };

interface Profile {
  name: string;
  description: string;
  actions: unknown[];
}

const RULE_ACTIONS: Record<string, string> = {
  notify: "notify me",
  freeze: "freeze it",
  restart: "restart it",
  kill: "quit it",
};

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const BUILDER_TYPES: { type: BuilderAction["type"]; label: string; field: string }[] = [
  { type: "FreezeApp", label: "Freeze app", field: "app" },
  { type: "ResumeApp", label: "Resume app", field: "app" },
  { type: "QuitApp", label: "Quit app", field: "app" },
  { type: "SetPowerProfile", label: "Set power profile", field: "profile" },
  { type: "ApplyPreset", label: "Apply settings preset", field: "preset" },
  { type: "StartService", label: "Start service", field: "name" },
  { type: "StopService", label: "Stop service", field: "name" },
  { type: "RestartService", label: "Restart service", field: "name" },
];

function newId(): string {
  return Math.random().toString(36).slice(2, 10);
}

function describeTrigger(t: Trigger): string {
  if (t.type === "AppLaunch") return `when ${t.app} starts`;
  const time = `${String(t.hour).padStart(2, "0")}:${String(t.minute).padStart(2, "0")}`;
  const days = t.days.length === 0 || t.days.length === 7
    ? "every day"
    : t.days.map((d) => DAYS[d]).join(", ");
  return `at ${time}, ${days}`;
}

function AppInput({
  value,
  onChange,
  apps,
}: {
  value: string;
  onChange: (v: string) => void;
  apps: string[];
}) {
  return (
    <>
      <input
        className="port-input"
        list="trace-running-apps"
        placeholder="app name, e.g. brave"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <datalist id="trace-running-apps">
        {apps.map((a) => (
          <option key={a} value={a} />
        ))}
      </datalist>
    </>
  );
}

function Rules({
  config,
  save,
  apps,
}: {
  config: Config;
  save: (c: Config) => void;
  apps: string[];
}) {
  const [app, setApp] = useState("");
  const [threshold, setThreshold] = useState("4096");
  const [includeSwap, setIncludeSwap] = useState(true);
  const [action, setAction] = useState("notify");
  const thresholdMb = parseInt(threshold, 10);

  return (
    <section className="panel">
      <h2>Automatic Rules</h2>
      {config.rules.length === 0 && (
        <p className="diagnose-hint">
          No rules yet. A rule watches one app's memory and acts when it
          crosses a limit.
        </p>
      )}
      {config.rules.map((r) => (
        <div key={r.id} className="control-row">
          <label className={r.enabled ? "" : "mem-muted"}>
            If <strong>{r.app}</strong> uses more than{" "}
            {(r.threshold_mb / 1024).toFixed(1)} GB
            {r.include_swap ? " (RAM + swap)" : " of RAM"}, {RULE_ACTIONS[r.action]}
          </label>
          <button
            className="ctl-btn"
            onClick={() =>
              save({
                ...config,
                rules: config.rules.map((x) =>
                  x.id === r.id ? { ...x, enabled: !x.enabled } : x
                ),
              })
            }
          >
            {r.enabled ? "Pause" : "Enable"}
          </button>
          <button
            className="ctl-btn ctl-danger"
            onClick={() =>
              save({ ...config, rules: config.rules.filter((x) => x.id !== r.id) })
            }
          >
            Delete
          </button>
        </div>
      ))}

      <h3 className="sub-heading">New rule</h3>
      <div className="control-row">
        <label>If this app</label>
        <AppInput value={app} onChange={setApp} apps={apps} />
      </div>
      <div className="control-row">
        <label>uses more than (MB)</label>
        <input
          className="port-input small"
          value={threshold}
          onChange={(e) => setThreshold(e.target.value)}
        />
        <label className="check-row">
          <input
            type="checkbox"
            checked={includeSwap}
            onChange={(e) => setIncludeSwap(e.target.checked)}
          />
          count swap too
        </label>
      </div>
      <div className="control-row">
        <label>then</label>
        <select className="port-input" value={action} onChange={(e) => setAction(e.target.value)}>
          {Object.entries(RULE_ACTIONS).map(([key, label]) => (
            <option key={key} value={key}>
              {label}
            </option>
          ))}
        </select>
        <button
          className="ctl-btn ctl-primary"
          disabled={!app.trim() || !(thresholdMb > 0)}
          onClick={() => {
            save({
              ...config,
              rules: [
                ...config.rules,
                {
                  id: newId(),
                  app: app.trim(),
                  threshold_mb: thresholdMb,
                  include_swap: includeSwap,
                  action,
                  enabled: true,
                },
              ],
            });
            setApp("");
          }}
        >
          Add rule
        </button>
      </div>
      <p className="diagnose-hint">
        Rules are checked once a minute while Trace is running and fire at
        most once every 10 minutes each. "Quit" and "restart" lose unsaved
        work in that app; every firing is recorded in the action log.
      </p>
    </section>
  );
}

function Schedules({
  config,
  save,
  apps,
  profiles,
}: {
  config: Config;
  save: (c: Config) => void;
  apps: string[];
  profiles: Profile[];
}) {
  const [profile, setProfile] = useState("");
  const [kind, setKind] = useState("at a time");
  const [time, setTime] = useState("09:00");
  const [days, setDays] = useState<number[]>([]);
  const [app, setApp] = useState("");
  const chosen = profile || profiles[0]?.name || "";

  function add() {
    const [hour, minute] = time.split(":").map((n) => parseInt(n, 10));
    const trigger: Trigger =
      kind === "at a time"
        ? { type: "Time", hour, minute, days }
        : { type: "AppLaunch", app: app.trim() };
    save({
      ...config,
      schedules: [
        ...config.schedules,
        { id: newId(), profile: chosen, trigger, enabled: true },
      ],
    });
  }

  return (
    <section className="panel">
      <h2>Scheduled Profiles</h2>
      {config.schedules.length === 0 && (
        <p className="diagnose-hint">
          Nothing scheduled. A schedule applies one of your profiles at a set
          time or when an app starts.
        </p>
      )}
      {config.schedules.map((s) => (
        <div key={s.id} className="control-row">
          <label className={s.enabled ? "" : "mem-muted"}>
            Apply <strong>{s.profile}</strong> {describeTrigger(s.trigger)}
          </label>
          <button
            className="ctl-btn"
            onClick={() =>
              save({
                ...config,
                schedules: config.schedules.map((x) =>
                  x.id === s.id ? { ...x, enabled: !x.enabled } : x
                ),
              })
            }
          >
            {s.enabled ? "Pause" : "Enable"}
          </button>
          <button
            className="ctl-btn ctl-danger"
            onClick={() =>
              save({
                ...config,
                schedules: config.schedules.filter((x) => x.id !== s.id),
              })
            }
          >
            Delete
          </button>
        </div>
      ))}

      <h3 className="sub-heading">New schedule</h3>
      <div className="control-row">
        <label>Apply profile</label>
        <select className="port-input" value={chosen} onChange={(e) => setProfile(e.target.value)}>
          {profiles.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
        <Segmented options={["at a time", "when an app starts"]} value={kind} onChange={setKind} />
      </div>
      {kind === "at a time" ? (
        <div className="control-row">
          <label>Time and days (none selected = every day)</label>
          <input
            className="port-input small"
            type="time"
            value={time}
            onChange={(e) => setTime(e.target.value)}
          />
          <div className="segmented">
            {DAYS.map((d, i) => (
              <button
                key={d}
                className={days.includes(i) ? "range-btn active" : "range-btn"}
                onClick={() =>
                  setDays(days.includes(i) ? days.filter((x) => x !== i) : [...days, i].sort())
                }
              >
                {d}
              </button>
            ))}
          </div>
        </div>
      ) : (
        <div className="control-row">
          <label>When this app starts</label>
          <AppInput value={app} onChange={setApp} apps={apps} />
        </div>
      )}
      <div className="control-actions">
        <button
          className="ctl-btn ctl-primary"
          disabled={!chosen || (kind !== "at a time" && !app.trim())}
          onClick={add}
        >
          Add schedule
        </button>
      </div>
      <p className="diagnose-hint">
        Schedules only run while Trace is open. Profiles with no actions do
        nothing — build one below first.
      </p>
    </section>
  );
}

function ProfileBuilder({ apps, onSaved }: { apps: string[]; onSaved: () => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [actions, setActions] = useState<BuilderAction[]>([]);
  const [type, setType] = useState<BuilderAction["type"]>("FreezeApp");
  const [target, setTarget] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const spec = BUILDER_TYPES.find((t) => t.type === type)!;

  async function saveProfile() {
    try {
      await invoke("save_profile", {
        profile: { name: name.trim(), description, actions },
      });
      setMessage(`✓ Saved "${name.trim()}" — it now appears under Profiles`);
      setName("");
      setDescription("");
      setActions([]);
      onSaved();
    } catch (e) {
      setMessage(`✗ ${e}`);
    }
  }

  return (
    <section className="panel">
      <h2>Build a Profile</h2>
      <div className="control-row">
        <label>Name</label>
        <input
          className="port-input"
          placeholder="Gaming Mode"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          className="port-input"
          placeholder="What it is for (optional)"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </div>
      <ol className="profile-actions">
        {actions.map((a, i) => (
          <li key={i}>
            {BUILDER_TYPES.find((t) => t.type === a.type)!.label}:{" "}
            {"app" in a ? a.app : "profile" in a ? a.profile : "preset" in a ? a.preset : a.name}{" "}
            <button
              className="chip chip-button"
              onClick={() => setActions(actions.filter((_, j) => j !== i))}
            >
              remove
            </button>
          </li>
        ))}
      </ol>
      <div className="control-row">
        <label>Add a step</label>
        <select
          className="port-input"
          value={type}
          onChange={(e) => {
            setType(e.target.value as BuilderAction["type"]);
            setTarget("");
          }}
        >
          {BUILDER_TYPES.map((t) => (
            <option key={t.type} value={t.type}>
              {t.label}
            </option>
          ))}
        </select>
        {spec.field === "app" ? (
          <AppInput value={target} onChange={setTarget} apps={apps} />
        ) : (
          <input
            className="port-input"
            placeholder={
              spec.field === "profile"
                ? "performance"
                : spec.field === "preset"
                ? "Gaming (a preset name from Tweaks)"
                : "bluetooth.service"
            }
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          />
        )}
        <button
          className="ctl-btn"
          disabled={!target.trim()}
          onClick={() => {
            setActions([
              ...actions,
              { type, [spec.field]: target.trim() } as BuilderAction,
            ]);
            setTarget("");
          }}
        >
          Add step
        </button>
      </div>
      <div className="control-actions">
        <button
          className="ctl-btn ctl-primary"
          disabled={!name.trim() || actions.length === 0}
          onClick={saveProfile}
        >
          Save profile
        </button>
      </div>
      {message && (
        <p className={message.startsWith("✓") ? "status-ok" : "status-fail"}>{message}</p>
      )}
      <p className="diagnose-hint">
        Saving under an existing name replaces that profile. Freeze and
        resume steps can be undone from Profiles; quitting an app or changing
        the power profile can't.
      </p>
    </section>
  );
}

export default function Automation({ onProfilesChanged }: { onProfilesChanged: () => void }) {
  const config = usePoll<Config>("get_automation");
  const profiles = usePoll<Profile[]>("list_profiles");
  const memory = usePoll<{ apps: { name: string }[] }>("get_memory_breakdown");
  const [error, setError] = useState<string | null>(null);
  if (!config.data) return null;
  const apps = (memory.data?.apps ?? []).slice(0, 40).map((a) => a.name);

  async function save(next: Config) {
    try {
      await invoke("save_automation", { config: next });
      setError(null);
    } catch (e) {
      setError(String(e));
    }
    config.reload();
  }

  return (
    <>
      <section className="panel">
        <h2>Alerts</h2>
        <div className="control-row">
          <label>
            Desktop notification when something turns critical (RAM nearly
            gone, swap or a disk almost full, overheating, memory stalls)
          </label>
          <Segmented
            options={["on", "off"]}
            value={config.data.alerts_enabled ? "on" : "off"}
            onChange={(v) => save({ ...config.data!, alerts_enabled: v === "on" })}
          />
        </div>
        {error && <p className="status-fail">✗ {error}</p>}
        <p className="diagnose-hint">
          Each alert repeats at most once every 30 minutes, and only while
          Trace is running.
        </p>
      </section>
      <Rules config={config.data} save={save} apps={apps} />
      <Schedules
        config={config.data}
        save={save}
        apps={apps}
        profiles={profiles.data ?? []}
      />
      <ProfileBuilder
        apps={apps}
        onSaved={() => {
          profiles.reload();
          onProfilesChanged();
        }}
      />
    </>
  );
}
