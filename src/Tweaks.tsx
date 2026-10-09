import { useContext, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { NavContext } from "./nav";
import { ConfirmButton, formatClock, formatDay, usePoll } from "./ui";

interface TweakState {
  id: string;
  category: string;
  title: string;
  help: string;
  kind: "toggle" | "choice" | "number";
  options: { value: string; label: string }[];
  min: number;
  max: number;
  step: number;
  unit: string;
  value: string;
  original: string;
  changed: boolean;
  needs_root: boolean;
  risk: "safe" | "careful" | "expert";
  persistence: "boot" | "session" | "inherent" | "never";
  kept: boolean;
  effect: string;
  detail: string;
}

interface PresetChange {
  id: string;
  title: string;
  value: string;
  value_label: string;
  current: string;
  differs: boolean;
  needs_root: boolean;
}

interface Preset {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  changes: PresetChange[];
  skipped: number;
}

interface Recommendation {
  id: string;
  title: string;
  value: string;
  value_label: string;
  current: string;
  reason: string;
  needs_root: boolean;
}

interface ChangeRow {
  ts: number;
  batch: number;
  tweak_id: string;
  title: string;
  old_value: string;
  new_value: string;
  ok: boolean;
  source: string;
}

interface ChangeResult {
  id: string;
  title: string;
  ok: boolean;
  message: string;
  old_value: string;
  new_value: string;
}

interface TweaksView {
  categories: { name: string; blurb: string }[];
  tweaks: TweakState[];
  presets: Preset[];
  recommendations: Recommendation[];
  changes: ChangeRow[];
  boot_service: "none" | "installed" | "stale" | "missing";
}

function labelOf(t: TweakState, value: string): string {
  if (t.kind === "toggle") return value;
  if (t.kind === "number") return `${value}${t.unit ? ` ${t.unit}` : ""}`;
  return t.options.find((o) => o.value === value)?.label ?? (value || "default");
}

function Switch({
  on,
  disabled,
  label,
  onChange,
}: {
  on: boolean;
  disabled?: boolean;
  label: string;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={on ? "switch on" : "switch"}
      disabled={disabled}
      onClick={() => onChange(!on)}
    >
      <span className="switch-knob" />
      <span className="switch-text">{on ? "On" : "Off"}</span>
    </button>
  );
}

/** Slider + number box. Reports a value only when you let go or press Enter. */
function NumberControl({
  t,
  value,
  disabled,
  onCommit,
}: {
  t: TweakState;
  value: string;
  disabled: boolean;
  onCommit: (v: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    const n = parseFloat(draft);
    if (!Number.isFinite(n)) return setDraft(value);
    const clamped = Math.min(t.max, Math.max(t.min, n));
    const text = String(Number.isInteger(t.step) ? Math.round(clamped) : +clamped.toFixed(2));
    setDraft(text);
    if (text !== value) onCommit(text);
  };
  return (
    <span className="number-control">
      <input
        type="range"
        min={t.min}
        max={t.max}
        step={t.step}
        value={parseFloat(draft) || 0}
        disabled={disabled}
        aria-label={t.title}
        onChange={(e) => setDraft(e.target.value)}
        onMouseUp={commit}
        onTouchEnd={commit}
        onKeyUp={commit}
      />
      <input
        className="port-input number-box"
        value={draft}
        disabled={disabled}
        aria-label={`${t.title} value`}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
      {t.unit && <span className="mem-muted">{t.unit}</span>}
    </span>
  );
}

function TweakRow({
  t,
  pending,
  busy,
  onChange,
  onKeep,
}: {
  t: TweakState;
  pending: string | undefined;
  busy: boolean;
  onChange: (t: TweakState, value: string) => void;
  onKeep: (t: TweakState, keep: boolean) => void;
}) {
  const value = pending ?? t.value;
  const staged = pending !== undefined && pending !== t.value;

  return (
    <div className={"tweak" + (staged ? " tweak-staged" : "")} id={`tweak-${t.id}`}>
      <div className="tweak-head">
        <span className="tweak-title">{t.title}</span>
        <span className="tweak-control">
          {t.kind === "toggle" && (
            <Switch
              on={value === "on"}
              disabled={busy}
              label={t.title}
              onChange={(on) => onChange(t, on ? "on" : "off")}
            />
          )}
          {t.kind === "choice" && (
            <select
              className="port-input tweak-select"
              value={value}
              disabled={busy}
              aria-label={t.title}
              onChange={(e) => onChange(t, e.target.value)}
            >
              {t.options.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          )}
          {t.kind === "number" && (
            <NumberControl t={t} value={value} disabled={busy} onCommit={(v) => onChange(t, v)} />
          )}
        </span>
      </div>
      <p className="tweak-help">{t.help}</p>
      <div className="tweak-meta">
        {staged && (
          <span className="badge badge-staged">
            waiting: {labelOf(t, t.value)} → {labelOf(t, value)}
          </span>
        )}
        {t.needs_root && <span className="badge" title="Changing this asks for your password">🔒 password</span>}
        {t.risk !== "safe" && (
          <span className={`badge badge-${t.risk}`}>{t.risk === "careful" ? "! noticeable" : "! expert"}</span>
        )}
        {t.effect && <span className="badge">takes effect {t.effect}</span>}
        {t.detail && <span className="mem-muted">{t.detail}</span>}
        {t.changed && !staged && (
          <span className="mem-muted">
            was {labelOf(t, t.original)} ·{" "}
            <button className="link-btn" disabled={busy} onClick={() => onChange(t, t.original)}>
              reset
            </button>
          </span>
        )}
        <span className="tweak-spacer" />
        {(t.persistence === "boot" || t.persistence === "session") && (
          <label
            className="check-row"
            title={
              t.persistence === "boot"
                ? "Re-applied by a small system service shortly after every boot (asks for your password to set up)"
                : "Re-applied whenever Trace starts"
            }
          >
            <input
              type="checkbox"
              checked={t.kept}
              disabled={busy}
              onChange={(e) => onKeep(t, e.target.checked)}
            />
            keep after restart
          </label>
        )}
        {t.persistence === "inherent" && <span className="mem-muted">stays after restart</span>}
        {t.persistence === "never" && <span className="mem-muted">resets at restart</span>}
      </div>
    </div>
  );
}

function PresetCard({
  preset,
  busy,
  onApply,
  onDelete,
}: {
  preset: Preset;
  busy: boolean;
  onApply: (p: Preset) => void;
  onDelete: (p: Preset) => void;
}) {
  const [open, setOpen] = useState(false);
  const differing = preset.changes.filter((c) => c.differs);
  const needsPassword = differing.some((c) => c.needs_root);
  return (
    <div className="preset">
      <div className="preset-name">{preset.name}</div>
      <p className="tweak-help">{preset.description}</p>
      <div className="tweak-meta">
        <span className="mem-muted">
          {differing.length === 0
            ? "already in effect"
            : `${differing.length} of ${preset.changes.length} settings would change`}
          {preset.skipped > 0 && ` · ${preset.skipped} not on this machine`}
        </span>
      </div>
      <div className="control-actions">
        <button
          className="ctl-btn ctl-primary"
          disabled={busy || differing.length === 0}
          onClick={() => onApply(preset)}
        >
          Apply{needsPassword ? " 🔒" : ""}
        </button>
        <button className="ctl-btn" onClick={() => setOpen(!open)}>
          {open ? "Hide details" : "What it changes"}
        </button>
        {!preset.builtin && (
          <ConfirmButton label="Delete" disabled={busy} onConfirm={() => onDelete(preset)} />
        )}
      </div>
      {open && (
        <div className="preset-changes">
          {preset.changes.map((c) => (
            <div key={c.id} className={c.differs ? "preset-change" : "preset-change mem-muted"}>
              <span>{c.title}</span>
              <span>{c.differs ? `${c.current} → ${c.value_label}` : `${c.value_label} ✓`}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default function Tweaks() {
  const { target } = useContext(NavContext);
  const { data, reload } = usePoll<TweaksView>("get_tweaks");
  const [pending, setPending] = useState<Record<string, string>>({});
  const [results, setResults] = useState<ChangeResult[] | null>(null);
  const [note, setNote] = useState<{ ok: boolean; message: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("All");
  const [showExpert, setShowExpert] = useState(false);
  const [onlyChanged, setOnlyChanged] = useState(false);
  const [presetName, setPresetName] = useState("");

  // Arriving from global search: show the setting that was searched for.
  useEffect(() => {
    if (target.page === "tweaks" && target.query) {
      setQuery(target.query);
      setCategory("All");
      setShowExpert(true);
    }
  }, [target]);

  const tweaks = data?.tweaks ?? [];
  const byId = useMemo(() => new Map(tweaks.map((t) => [t.id, t])), [tweaks]);

  async function apply(changes: { id: string; value: string }[], source: string) {
    if (changes.length === 0) return;
    setBusy(true);
    setNote(null);
    try {
      const r = await invoke<ChangeResult[]>("apply_tweaks", { changes, source, keep: null });
      setResults(r);
      setPending((p) => {
        const next = { ...p };
        for (const c of changes) delete next[c.id];
        return next;
      });
    } catch (e) {
      setNote({ ok: false, message: String(e) });
    } finally {
      setBusy(false);
      reload();
    }
  }

  // Settings that need a password are collected and applied together, so
  // several changes cost one prompt; the rest apply the moment you flip them.
  function change(t: TweakState, value: string) {
    if (t.needs_root) {
      setPending((p) => {
        const next = { ...p };
        if (value === t.value) delete next[t.id];
        else next[t.id] = value;
        return next;
      });
    } else {
      apply([{ id: t.id, value }], "manual");
    }
  }

  async function keep(t: TweakState, keepIt: boolean) {
    setBusy(true);
    try {
      setNote(await invoke("set_tweak_kept", { id: t.id, keep: keepIt }));
    } catch (e) {
      setNote({ ok: false, message: String(e) });
    } finally {
      setBusy(false);
      reload();
    }
  }

  async function undo(batch: number) {
    setBusy(true);
    try {
      setResults(await invoke<ChangeResult[]>("undo_tweak_batch", { batch }));
    } catch (e) {
      setNote({ ok: false, message: String(e) });
    } finally {
      setBusy(false);
      reload();
    }
  }

  async function presetAction(command: string, name: string) {
    setNote(await invoke(command, { name }));
    setPresetName("");
    reload();
  }

  if (!data) return <p className="loading">Reading this machine's settings…</p>;

  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const visible = tweaks.filter((t) => {
    if (category !== "All" && t.category !== category) return false;
    if (onlyChanged && !t.changed && !t.kept && pending[t.id] === undefined) return false;
    if (words.length > 0) {
      const text = `${t.title} ${t.help} ${t.id} ${t.category}`.toLowerCase();
      return words.every((w) => text.includes(w));
    }
    return showExpert || t.risk !== "expert" || t.changed;
  });
  const hiddenExpert = tweaks.filter((t) => t.risk === "expert" && !t.changed).length;
  const pendingList = Object.entries(pending).filter(([id]) => byId.has(id));
  const changedCount = tweaks.filter((t) => t.changed).length;
  const failed = (results ?? []).filter((r) => !r.ok);
  const succeeded = (results ?? []).filter((r) => r.ok);
  const browsing = words.length === 0 && !onlyChanged && category === "All";

  // Recent changes, grouped into the batches they were applied in.
  const batches: { batch: number; rows: ChangeRow[] }[] = [];
  for (const row of data.changes) {
    const last = batches[batches.length - 1];
    if (last && last.batch === row.batch) last.rows.push(row);
    else batches.push({ batch: row.batch, rows: [row] });
  }

  return (
    <>
      <div className="range-picker tweak-toolbar">
        <input
          className="port-input tweak-search"
          placeholder={`Search ${tweaks.length} settings…`}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <label className="check-row">
          <input type="checkbox" checked={onlyChanged} onChange={(e) => setOnlyChanged(e.target.checked)} />
          only what I changed ({changedCount})
        </label>
        <label className="check-row">
          <input type="checkbox" checked={showExpert} onChange={(e) => setShowExpert(e.target.checked)} />
          show expert settings ({hiddenExpert})
        </label>
      </div>
      <div className="range-picker section-picker">
        {["All", ...data.categories.map((c) => c.name)].map((name) => {
          const count = name === "All" ? tweaks.length : tweaks.filter((t) => t.category === name).length;
          if (count === 0) return null;
          return (
            <button
              key={name}
              className={name === category ? "range-btn active" : "range-btn"}
              onClick={() => setCategory(name)}
            >
              {name} <span className="mem-muted">{count}</span>
            </button>
          );
        })}
      </div>

      {note && <p className={note.ok ? "status-ok" : "status-fail"}>{note.ok ? "✓" : "✗"} {note.message}</p>}
      {results && (
        <section className="panel wide">
          <h2>
            Last change — {succeeded.length} applied{failed.length > 0 && `, ${failed.length} failed`}
          </h2>
          {failed.map((r) => (
            <p key={r.id} className="status-fail">
              ✗ {r.title}: {r.message}
            </p>
          ))}
          {succeeded.length > 0 && (
            <p className="status-ok">✓ {succeeded.map((r) => r.title).join(" · ")}</p>
          )}
          <div className="control-actions">
            <button className="ctl-btn" onClick={() => setResults(null)}>
              Dismiss
            </button>
          </div>
        </section>
      )}

      {browsing && data.recommendations.length > 0 && (
        <section className="panel wide">
          <h2>Suggested for This Machine ({data.recommendations.length})</h2>
          {data.recommendations.map((r) => (
            <div key={r.id} className="issue-row suggestion">
              <button
                className="ctl-btn"
                disabled={busy}
                onClick={() => apply([{ id: r.id, value: r.value }], "suggestion")}
              >
                Apply{r.needs_root ? " 🔒" : ""}
              </button>
              <div>
                <div className="issue-title">
                  {r.title}: {r.current} → {r.value_label}
                </div>
                <div className="issue-detail">{r.reason}</div>
              </div>
            </div>
          ))}
          <div className="control-actions">
            <button
              className="ctl-btn ctl-primary"
              disabled={busy}
              onClick={() =>
                apply(
                  data.recommendations.map((r) => ({ id: r.id, value: r.value })),
                  "suggestion"
                )
              }
            >
              Apply all {data.recommendations.length} 🔒
            </button>
          </div>
          <p className="diagnose-hint">
            Worked out from this machine's own state, not a generic checklist. Each one can be
            undone from Recent changes below.
          </p>
        </section>
      )}

      {browsing && (
        <section className="panel wide">
          <h2>Presets</h2>
          <div className="preset-grid">
            {data.presets.map((p) => (
              <PresetCard
                key={p.id}
                preset={p}
                busy={busy}
                onApply={(preset) =>
                  apply(
                    preset.changes.filter((c) => c.differs).map((c) => ({ id: c.id, value: c.value })),
                    `preset:${preset.name}`
                  )
                }
                onDelete={(preset) => presetAction("delete_tweak_preset", preset.name)}
              />
            ))}
          </div>
          <div className="control-row">
            <label>
              Save everything you have changed ({changedCount} setting{changedCount === 1 ? "" : "s"}) as
              your own preset
            </label>
            <input
              className="port-input"
              placeholder="Name, e.g. On the train"
              value={presetName}
              onChange={(e) => setPresetName(e.target.value)}
            />
            <button
              className="ctl-btn"
              disabled={busy || !presetName.trim() || changedCount === 0}
              onClick={() => presetAction("save_tweak_preset", presetName)}
            >
              Save preset
            </button>
          </div>
          <p className="diagnose-hint">
            A preset changes several settings at once with a single password prompt. Presets can also
            be run on a schedule or when an app starts: add an "Apply settings preset" step to a
            profile under Control → Automation.
          </p>
        </section>
      )}

      {data.categories.map((c) => {
        const rows = visible.filter((t) => t.category === c.name);
        if (rows.length === 0) return null;
        return (
          <section key={c.name} className="panel wide">
            <h2>
              {c.name} <span className="mem-muted tweak-count">{rows.length}</span>
            </h2>
            <p className="diagnose-hint tweak-blurb">{c.blurb}</p>
            <div className="tweak-grid">
              {rows.map((t) => (
                <TweakRow
                  key={t.id}
                  t={t}
                  pending={pending[t.id]}
                  busy={busy}
                  onChange={change}
                  onKeep={keep}
                />
              ))}
            </div>
          </section>
        );
      })}
      {visible.length === 0 && (
        <p className="diagnose-hint">
          No setting matches. Settings that this machine doesn't have (other hardware, other desktops)
          aren't listed.
        </p>
      )}

      {browsing && batches.length > 0 && (
        <section className="panel wide">
          <h2>Recent Changes</h2>
          {batches.slice(0, 8).map((b) => (
            <div key={b.batch} className="change-batch">
              <div className="change-head">
                <span className="event-time">
                  {formatDay(b.batch)} {formatClock(b.batch)}
                </span>
                <span className="mem-muted">{b.rows[0].source.replace("preset:", "preset “") + (b.rows[0].source.startsWith("preset:") ? "”" : "")}</span>
                <span className="tweak-spacer" />
                {b.rows.some((r) => r.ok) && b.rows[0].source !== "undo" && (
                  <button className="ctl-btn-sm" disabled={busy} onClick={() => undo(b.batch)}>
                    Undo
                  </button>
                )}
              </div>
              {b.rows.map((r, i) => (
                <div key={i} className="preset-change">
                  <span>
                    <span className={r.ok ? "status-ok" : "status-fail"}>{r.ok ? "✓" : "✗"}</span> {r.title}
                  </span>
                  <span className="mem-muted">
                    {byId.get(r.tweak_id) ? labelOf(byId.get(r.tweak_id)!, r.old_value) : r.old_value} →{" "}
                    {byId.get(r.tweak_id) ? labelOf(byId.get(r.tweak_id)!, r.new_value) : r.new_value}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </section>
      )}

      {browsing && (
        <p className="diagnose-hint">
          {data.boot_service === "installed" &&
            "Settings marked \"keep after restart\" are re-applied about 15 seconds after boot by a small system service (/etc/trace/tweaks.sh). Unticking the last one removes it. "}
          {(data.boot_service === "stale" || data.boot_service === "missing") &&
            "The service that re-applies kept settings at boot is out of date — tick or untick any \"keep after restart\" box to rewrite it. "}
          Trace deliberately leaves out anything that can't be undone or could stop the machine
          booting: the bootloader, kernel command line, disk layout, sudo rules and one-way kernel
          lock-downs.
        </p>
      )}

      {pendingList.length > 0 && (
        <div className="pending-bar">
          <span>
            <strong>{pendingList.length}</strong> change{pendingList.length === 1 ? "" : "s"} waiting:{" "}
            <span className="mem-muted">
              {pendingList
                .slice(0, 3)
                .map(([id]) => byId.get(id)!.title)
                .join(", ")}
              {pendingList.length > 3 && ` +${pendingList.length - 3} more`}
            </span>
          </span>
          <span className="tweak-spacer" />
          <button className="ctl-btn" disabled={busy} onClick={() => setPending({})}>
            Discard
          </button>
          <button
            className="ctl-btn ctl-primary"
            disabled={busy}
            onClick={() =>
              apply(
                pendingList.map(([id, value]) => ({ id, value })),
                "manual"
              )
            }
          >
            {busy ? "Waiting for your password…" : "Apply — one password prompt"}
          </button>
        </div>
      )}
    </>
  );
}
