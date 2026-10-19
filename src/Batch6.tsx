import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ActionResult, ConfirmButton, Segmented, Sparkline, StatusLine, formatClock, formatDay, useAction, usePoll } from "./ui";
import { NtfySetting } from "./Batch7";

interface ScreenDay {
  day_start: number;
  total_secs: number;
  hours: { hour: number; secs: number; top_app: string; apps: [string, number][] }[];
}

function span(secs: number): string {
  return secs >= 3600 ? `${Math.floor(secs / 3600)} h ${Math.floor((secs % 3600) / 60)} min` : `${Math.round(secs / 60)} min`;
}

/** What was in front of you during each hour of a day. */
export function ScreenStrip() {
  const [offset, setOffset] = useState(0);
  const { data } = usePoll<ScreenDay>("get_screen_day", { dayOffset: offset }, 60000);
  const [picked, setPicked] = useState<number | null>(null);
  if (!data) return null;
  const hour = picked != null ? data.hours[picked] : null;
  return (
    <section className="panel wide">
      <h2>
        Screen Time, Hour By Hour{" "}
        <button className="chip chip-button" onClick={() => setOffset(offset - 1)}>
          ‹
        </button>{" "}
        <span className="mem-muted">{offset === 0 ? "today" : offset === -1 ? "yesterday" : formatDay(data.day_start)}</span>{" "}
        <button className="chip chip-button" disabled={offset >= 0} onClick={() => setOffset(offset + 1)}>
          ›
        </button>
      </h2>
      <div className="hour-strip">
        {data.hours.map((h) => (
          <button
            key={h.hour}
            className={`hour-cell${picked === h.hour ? " hour-picked" : ""}`}
            title={h.secs ? `${h.hour}:00 — ${span(h.secs)}, mostly ${h.top_app}` : `${h.hour}:00 — nothing`}
            onClick={() => setPicked(picked === h.hour ? null : h.hour)}
          >
            <span className="hour-fill" style={{ height: `${(h.secs / 3600) * 100}%` }} />
            <span className="hour-label">{h.hour % 3 === 0 ? h.hour : ""}</span>
          </button>
        ))}
      </div>
      <p className="diagnose-hint">
        {data.total_secs === 0 ? "Nothing recorded for this day." : `${span(data.total_secs)} at the screen.`}{" "}
        {hour &&
          (hour.secs
            ? `${hour.hour}:00–${hour.hour + 1}:00: ${hour.apps.map(([app, secs]) => `${app} ${span(secs)}`).join(", ")}.`
            : `${hour.hour}:00: away or switched off.`)}{" "}
        Time only counts while you are actually there: five minutes without touching anything stops the clock, unless
        sound is playing.
      </p>
    </section>
  );
}

/** When the microphone or camera was last used, and by what. */
export function MediaHistory() {
  const { data } = usePoll<{ ts: number; kind: string; app: string }[]>("get_media_history", undefined, 60000);
  if (!data) return null;
  return (
    <>
      <h3 className="sub-heading">Recent use</h3>
      {data.length === 0 && <p className="diagnose-hint">Nothing recorded yet. Trace notes each time an app starts using either one.</p>}
      <div className="event-scroll">
        {data.map((e, i) => (
          <div key={i} className="plain-row">
            <span className="mem-muted">
              {formatDay(e.ts)} {formatClock(e.ts)}
            </span>{" "}
            {e.app} used the {e.kind}
          </div>
        ))}
      </div>
    </>
  );
}

interface Vulnerable {
  package: string;
  installed: string;
  fixed: string;
  severity: string;
  advisory: string;
}

/** Installed packages the Arch security team lists as vulnerable. */
export function VulnerablePackages() {
  const [list, setList] = useState<Vulnerable[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    setError(null);
    try {
      setList(await invoke("get_vulnerable_packages"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const fixable = list?.filter((v) => v.fixed).length ?? 0;
  return (
    <div className="finding">
      <div className="control-actions">
        <button className="ctl-btn" onClick={run} disabled={busy}>
          {busy ? "Asking the security tracker…" : "Check installed packages for known vulnerabilities"}
        </button>
      </div>
      {error && <span className="status-fail">✗ {error}</span>}
      {list && list.length === 0 && <strong className="status-ok">✓ No installed package is listed as vulnerable</strong>}
      {list && list.length > 0 && (
        <>
          <strong className="status-warn">
            ! {list.length} installed package(s) have a known vulnerability{fixable ? `; updating fixes ${fixable} of them` : ""}
          </strong>
          {list.slice(0, 12).map((v) => (
            <span key={v.package} className="mem-muted">
              {v.package} {v.installed} — {v.severity.toLowerCase()} ({v.advisory}){v.fixed ? `, fixed in ${v.fixed}` : ", no fix released yet"}
            </span>
          ))}
        </>
      )}
    </div>
  );
}

interface Extras {
  phone_alerts: boolean;
  ntfy_url: string;
  kbd_timeout_secs: number;
  weekly_digest: boolean;
  break_minutes: number;
  app_limits: { app: string; minutes: number }[];
  digest_week: number;
  setup_done: boolean;
}

interface ExtrasView {
  extras: Extras;
  phones: { id: string; name: string }[];
  has_kbd_light: boolean;
}

/** Small optional behaviours: phone alerts, keyboard light, digest, breaks, app time. */
export function Reminders({ apps }: { apps: string[] }) {
  const view = usePoll<ExtrasView>("get_extras", undefined, 60000);
  const test = useAction();
  const [app, setApp] = useState("");
  const [minutes, setMinutes] = useState("60");
  if (!view.data) return null;
  const { extras, phones, has_kbd_light } = view.data;
  async function save(next: Partial<Extras>) {
    await invoke("save_extras", { extras: { ...extras, ...next } });
    view.reload();
  }
  const onOff = (value: boolean, change: (on: boolean) => void, disabled = false) => (
    <Segmented options={["on", "off"]} value={value ? "on" : "off"} disabled={disabled} onChange={(v) => change(v === "on")} />
  );
  return (
    <section className="panel">
      <h2>Reminders & Small Things</h2>
      <div className="control-row">
        <label>
          Monday summary of last week
          <span className="mem-muted"> — hours, most-used apps, crashes, in one notification</span>
        </label>
        {onOff(extras.weekly_digest, (on) => save({ weekly_digest: on }))}
      </div>
      <div className="control-row">
        <label>
          Suggest a break
          <span className="mem-muted"> — after this long at the screen without five minutes away</span>
        </label>
        <select className="port-input" value={extras.break_minutes} onChange={(e) => save({ break_minutes: parseInt(e.target.value) })}>
          <option value={0}>never</option>
          {[30, 45, 60, 90, 120].map((m) => (
            <option key={m} value={m}>
              after {m} min
            </option>
          ))}
        </select>
      </div>
      {has_kbd_light && (
        <div className="control-row">
          <label>
            Keyboard light off when idle
            <span className="mem-muted"> — comes back at the next key press</span>
          </label>
          <select className="port-input" value={extras.kbd_timeout_secs} onChange={(e) => save({ kbd_timeout_secs: parseInt(e.target.value) })}>
            <option value={0}>never</option>
            {[10, 30, 60, 300].map((s) => (
              <option key={s} value={s}>
                after {s < 60 ? `${s} s` : `${s / 60} min`}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="control-row">
        <label>
          Alert my phone while I'm away
          <span className="mem-muted">
            {" "}
            — a USB device plugged in or a wrong password, sent through KDE Connect{" "}
            {phones.length ? `to ${phones.map((p) => p.name).join(", ")}` : "(no paired phone is reachable right now)"}
          </span>
        </label>
        <button className="ctl-btn ctl-btn-sm" disabled={test.busy || phones.length === 0} onClick={() => test.run("test_phone_alert")}>
          Test
        </button>
        {onOff(extras.phone_alerts, (on) => save({ phone_alerts: on }))}
      </div>
      <StatusLine result={test.result} />
      <NtfySetting value={extras.ntfy_url ?? ""} onSave={(ntfy_url) => save({ ntfy_url })} />

      <h3 className="sub-heading">Daily time per app</h3>
      {extras.app_limits.map((l) => (
        <div key={l.app} className="wifi-row">
          <span className="wifi-name">
            <strong>{l.app}</strong> <span className="mem-muted">a nudge after {l.minutes} min in front each day</span>
          </span>
          <button className="chip chip-button" onClick={() => save({ app_limits: extras.app_limits.filter((x) => x.app !== l.app) })}>
            ×
          </button>
        </div>
      ))}
      <div className="control-actions">
        <input className="port-input" list="limit-apps" placeholder="app, e.g. brave" value={app} onChange={(e) => setApp(e.target.value)} />
        <datalist id="limit-apps">
          {apps.map((a) => (
            <option key={a} value={a} />
          ))}
        </datalist>
        <input className="port-input number-box" aria-label="Minutes per day" value={minutes} onChange={(e) => setMinutes(e.target.value.replace(/\D/g, ""))} />
        <span className="mem-muted">min</span>
        <button
          className="ctl-btn"
          disabled={!app.trim() || !(parseInt(minutes) > 0)}
          onClick={() => {
            save({ app_limits: [...extras.app_limits.filter((x) => x.app !== app.trim()), { app: app.trim(), minutes: parseInt(minutes) }] });
            setApp("");
          }}
        >
          Add
        </button>
      </div>
      <p className="diagnose-hint">These only tell you; nothing is closed or blocked. Phone and ntfy alerts are the only things here that leave this machine.</p>
    </section>
  );
}

/** Signal strength of the Wi-Fi link over the last day. */
export function WifiSignal() {
  const { data } = usePoll<[number, number][]>("get_wifi_signal", undefined, 60000);
  if (!data || data.length < 3) return null;
  const values = data.map(([, dbm]) => dbm);
  const last = values[values.length - 1];
  const worst = Math.min(...values);
  // −90 dBm is unusable, −30 is next to the router.
  const quality = (dbm: number) => Math.max(1, Math.min(100, ((dbm + 90) / 60) * 100));
  const word = last >= -55 ? "strong" : last >= -67 ? "good" : last >= -75 ? "weak" : "very weak";
  return (
    <section className="panel">
      <h2>Wi-Fi Signal, Last 24 Hours</h2>
      <Sparkline values={values.map(quality)} color="#6ad98c" width={320} height={48} />
      <p className="diagnose-hint">
        Now {last} dBm ({word}); weakest {worst} dBm. Dips that line up with slow moments mean the problem is distance
        or walls, not your provider.
      </p>
    </section>
  );
}

/** A tone preset every app's sound is routed through. */
export function Equaliser({ onChange }: { onChange: () => void }) {
  const view = usePoll<{ current: string; presets: [string, string][] }>("get_equaliser", undefined, 30000);
  const { result, busy, run } = useAction(() => {
    view.reload();
    onChange();
  });
  if (!view.data) return null;
  return (
    <>
      <div className="control-row">
        <label>
          Equaliser
          <span className="mem-muted"> — one tone preset for everything that plays</span>
        </label>
        <select className="port-input" value={view.data.current} disabled={busy} onChange={(e) => run("set_equaliser", { preset: e.target.value })}>
          <option value="">off</option>
          {view.data.presets.map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
      </div>
      <StatusLine result={result} />
    </>
  );
}

/** A weekly root job so maintenance needs the password once, not every week. */
export function AutoMaintenance() {
  const status = usePoll<{ enabled: boolean; last_lines: string[] }>("get_auto_maintenance", undefined, 120000);
  const { result, busy, run } = useAction(() => status.reload());
  if (!status.data) return null;
  return (
    <>
      <div className="control-row">
        <label>
          Run it by itself every week
          <span className="mem-muted"> — while on the charger; asks for your password once to set up</span>
        </label>
        <Segmented options={["on", "off"]} value={status.data.enabled ? "on" : "off"} disabled={busy} onChange={(v) => run("set_auto_maintenance", { on: v === "on" })} />
      </div>
      <StatusLine result={result} />
      {status.data.last_lines.length > 0 && (
        <div className="plain-row">
          <span className="mem-muted">Last automatic run {status.data.last_lines[0]}:</span> {status.data.last_lines.slice(1).join(" · ")}
        </div>
      )}
    </>
  );
}

/** Wipe an unused solid-state drive completely. Guarded several times over. */
export function SecureErase() {
  const drives = usePoll<{ name: string; size: string; model: string; blocked: string }[]>("get_erasable_drives", undefined, 15000);
  const [target, setTarget] = useState<string | null>(null);
  const [typed, setTyped] = useState("");
  const { result, busy, run } = useAction(() => {
    drives.reload();
    setTarget(null);
    setTyped("");
  });
  if (!drives.data) return null;
  return (
    <section className="panel">
      <h2>Erase A Drive</h2>
      {drives.data.map((d) => (
        <div key={d.name} className="finding">
          <div className="control-row">
            <label>
              <strong>{d.name}</strong>{" "}
              <span className="mem-muted">
                {d.model} · {d.size}
              </span>
            </label>
            {!d.blocked && target !== d.name && (
              <button className="ctl-btn ctl-danger" onClick={() => setTarget(d.name)}>
                Erase…
              </button>
            )}
          </div>
          {d.blocked && <span className="mem-muted">{d.blocked}</span>}
          {target === d.name && (
            <div className="control-actions">
              <input className="port-input" placeholder={`type ${d.name} to confirm`} value={typed} onChange={(e) => setTyped(e.target.value)} />
              <ConfirmButton
                label="Erase everything on it"
                confirmLabel="This cannot be undone — erase?"
                disabled={busy || typed.trim() !== d.name}
                onConfirm={() => run("secure_erase", { device: d.name, typed })}
              />
              <button className="ctl-btn" onClick={() => setTarget(null)}>
                Cancel
              </button>
            </div>
          )}
        </div>
      ))}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        For a drive you are giving away or reusing. Only a solid-state drive with nothing mounted can be erased; the
        drive this system runs from is always refused. Nothing can bring the data back afterwards.
      </p>
    </section>
  );
}

const SHADER = `precision highp float;uniform float t;uniform vec2 r;
void main(){vec2 p=(gl_FragCoord.xy/r-0.5)*3.0;float c=0.0;
for(int i=0;i<220;i++){float a=float(i)*0.031+t;p=vec2(p.x*cos(a)-p.y*sin(a),p.x*sin(a)+p.y*cos(a));p=abs(p)/dot(p,p)-0.9;c+=exp(-9.0*abs(dot(p,p)-0.4));}
gl_FragColor=vec4(c*0.012,c*0.02,c*0.03,1.0);}`;

/** A deliberately expensive picture, to load the graphics chip during a cooling test. */
export function GpuLoad() {
  const ref = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const canvas = ref.current;
    const gl = canvas?.getContext("webgl");
    if (!canvas || !gl) {
      setFailed(true);
      return;
    }
    const compile = (type: number, source: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, source);
      gl.compileShader(s);
      return s;
    };
    const program = gl.createProgram()!;
    gl.attachShader(program, compile(gl.VERTEX_SHADER, "attribute vec2 a;void main(){gl_Position=vec4(a,0.0,1.0);}"));
    gl.attachShader(program, compile(gl.FRAGMENT_SHADER, SHADER));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      setFailed(true);
      return;
    }
    gl.useProgram(program);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.bindAttribLocation(program, 0, "a");
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.uniform2f(gl.getUniformLocation(program, "r"), canvas.width, canvas.height);
    const time = gl.getUniformLocation(program, "t");
    let frame = 0;
    const draw = (now: number) => {
      gl.uniform1f(time, now / 4000);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, []);
  if (failed) return <p className="diagnose-hint">This window can't draw 3D, so only the processor is being loaded.</p>;
  return <canvas ref={ref} className="gpu-load" width={1280} height={720} aria-label="Graphics load" />;
}

const SETUP_QUESTIONS: { id: string; question: string; yes: string; preset: string }[] = [
  { id: "battery", question: "Is it mostly used away from the charger?", yes: "Favour battery life", preset: "battery" },
  { id: "memory", question: "Does it often run out of memory with many apps open?", yes: "Use compressed memory", preset: "zram" },
  { id: "developer", question: "Do you write code on it?", yes: "Raise the limits development tools hit", preset: "developer" },
  { id: "gaming", question: "Do you play games on it?", yes: "Switch to the Gaming preset while a game runs", preset: "gaming" },
];

/** A short first-run set-up: four questions, then the matching presets. */
export function SetupCard() {
  const view = usePoll<ExtrasView>("get_extras");
  const [answers, setAnswers] = useState<Record<string, boolean>>({});
  const [results, setResults] = useState<ActionResult[]>([]);
  const [busy, setBusy] = useState(false);
  if (!view.data || view.data.extras.setup_done) return null;
  async function finish(apply: boolean) {
    setBusy(true);
    const done: ActionResult[] = [];
    if (apply) {
      for (const q of SETUP_QUESTIONS.filter((q) => answers[q.id])) {
        if (q.id === "gaming") {
          await invoke("set_game_mode", { mode: { enabled: true, preset: "gaming", per_game: {}, seen: [] } });
          done.push({ ok: true, message: "Game mode switched on" });
        } else {
          done.push(await invoke<ActionResult>("apply_preset_by_id", { id: q.preset }));
        }
      }
    }
    setResults(done);
    await invoke("save_extras", { extras: { ...view.data!.extras, setup_done: true } });
    setBusy(false);
    if (done.length === 0) view.reload();
  }
  if (results.length > 0) {
    return (
      <section className="panel wide">
        <h2>Set-up Done</h2>
        {results.map((r, i) => (
          <StatusLine key={i} result={r} />
        ))}
        <div className="control-actions">
          <button className="ctl-btn" onClick={() => view.reload()}>
            Close
          </button>
        </div>
      </section>
    );
  }
  const chosen = SETUP_QUESTIONS.filter((q) => answers[q.id]).length;
  return (
    <section className="panel wide">
      <h2>Quick Set-up</h2>
      {SETUP_QUESTIONS.map((q) => (
        <div key={q.id} className="control-row">
          <label>
            {q.question}
            <span className="mem-muted"> — yes: {q.yes.toLowerCase()}</span>
          </label>
          <Segmented options={["yes", "no"]} value={answers[q.id] ? "yes" : "no"} onChange={(v) => setAnswers({ ...answers, [q.id]: v === "yes" })} />
        </div>
      ))}
      <div className="control-actions">
        <button className="ctl-btn ctl-primary" disabled={busy || chosen === 0} onClick={() => finish(true)}>
          {busy ? "Applying…" : `Apply ${chosen || ""} choice${chosen === 1 ? "" : "s"}`}
        </button>
        <button className="ctl-btn" disabled={busy} onClick={() => finish(false)}>
          Not now
        </button>
      </div>
      <p className="diagnose-hint">
        Each choice applies a preset from the Tweaks page, where every setting it changed is listed and can be undone.
        Some ask for your password.
      </p>
    </section>
  );
}

export interface CardPrefs {
  order: string[];
  hidden: string[];
}

const PREFS_KEY = "trace-overview-cards";

export function loadCardPrefs(ids: string[]): CardPrefs {
  try {
    const saved = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<CardPrefs>;
    const order = [...(saved.order ?? []).filter((id) => ids.includes(id)), ...ids.filter((id) => !(saved.order ?? []).includes(id))];
    return { order, hidden: (saved.hidden ?? []).filter((id) => ids.includes(id)) };
  } catch {
    return { order: ids, hidden: [] };
  }
}

/** Choose which Overview cards show, and in what order. */
export function CardChooser({ prefs, labels, onChange }: { prefs: CardPrefs; labels: Record<string, string>; onChange: (p: CardPrefs) => void }) {
  const [open, setOpen] = useState(false);
  function save(next: CardPrefs) {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(next));
    } catch {
      // Not being able to remember the choice is no reason to refuse it.
    }
    onChange(next);
  }
  function move(id: string, by: number) {
    const order = [...prefs.order];
    const i = order.indexOf(id);
    const j = i + by;
    if (j < 0 || j >= order.length) return;
    [order[i], order[j]] = [order[j], order[i]];
    save({ ...prefs, order });
  }
  if (!open) {
    return (
      <div className="card-chooser" style={{ order: 999 }}>
        <button className="chip chip-button" onClick={() => setOpen(true)}>
          Choose and arrange these cards…
        </button>
      </div>
    );
  }
  return (
    <section className="panel wide" style={{ order: 999 }}>
      <h2>
        Cards On This Page{" "}
        <button className="chip chip-button" onClick={() => setOpen(false)}>
          Done
        </button>
      </h2>
      {prefs.order.map((id, i) => {
        const hidden = prefs.hidden.includes(id);
        return (
          <div key={id} className="wifi-row">
            <span className={`wifi-name${hidden ? " mem-muted" : ""}`}>{labels[id]}</span>
            <button className="chip chip-button" disabled={i === 0} onClick={() => move(id, -1)} aria-label={`Move ${labels[id]} earlier`}>
              ↑
            </button>
            <button className="chip chip-button" disabled={i === prefs.order.length - 1} onClick={() => move(id, 1)} aria-label={`Move ${labels[id]} later`}>
              ↓
            </button>
            <button
              className={`chip chip-button${hidden ? "" : " chip-on"}`}
              onClick={() => save({ ...prefs, hidden: hidden ? prefs.hidden.filter((h) => h !== id) : [...prefs.hidden, id] })}
            >
              {hidden ? "Hidden" : "Shown"}
            </button>
          </div>
        );
      })}
    </section>
  );
}
