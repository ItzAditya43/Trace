import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ActionResult, ConfirmButton, Segmented, StatusLine, bytesToHuman, formatDay, useAction, usePoll } from "./ui";
import { openUrl } from "@tauri-apps/plugin-opener";

async function pickFolder(): Promise<string | null> {
  const path = await openDialog({ directory: true, multiple: false });
  return typeof path === "string" ? path : null;
}

interface MaintenanceStatus {
  last_run: number;
  last_report: string[];
  journal_bytes: number;
  package_cache_bytes: number;
  crash_dump_bytes: number;
  docker: boolean;
}

/** Trim, old packages, old logs, crash dumps and Docker leftovers in one go. */
export function Maintenance() {
  const status = usePoll<MaintenanceStatus>("get_maintenance", undefined, 300000);
  const [report, setReport] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    setError(null);
    try {
      setReport(await invoke("run_maintenance"));
      status.reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const s = status.data;
  if (!s) return null;
  const shown = report ?? s.last_report;
  const overdue = s.last_run > 0 && Date.now() / 1000 - s.last_run > 7 * 86400;
  return (
    <section className="panel">
      <h2>Weekly Maintenance</h2>
      <div className="meta-row">
        <span>Last run</span>
        <span className={overdue ? "status-warn" : ""}>{s.last_run ? formatDay(s.last_run) : "never"}</span>
      </div>
      <p className="diagnose-hint">
        Now: system log {bytesToHuman(s.journal_bytes)} · package cache {bytesToHuman(s.package_cache_bytes)} · crash dumps{" "}
        {bytesToHuman(s.crash_dump_bytes)}
      </p>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={run} disabled={busy}>
          {busy ? "Working… (asks for your password once)" : "Run maintenance now"}
        </button>
      </div>
      {error && <p className="status-fail">✗ {error}</p>}
      {shown.map((line, i) => (
        <div key={i} className="plain-row">
          ✓ {line}
        </div>
      ))}
      <p className="diagnose-hint">
        Trims the drives, keeps the two newest versions of each package, cuts the system log to 500 MB, removes crash
        dumps older than a week{s.docker ? " and clears unused Docker leftovers" : ""}. It needs your password, so Trace
        reminds you when a week has passed rather than running it behind your back.
      </p>
    </section>
  );
}

interface WorkStatus {
  has_compose: boolean;
  command: string;
  suggested: string;
  command_running: boolean;
  containers_up: number;
}

/** Start or stop everything a project needs: containers and its dev command. */
export function WorkMode({ project }: { project: string }) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<WorkStatus | null>(null);
  const [command, setCommand] = useState("");
  const [log, setLog] = useState<string | null>(null);
  async function load() {
    const s = await invoke<WorkStatus>("get_work_status", { path: project });
    setStatus(s);
    setCommand((c) => c || s.command || s.suggested);
  }
  const { result, busy, run } = useAction(load);
  if (!open) {
    return (
      <button
        className="ctl-btn ctl-btn-sm"
        onClick={() => {
          setOpen(true);
          load();
        }}
      >
        Work mode…
      </button>
    );
  }
  if (!status) return null;
  const on = status.command_running || status.containers_up > 0;
  return (
    <div className="work-box">
      <div className="control-row">
        <input className="port-input grow-input" placeholder="dev command, e.g. npm run dev" value={command} disabled={on} onChange={(e) => setCommand(e.target.value)} />
        {on ? (
          <button className="ctl-btn" disabled={busy} onClick={() => run("set_work_mode", { path: project, on: false, command: "" })}>
            Stop working
          </button>
        ) : (
          <button className="ctl-btn ctl-primary" disabled={busy || (!command.trim() && !status.has_compose)} onClick={() => run("set_work_mode", { path: project, on: true, command })}>
            Start working
          </button>
        )}
      </div>
      <span className="mem-muted">
        {status.has_compose ? `Compose file found: ${status.containers_up} container(s) up. ` : "No compose file. "}
        {status.command_running ? "Command is running. " : ""}
        {status.command_running && (
          <button className="chip chip-button" onClick={async () => setLog(await invoke("get_work_log", { path: project }))}>
            Show its output
          </button>
        )}
      </span>
      {log != null && <pre className="log-box">{log || "(nothing printed yet)"}</pre>}
      <StatusLine result={result} />
    </div>
  );
}

interface UpdateBrief {
  count: number;
  needs_reboot: string[];
  big: string[];
  news: { title: string; date: string; link: string; manual: boolean }[];
  pacnew: string[];
  stale_apps: string[];
  can_open_terminal: boolean;
}

/** What an update would change and what to know before running it. */
export function UpdateAssistant() {
  const [brief, setBrief] = useState<UpdateBrief | null>(null);
  const [busy, setBusy] = useState(false);
  const [diff, setDiff] = useState<{ path: string; text: string } | null>(null);
  async function load() {
    setBusy(true);
    try {
      setBrief(await invoke("get_update_brief"));
    } finally {
      setBusy(false);
    }
  }
  const action = useAction(load);
  async function showDiff(path: string) {
    try {
      setDiff({ path, text: await invoke("get_pacnew_diff", { path }) });
    } catch (e) {
      setDiff({ path, text: String(e) });
    }
  }
  return (
    <section className="panel">
      <h2>Before You Update</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={load} disabled={busy}>
          {busy ? "Asking the mirrors…" : brief ? "Check again" : "Check what an update would do"}
        </button>
        {brief && brief.count > 0 && brief.can_open_terminal && (
          <button className="ctl-btn" disabled={action.busy} onClick={() => action.run("open_update_terminal")}>
            Update in a terminal
          </button>
        )}
      </div>
      {brief && (
        <>
          <div className="finding">
            <strong>{brief.count === 0 ? "✓ Everything is up to date" : `${brief.count} package(s) waiting`}</strong>
            {brief.needs_reboot.length > 0 && (
              <span className="status-warn">
                ! Needs a restart of the machine afterwards: {brief.needs_reboot.slice(0, 6).join(", ")}
                {brief.needs_reboot.length > 6 ? ` and ${brief.needs_reboot.length - 6} more` : ""}
              </span>
            )}
            {brief.big.length > 0 && <span className="mem-muted">Close first, they are replaced while running: {brief.big.join(", ")}</span>}
          </div>
          {brief.news.length > 0 && (
            <div className="finding">
              <strong>Arch news</strong>
              {brief.news.map((n) => (
                <span key={n.link} className={n.manual ? "status-warn" : "mem-muted"}>
                  {n.manual ? "! " : ""}
                  {n.date} —{" "}
                  <button className="link-button" onClick={() => openUrl(n.link)}>
                    {n.title}
                  </button>
                </span>
              ))}
              <span className="mem-muted">Items marked ! say something must be done by hand. Read them before updating if they are recent.</span>
            </div>
          )}
          {brief.stale_apps.length > 0 && (
            <div className="finding">
              <strong>Still running the old version</strong>
              <span className="mem-muted">
                {brief.stale_apps.slice(0, 14).join(", ")}
                {brief.stale_apps.length > 14 ? ` and ${brief.stale_apps.length - 14} more` : ""} — restart them (or the machine) to
                finish the last update.
              </span>
            </div>
          )}
          {brief.pacnew.length > 0 && (
            <div className="finding">
              <strong>{brief.pacnew.length} settings file(s) have a newer packaged version</strong>
              {brief.pacnew.map((p) => (
                <div key={p} className="wifi-row">
                  <span className="wifi-name" title={p}>
                    {p.replace(/\.pacnew$/, "")}
                  </span>
                  <button className="chip chip-button" onClick={() => showDiff(p)}>
                    Compare
                  </button>
                  <ConfirmButton className="chip chip-button" label="Keep mine" confirmLabel="Sure?" disabled={action.busy} onConfirm={() => action.run("resolve_pacnew", { path: p, useNew: false })} />
                  <ConfirmButton className="chip chip-button" label="Use new" confirmLabel="Sure?" disabled={action.busy} onConfirm={() => action.run("resolve_pacnew", { path: p, useNew: true })} />
                </div>
              ))}
              {diff && (
                <>
                  <span className="mem-muted">{diff.path}: lines with − are yours, + are the packaged version.</span>
                  <pre className="log-box">{diff.text}</pre>
                </>
              )}
            </div>
          )}
        </>
      )}
      <StatusLine result={action.result} />
    </section>
  );
}

interface FileRow {
  path: string;
  bytes: number;
  modified: number;
}

interface FileScan {
  folder: string;
  files: number;
  truncated: boolean;
  largest: FileRow[];
  old: FileRow[];
  duplicates: { bytes: number; paths: string[] }[];
  duplicate_bytes: number;
}

/** Duplicates, the biggest files and files untouched for a year, in one folder. */
export function FileFinder() {
  const [scan, setScan] = useState<FileScan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState("duplicates");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  async function run(folder?: string | null) {
    const target = folder ?? (await pickFolder());
    if (!target) return;
    setBusy(true);
    setError(null);
    setPicked(new Set());
    try {
      setScan(await invoke("scan_files", { folder: target }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const action = useAction(() => scan && run(scan.folder));
  function toggle(path: string) {
    const next = new Set(picked);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setPicked(next);
  }
  const short = (p: string) => (scan ? p.replace(scan.folder + "/", "") : p);
  const row = (f: FileRow, extra?: string) => (
    <label key={f.path} className="file-row" title={f.path}>
      <input type="checkbox" checked={picked.has(f.path)} onChange={() => toggle(f.path)} />
      <span className="wifi-name">{short(f.path)}</span>
      <span className="mem-muted">
        {bytesToHuman(f.bytes)}
        {extra ?? ""}
      </span>
    </label>
  );
  return (
    <section className="panel wide">
      <h2>Duplicates & Forgotten Files</h2>
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" onClick={() => run()} disabled={busy}>
          {busy ? "Reading the folder…" : scan ? "Pick another folder…" : "Pick a folder to look through…"}
        </button>
        {scan && (
          <Segmented
            options={["duplicates", "largest", "old"]}
            value={tab}
            onChange={setTab}
          />
        )}
        {picked.size > 0 && (
          <ConfirmButton
            label={`Move ${picked.size} to the trash`}
            confirmLabel={`Trash ${picked.size} file(s)?`}
            disabled={action.busy}
            onConfirm={() => action.run("trash_files", { paths: [...picked] })}
          />
        )}
      </div>
      {error && <p className="status-fail">✗ {error}</p>}
      <StatusLine result={action.result} />
      {scan && (
        <>
          <p className="diagnose-hint">
            {scan.files.toLocaleString()} files in {scan.folder}
            {scan.truncated ? " (stopped at the limit — pick a smaller folder for a full answer)" : ""}.{" "}
            {tab === "duplicates" && `Identical copies waste ${bytesToHuman(scan.duplicate_bytes)}. Tick the copies to remove; leave one of each.`}
            {tab === "old" && "Over 10 MB and not changed in more than a year."}
          </p>
          <div className="event-scroll tall-scroll">
            {tab === "duplicates" &&
              (scan.duplicates.length === 0 ? (
                <p className="diagnose-hint">No identical files of 1 MB or more.</p>
              ) : (
                scan.duplicates.map((g, i) => (
                  <div key={i} className="finding">
                    <span className="mem-muted">
                      {g.paths.length} identical copies, {bytesToHuman(g.bytes)} each
                    </span>
                    {g.paths.map((p) => row({ path: p, bytes: g.bytes, modified: 0 }))}
                  </div>
                ))
              ))}
            {tab === "largest" && scan.largest.map((f) => row(f))}
            {tab === "old" && (scan.old.length === 0 ? <p className="diagnose-hint">Nothing that old and that big.</p> : scan.old.map((f) => row(f, ` · ${formatDay(f.modified)}`)))}
          </div>
        </>
      )}
      {!scan && (
        <p className="diagnose-hint">
          Hidden folders and build folders are skipped. Files go to the trash, never straight to deletion, so a mistake
          can be undone.
        </p>
      )}
    </section>
  );
}

interface BackupView {
  config: { sources: string[]; destination: string; weekly: boolean; last_ok: number };
  progress: { running: boolean; message: string };
  destination_connected: boolean;
  same_drive: boolean;
  snapshots: string[];
  folder: string;
}

/** Dated copies of chosen folders on another drive, with a check and restore. */
export function Backups() {
  const view = usePoll<BackupView>("get_backup", undefined, 4000);
  const { result, busy, run } = useAction(() => view.reload());
  const v = view.data;
  if (!v) return null;
  async function save(config: BackupView["config"]) {
    await run("save_backup", { config });
  }
  async function addSource() {
    const p = await pickFolder();
    if (p && !v!.config.sources.includes(p)) save({ ...v!.config, sources: [...v!.config.sources, p] });
  }
  async function setDestination() {
    const p = await pickFolder();
    if (p) save({ ...v!.config, destination: p });
  }
  const ready = v.config.sources.length > 0 && v.destination_connected;
  const pretty = (name: string) => `${name.slice(0, 10)} ${name.slice(11, 13)}:${name.slice(13, 15)}`;
  return (
    <section className="panel wide">
      <h2>Backups</h2>
      <div className="chip-row">
        <span className="mem-muted">Back up</span>
        {v.config.sources.map((s) => (
          <button key={s} className="chip chip-button" title="Click to stop backing this up" onClick={() => save({ ...v.config, sources: v.config.sources.filter((x) => x !== s) })}>
            {s} ×
          </button>
        ))}
        <button className="chip chip-button chip-add" onClick={addSource}>
          + folder
        </button>
      </div>
      <div className="chip-row">
        <span className="mem-muted">to</span>
        <button className="chip chip-button" onClick={setDestination}>
          {v.config.destination || "choose a destination…"}
        </button>
        {v.config.destination && !v.destination_connected && <span className="status-warn">not connected</span>}
        <label className="mem-muted">
          <input type="checkbox" checked={v.config.weekly} onChange={(e) => save({ ...v.config, weekly: e.target.checked })} /> by itself every week when
          the drive is connected
        </label>
      </div>
      {v.same_drive && (
        <p className="status-warn">! The destination is on the same drive as what it backs up. That protects against deleting a file, not against the drive failing.</p>
      )}
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" disabled={!ready || busy || v.progress.running} onClick={() => run("backup_action", { action: "start", name: "" })}>
          {v.progress.running ? v.progress.message : "Back up now"}
        </button>
        {v.config.last_ok > 0 && <span className="mem-muted">Last good backup: {formatDay(v.config.last_ok)}</span>}
      </div>
      {!v.progress.running && v.progress.message && <p className="diagnose-hint">{v.progress.message}</p>}
      <StatusLine result={result} />
      {v.snapshots.map((name) => (
        <div key={name} className="wifi-row">
          <span className="wifi-name">{pretty(name)}</span>
          <button className="chip chip-button" disabled={busy} onClick={() => run("backup_action", { action: "open", name })}>
            Open to restore
          </button>
          <button className="chip chip-button" disabled={busy || v.progress.running} onClick={() => run("backup_action", { action: "verify", name })}>
            Check it
          </button>
          <ConfirmButton className="chip chip-button" label="Delete" confirmLabel="Sure?" disabled={busy} onConfirm={() => run("backup_action", { action: "delete", name })} />
        </div>
      ))}
      <p className="diagnose-hint">
        Each backup is a complete dated copy you can open like any folder, but files that didn't change take no extra
        space. Build folders and caches are left out. “Check it” reads the whole backup back and compares it with your
        files. To restore, open a backup and copy out what you need.
      </p>
    </section>
  );
}

/** Apply a preset while a game runs and put everything back afterwards. */
export function GameModeCard() {
  const view = usePoll<{ mode: { enabled: boolean; preset: string }; running: string | null }>("get_game_mode", undefined, 15000);
  const presets = usePoll<[string, string][]>("get_preset_names");
  if (!view.data) return null;
  const { mode, running } = view.data;
  async function save(next: { enabled: boolean; preset: string }) {
    await invoke("set_game_mode", { mode: next });
    view.reload();
  }
  return (
    <section className="panel">
      <h2>Game Mode</h2>
      <div className="control-row">
        <label>
          When a game starts, apply
          <span className="mem-muted"> — and put every setting back when it closes</span>
        </label>
        <select className="port-input" value={mode.preset} onChange={(e) => save({ ...mode, preset: e.target.value })}>
          <option value="">choose a preset…</option>
          {(presets.data ?? []).map(([id, name]) => (
            <option key={id} value={id}>
              {name}
            </option>
          ))}
        </select>
        <Segmented options={["on", "off"]} value={mode.enabled ? "on" : "off"} disabled={!mode.preset} onChange={(v) => save({ ...mode, enabled: v === "on" })} />
      </div>
      <p className="diagnose-hint">
        {running ? `Running now: ${running}. ` : ""}
        Notices Steam and Proton games, Lutris and gamescope within a minute of starting. Parts of a preset that need
        your password are skipped unless the trusted helper (Tweaks) is on. A frame-rate overlay is not included:
        add <code>mangohud %command%</code> to the game's launch options for that.
      </p>
    </section>
  );
}

interface GroupUse {
  budget: { name: string; apps: string[]; max_gb: number; action: string };
  used_bytes: number;
}

/** One memory allowance shared by several apps. */
export function GroupBudgets({ apps }: { apps: string[] }) {
  const groups = usePoll<GroupUse[]>("get_group_budgets", undefined, 30000);
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<string[]>([]);
  const [gb, setGb] = useState("6");
  const [action, setAction] = useState("notify");
  if (!groups.data) return null;
  async function save(budgets: GroupUse["budget"][]) {
    await invoke("save_group_budgets", { budgets });
    groups.reload();
  }
  const current = groups.data.map((g) => g.budget);
  return (
    <section className="panel">
      <h2>Memory Allowance For A Group</h2>
      {groups.data.map((g) => {
        const share = Math.min(100, (g.used_bytes / (g.budget.max_gb * 1e9)) * 100);
        return (
          <div key={g.budget.name} className="bar-row">
            <span className="bar-label" title={g.budget.apps.join(", ")}>
              {g.budget.name}
            </span>
            <div className="bar-track">
              <div className={`bar-fill${share >= 100 ? " bar-over" : ""}`} style={{ width: `${share}%` }} />
            </div>
            <span className="bar-value" title={g.budget.action === "freeze" ? "Over the allowance, the heaviest app is frozen" : "Over the allowance, you get a notification"}>
              {(g.used_bytes / 1e9).toFixed(1)}/{g.budget.max_gb} GB
            </span>
            <button className="chip chip-button" title="Remove this group" onClick={() => save(current.filter((b) => b.name !== g.budget.name))}>
              ×
            </button>
          </div>
        );
      })}
      <div className="chip-row">
        {apps.slice(0, 14).map((a) => (
          <button key={a} className={`chip chip-button${chosen.includes(a) ? " chip-on" : ""}`} onClick={() => setChosen(chosen.includes(a) ? chosen.filter((c) => c !== a) : [...chosen, a])}>
            {a}
          </button>
        ))}
      </div>
      <div className="control-actions">
        <input className="port-input" placeholder="group name, e.g. Browsers" value={name} onChange={(e) => setName(e.target.value)} />
        <input className="port-input number-box" aria-label="Allowance in GB" value={gb} onChange={(e) => setGb(e.target.value.replace(/[^\d.]/g, ""))} />
        <span className="mem-muted">GB, then</span>
        <select className="port-input" value={action} onChange={(e) => setAction(e.target.value)}>
          <option value="notify">tell me</option>
          <option value="freeze">freeze the heaviest</option>
        </select>
        <button
          className="ctl-btn"
          disabled={!name.trim() || chosen.length === 0 || !(parseFloat(gb) > 0)}
          onClick={() => {
            save([...current.filter((b) => b.name !== name.trim()), { name: name.trim(), apps: chosen, max_gb: parseFloat(gb), action }]);
            setName("");
            setChosen([]);
          }}
        >
          Add group
        </button>
      </div>
      <p className="diagnose-hint">Checked once a minute. Over the allowance, Trace acts at most once every 15 minutes.</p>
    </section>
  );
}

interface Mixer {
  streams: { id: number; app: string; title: string; volume: number; muted: boolean; sink: string }[];
  outputs: { name: string; label: string }[];
  noise_suppression: boolean;
}

/** Volume and output per app, and a noise-filtered microphone. */
export function AppMixer() {
  const mixer = usePoll<Mixer>("get_mixer", undefined, 4000);
  const { result, busy, run } = useAction(() => mixer.reload());
  const [drag, setDrag] = useState<Record<number, number>>({});
  if (!mixer.data) return null;
  const m = mixer.data;
  async function set(id: number, action: string, value: string) {
    const r = await invoke<ActionResult>("set_stream", { id, action, value });
    if (r.ok) mixer.reload();
  }
  return (
    <section className="panel">
      <h2>Sound Per App</h2>
      {m.streams.length === 0 && <p className="diagnose-hint">Nothing is playing sound right now.</p>}
      {m.streams.map((s) => (
        <div key={s.id} className="mixer-row">
          <span className="wifi-name" title={s.title}>
            <strong>{s.app}</strong> <span className="mem-muted">{s.title}</span>
          </span>
          <input
            type="range"
            min={0}
            max={150}
            value={drag[s.id] ?? s.volume}
            aria-label={`Volume of ${s.app}`}
            onChange={(e) => setDrag({ ...drag, [s.id]: parseInt(e.target.value) })}
            onPointerUp={() => {
              if (drag[s.id] != null) set(s.id, "volume", String(drag[s.id]));
              setDrag(({ [s.id]: _, ...rest }) => rest);
            }}
          />
          <span className="bar-value">{drag[s.id] ?? s.volume}%</span>
          <button className={`chip chip-button${s.muted ? " chip-on" : ""}`} onClick={() => set(s.id, "mute", s.muted ? "0" : "1")}>
            {s.muted ? "Muted" : "Mute"}
          </button>
          {m.outputs.length > 1 && (
            <select className="port-input" value={s.sink} aria-label={`Where ${s.app} plays`} onChange={(e) => set(s.id, "move", e.target.value)}>
              {m.outputs.map((o) => (
                <option key={o.name} value={o.name}>
                  {o.label}
                </option>
              ))}
            </select>
          )}
        </div>
      ))}
      <div className="control-row">
        <label>
          Remove background noise from the microphone
          <span className="mem-muted"> — adds a filtered microphone and makes it the default</span>
        </label>
        <Segmented options={["on", "off"]} value={m.noise_suppression ? "on" : "off"} disabled={busy} onChange={(v) => run("set_noise_suppression", { on: v === "on" })} />
      </div>
      <StatusLine result={result} />
    </section>
  );
}

/** Lift the charge limit for one charge, then put it back. */
export function ChargeOnce() {
  const status = usePoll<{ limit: number | null; restore_to: number | null }>("get_charge_once", undefined, 30000);
  const { result, busy, run } = useAction(() => status.reload());
  const s = status.data;
  if (!s || s.limit == null) return null;
  if (s.restore_to == null && s.limit >= 100) return null;
  return (
    <>
      <div className="control-row">
        <label>
          {s.restore_to != null ? `Charging to 100% this once` : `Charging stops at ${s.limit}%`}
          <span className="mem-muted">
            {" "}
            — {s.restore_to != null ? `the ${s.restore_to}% limit returns when it is full or unplugged` : "need a full battery for a trip?"}
          </span>
        </label>
        {s.restore_to == null && (
          <button className="ctl-btn" disabled={busy} onClick={() => run("start_charge_once")}>
            Charge to full this once
          </button>
        )}
      </div>
      <StatusLine result={result} />
    </>
  );
}

interface MonitorColour {
  name: string;
  description: string;
  mode: string;
  saturation: number;
  brightness: number;
  ten_bit: boolean;
}

const COLOUR_MODES: [string, string][] = [
  ["srgb", "Standard (sRGB)"],
  ["wide", "Wide colour"],
  ["hdr", "HDR"],
  ["auto", "Automatic"],
];

function ColourRow({ monitor, onDone }: { monitor: MonitorColour; onDone: () => void }) {
  const [mode, setMode] = useState(COLOUR_MODES.some(([m]) => m === monitor.mode) ? monitor.mode : "srgb");
  const [saturation, setSaturation] = useState(monitor.saturation);
  const [brightness, setBrightness] = useState(monitor.brightness);
  const { result, busy, run } = useAction(onDone);
  const changed = mode !== monitor.mode || saturation !== monitor.saturation || brightness !== monitor.brightness;
  return (
    <div className="finding">
      <strong>
        {monitor.name} <span className="mem-muted">{monitor.description}</span>
      </strong>
      <div className="mixer-row">
        <select className="port-input" value={mode} onChange={(e) => setMode(e.target.value)} aria-label={`Colour mode of ${monitor.name}`}>
          {COLOUR_MODES.map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
        <span className="mem-muted">Saturation</span>
        <input type="range" min={0.5} max={1.5} step={0.05} value={saturation} onChange={(e) => setSaturation(parseFloat(e.target.value))} aria-label="Saturation" />
        <span className="bar-value">{saturation.toFixed(2)}</span>
        <span className="mem-muted">Brightness</span>
        <input type="range" min={0.5} max={2} step={0.05} value={brightness} onChange={(e) => setBrightness(parseFloat(e.target.value))} aria-label="Brightness of normal content" />
        <span className="bar-value">{brightness.toFixed(2)}</span>
        <button className="ctl-btn ctl-btn-sm" disabled={busy || !changed} onClick={() => run("set_monitor_colour", { name: monitor.name, mode, saturation, brightness })}>
          Apply
        </button>
        <button className="ctl-btn ctl-btn-sm" disabled={busy} onClick={() => run("set_monitor_colour", { name: monitor.name, mode: "srgb", saturation: 1, brightness: 1 })}>
          Reset
        </button>
      </div>
      <StatusLine result={result} />
    </div>
  );
}

/** Colour mode, saturation and brightness of normal content, per monitor. */
export function MonitorColours() {
  const { data, reload } = usePoll<MonitorColour[]>("get_monitor_colours");
  if (!data || data.length === 0) return null;
  return (
    <section className="panel wide">
      <h2>Colour Per Monitor</h2>
      {data.map((m) => (
        <ColourRow key={`${m.name}${m.mode}${m.saturation}${m.brightness}`} monitor={m} onDone={reload} />
      ))}
      <p className="diagnose-hint">
        Wide colour and HDR switch the monitor to 10-bit and only look right on a panel that supports them; if the
        picture goes wrong, press Reset. Saturation and brightness apply to normal (non-HDR) content in those modes.
        Changes last until you log out or reload Hyprland. Colour profile files (ICC) are not supported yet.
      </p>
    </section>
  );
}

/** Per-device switches shown under each USB device. */
export function UsbPower({ port, canWake, saving, onDone }: { port: string; canWake: boolean; saving: boolean; onDone: () => void }) {
  const { result, busy, run } = useAction(onDone);
  return (
    <span className="chip-row">
      <button className={`chip chip-button${canWake ? " chip-on" : ""}`} disabled={busy} title="May this device wake the laptop from sleep?" onClick={() => run("set_usb_power", { port, what: "wake", on: !canWake })}>
        Wake: {canWake ? "on" : "off"}
      </button>
      <button className={`chip chip-button${saving ? " chip-on" : ""}`} disabled={busy} title="May this device power down when idle?" onClick={() => run("set_usb_power", { port, what: "saving", on: !saving })}>
        Power saving: {saving ? "on" : "off"}
      </button>
      {result && !result.ok && <span className="status-fail">{result.message}</span>}
    </span>
  );
}

interface MiniSnapshot {
  cpu_usage_percent: number;
  total_memory_bytes: number;
  used_memory_bytes: number;
  temperatures: { celsius: number }[];
  networks: { rx_bytes_per_sec: number; tx_bytes_per_sec: number }[];
  pressure: { mem_some: number };
}

/** The small always-on-top window: vital signs and the heaviest apps. */
export function Mini() {
  const [snap, setSnap] = useState<MiniSnapshot | null>(null);
  const [apps, setApps] = useState<{ name: string; ram_bytes: number; swap_bytes: number }[]>([]);
  const [note, setNote] = useState<ActionResult | null>(null);
  async function refresh() {
    setSnap(await invoke("get_snapshot"));
    const memory = await invoke<{ apps: { name: string; ram_bytes: number; swap_bytes: number }[] }>("get_memory_breakdown");
    setApps(memory.apps.slice(0, 6));
  }
  useEffect(() => {
    document.body.classList.add("mini-body");
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, []);
  async function act(name: string, action: string) {
    setNote(await invoke("app_action", { name, action }));
    refresh();
  }
  if (!snap) return <p className="loading">…</p>;
  const ram = (snap.used_memory_bytes / snap.total_memory_bytes) * 100;
  const hot = Math.max(0, ...snap.temperatures.map((t) => t.celsius));
  const down = snap.networks.reduce((s, n) => s + n.rx_bytes_per_sec, 0);
  return (
    <div className="mini">
      <div className="mini-head" data-tauri-drag-region>
        <span data-tauri-drag-region>Trace</span>
        <button className="chip chip-button" onClick={() => import("@tauri-apps/api/window").then((w) => w.getCurrentWindow().close())}>
          ×
        </button>
      </div>
      <div className="mini-stats">
        <div>
          <strong className={snap.cpu_usage_percent > 85 ? "status-warn" : ""}>{snap.cpu_usage_percent.toFixed(0)}%</strong>
          <span>CPU</span>
        </div>
        <div>
          <strong className={ram > 90 ? "status-fail" : ram > 80 ? "status-warn" : ""}>{ram.toFixed(0)}%</strong>
          <span>RAM</span>
        </div>
        <div>
          <strong className={hot > 90 ? "status-fail" : ""}>{hot.toFixed(0)}°</strong>
          <span>Temp</span>
        </div>
        <div>
          <strong>{down > 1048576 ? `${(down / 1048576).toFixed(1)}M` : `${(down / 1024).toFixed(0)}K`}</strong>
          <span>Down/s</span>
        </div>
      </div>
      {snap.pressure.mem_some > 10 && <p className="status-fail">Memory is stalling the machine. Freeze or quit something below.</p>}
      {apps.map((a) => (
        <div key={a.name} className="mini-app">
          <span className="wifi-name">{a.name}</span>
          <span className="mem-muted">{bytesToHuman(a.ram_bytes + a.swap_bytes)}</span>
          <button className="chip chip-button" onClick={() => act(a.name, "freeze")}>
            Freeze
          </button>
          <ConfirmButton className="chip chip-button" label="Quit" confirmLabel="Sure?" onConfirm={() => act(a.name, "kill")} />
        </div>
      ))}
      {note && <p className={note.ok ? "status-ok" : "status-fail"}>{note.message}</p>}
    </div>
  );
}
