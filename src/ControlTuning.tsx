import { useState } from "react";
import {
  ConfirmButton,
  Segmented,
  StatusLine,
  bytesToHuman,
  useAction,
  usePoll,
} from "./ui";

interface MemoryTuning {
  swappiness: number;
  swap_used_bytes: number;
  available_bytes: number;
  can_flush_swap: boolean;
}

interface CpuPower {
  governor: string | null;
  available_governors: string[];
  energy_preference: string | null;
  available_energy_preferences: string[];
  power_profile: string | null;
  available_power_profiles: string[];
  platform_profile: string | null;
  available_platform_profiles: string[];
  cores: { index: number; online: boolean; can_toggle: boolean }[];
}

interface Fan {
  chip: string;
  label: string;
  rpm: number;
  pwm_path: string | null;
  pwm_percent: number | null;
  automatic: boolean | null;
}

export function MemoryTuningControl() {
  const { data, reload } = usePoll<MemoryTuning>("get_memory_tuning", undefined, 10000);
  const { result, busy, run } = useAction(reload);
  const [swappiness, setSwappiness] = useState<number | null>(null);
  if (!data) return null;
  const value = swappiness ?? data.swappiness;

  return (
    <section className="panel">
      <h2>Memory &amp; Swap Tuning</h2>
      <div className="control-row">
        <label>
          Swappiness ({value}) — lower keeps apps in RAM longer, higher swaps
          sooner
        </label>
        <input
          type="range"
          min={0}
          max={200}
          value={value}
          onChange={(e) => setSwappiness(parseInt(e.target.value, 10))}
        />
        <button
          className="ctl-btn"
          disabled={busy || value === data.swappiness}
          onClick={() => run("set_swappiness", { value })}
        >
          Apply
        </button>
      </div>
      <div className="control-actions">
        <ConfirmButton
          label={`Pull ${bytesToHuman(data.swap_used_bytes)} back out of swap`}
          className="ctl-btn"
          disabled={busy || !data.can_flush_swap}
          onConfirm={() => run("flush_swap")}
        />
        <button className="ctl-btn" disabled={busy} onClick={() => run("drop_caches")}>
          Drop file cache
        </button>
        <button className="ctl-btn" disabled={busy} onClick={() => run("compact_memory")}>
          Compact memory
        </button>
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        {data.swap_used_bytes > 0 && !data.can_flush_swap
          ? `Emptying swap is disabled: ${bytesToHuman(data.swap_used_bytes)} is swapped but only ${bytesToHuman(data.available_bytes)} of RAM is available, so it would not fit. `
          : ""}
        Each of these asks for your password. Swappiness resets at reboot.
        Dropping the file cache frees nothing apps could not already use — it
        is mainly useful before a benchmark.
      </p>
    </section>
  );
}

export function CpuPowerControl() {
  const { data, reload } = usePoll<CpuPower>("get_cpu_power", undefined, 10000);
  const { result, busy, run } = useAction(reload);
  if (!data) return null;
  const online = data.cores.filter((c) => c.online).length;

  return (
    <section className="panel">
      <h2>CPU &amp; Power</h2>
      {data.available_power_profiles.length > 0 && (
        <div className="control-row">
          <label>Power profile</label>
          <Segmented
            options={data.available_power_profiles}
            value={data.power_profile}
            disabled={busy}
            onChange={(profile) => run("set_power_profile", { profile })}
          />
        </div>
      )}
      {data.available_platform_profiles.length > 0 && (
        <div className="control-row">
          <label>Fan &amp; thermal profile (firmware)</label>
          <Segmented
            options={data.available_platform_profiles}
            value={data.platform_profile}
            disabled={busy}
            onChange={(profile) => run("set_platform_profile", { profile })}
          />
        </div>
      )}
      {data.available_governors.length > 0 && (
        <div className="control-row">
          <label>CPU governor</label>
          <Segmented
            options={data.available_governors}
            value={data.governor}
            disabled={busy}
            onChange={(governor) => run("set_cpu_governor", { governor })}
          />
        </div>
      )}
      {data.available_energy_preferences.length > 0 && (
        <div className="control-row">
          <label>Energy preference</label>
          <Segmented
            options={data.available_energy_preferences}
            value={data.energy_preference}
            disabled={busy}
            onChange={(preference) => run("set_energy_preference", { preference })}
          />
        </div>
      )}
      <div className="control-row">
        <label>
          Cores ({online} of {data.cores.length} on) — click to switch one off
          or on
        </label>
      </div>
      <div className="core-toggle-grid">
        {data.cores.map((c) => (
          <button
            key={c.index}
            className={c.online ? "core-toggle on" : "core-toggle"}
            disabled={busy || !c.can_toggle}
            title={
              c.can_toggle
                ? `Core ${c.index} is ${c.online ? "on" : "off"}`
                : `Core ${c.index} can't be switched off`
            }
            onClick={() =>
              run("set_core_online", { index: c.index, online: !c.online })
            }
          >
            {c.index}
          </button>
        ))}
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        The power profile changes without a password; governor, firmware
        profile and cores ask for one. Switching cores off saves power but
        the remaining ones carry all the work. Everything here resets at
        reboot.
      </p>
    </section>
  );
}

export function FanControl() {
  const { data, reload } = usePoll<Fan[]>("list_fans", undefined, 3000);
  const { result, busy, run } = useAction(reload);
  const [percent, setPercent] = useState(60);
  if (!data) return null;

  return (
    <section className="panel">
      <h2>Fans</h2>
      {data.length === 0 && (
        <p className="diagnose-hint">
          This machine doesn't expose its fans to Linux.
        </p>
      )}
      {data.map((f) => (
        <div key={f.chip + f.label} className="control-row">
          <label>
            {f.label} — {f.rpm} RPM
            {f.automatic && <span className="mem-muted"> (automatic)</span>}
          </label>
          {f.pwm_path && (
            <>
              <button
                className="ctl-btn"
                disabled={busy}
                onClick={() =>
                  run("set_fan_mode", { pwmPath: f.pwm_path, mode: "auto", percent: 0 })
                }
              >
                Automatic
              </button>
              <button
                className="ctl-btn"
                disabled={busy}
                onClick={() =>
                  run("set_fan_mode", { pwmPath: f.pwm_path, mode: "full", percent: 100 })
                }
              >
                Full speed
              </button>
              {f.pwm_percent != null && (
                <button
                  className="ctl-btn"
                  disabled={busy}
                  onClick={() =>
                    run("set_fan_mode", { pwmPath: f.pwm_path, mode: "manual", percent })
                  }
                >
                  Hold at {percent}%
                </button>
              )}
            </>
          )}
        </div>
      ))}
      {data.some((f) => f.pwm_percent != null) && (
        <div className="control-row">
          <label>Manual speed ({percent}%)</label>
          <input
            type="range"
            min={25}
            max={100}
            value={percent}
            onChange={(e) => setPercent(parseInt(e.target.value, 10))}
          />
        </div>
      )}
      <StatusLine result={result} />
      {data.length > 0 && (
        <p className="diagnose-hint">
          Fan changes ask for your password and last until reboot. Manual
          speeds are never allowed below 25% so a fan can't be stopped by
          accident; on laptops whose firmware owns the curve, use the fan
          &amp; thermal profile above instead.
        </p>
      )}
    </section>
  );
}
