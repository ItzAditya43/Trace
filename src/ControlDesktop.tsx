import { ConfirmButton, StatusLine, bytesToHuman, useAction, usePoll } from "./ui";

interface AudioDevice {
  name: string;
  description: string;
  is_default: boolean;
  muted: boolean;
}

interface CleanupStatus {
  trash_bytes: number;
  thumbnails_bytes: number;
  journal_bytes: number;
  coredump_bytes: number;
  user_cache_bytes: number;
}

export function SessionControl() {
  const { result, busy, run } = useAction();
  const act = (action: string) => run("session_action", { action });

  return (
    <section className="panel">
      <h2>Session &amp; Power</h2>
      <div className="control-actions">
        <button className="ctl-btn" disabled={busy} onClick={() => act("lock")}>
          Lock screen
        </button>
        <button className="ctl-btn" disabled={busy} onClick={() => act("suspend")}>
          Suspend
        </button>
        <ConfirmButton label="Log out" disabled={busy} onConfirm={() => act("logout")} />
        <ConfirmButton
          label="Reboot"
          confirmLabel="Closes everything — confirm reboot"
          disabled={busy}
          onConfirm={() => act("reboot")}
        />
        <ConfirmButton
          label="Shut down"
          confirmLabel="Closes everything — confirm shutdown"
          disabled={busy}
          onConfirm={() => act("poweroff")}
        />
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Log out, reboot and shut down close every open app without saving.
      </p>
    </section>
  );
}

function DeviceList({
  title,
  kind,
  devices,
  busy,
  onPick,
}: {
  title: string;
  kind: string;
  devices: AudioDevice[];
  busy: boolean;
  onPick: (kind: string, name: string) => void;
}) {
  return (
    <>
      <h3 className="sub-heading">{title}</h3>
      {devices.length === 0 && <p className="diagnose-hint">None found.</p>}
      {devices.map((d) => (
        <div key={d.name} className="control-row">
          <label>
            {d.is_default ? "● " : ""}
            {d.description}
            {d.muted && <span className="mem-muted"> (muted)</span>}
          </label>
          {d.is_default ? (
            <span className="status-ok">In use</span>
          ) : (
            <button className="ctl-btn" disabled={busy} onClick={() => onPick(kind, d.name)}>
              Use this
            </button>
          )}
        </div>
      ))}
    </>
  );
}

export function AudioControl() {
  const { data, reload } = usePoll<{ outputs: AudioDevice[]; inputs: AudioDevice[] }>(
    "get_audio_devices",
    undefined,
    10000
  );
  const { result, busy, run } = useAction(reload);
  if (!data) return null;
  const pick = (kind: string, name: string) => run("set_default_audio", { kind, name });

  return (
    <section className="panel">
      <h2>Audio Devices</h2>
      <DeviceList title="Output" kind="sink" devices={data.outputs} busy={busy} onPick={pick} />
      <DeviceList title="Input" kind="source" devices={data.inputs} busy={busy} onPick={pick} />
      <StatusLine result={result} />
    </section>
  );
}

export function CleanupControl() {
  const { data, loading, reload } = usePoll<CleanupStatus>("get_cleanup_status");
  const { result, busy, run } = useAction(reload);

  if (!data) {
    return (
      <section className="panel">
        <h2>Disk Cleanup</h2>
        <p className="loading">Measuring what can be cleaned…</p>
      </section>
    );
  }
  const rows: {
    target: string;
    label: string;
    bytes: number;
    button: string;
    note: string;
  }[] = [
    {
      target: "trash",
      label: "Trash",
      bytes: data.trash_bytes,
      button: "Empty trash",
      note: "deleted files you haven't purged — gone for good after this",
    },
    {
      target: "journal",
      label: "System journal",
      bytes: data.journal_bytes,
      button: "Trim to 500 MB",
      note: "old system logs; asks for your password",
    },
    {
      target: "coredumps",
      label: "Crash dumps",
      bytes: data.coredump_bytes,
      button: "Delete dumps",
      note: "memory images of crashed programs; asks for your password",
    },
    {
      target: "thumbnails",
      label: "Thumbnail cache",
      bytes: data.thumbnails_bytes,
      button: "Clear",
      note: "image previews, rebuilt on demand",
    },
  ];

  return (
    <section className="panel">
      <h2>Disk Cleanup</h2>
      {rows.map((r) => (
        <div key={r.target} className="control-row">
          <label>
            {r.label} — <strong>{bytesToHuman(r.bytes)}</strong>
            <span className="mem-muted"> · {r.note}</span>
          </label>
          <ConfirmButton
            label={r.button}
            className="ctl-btn"
            disabled={busy || loading || r.bytes < 1024 * 1024}
            onConfirm={() => run("clean_up", { target: r.target })}
          />
        </div>
      ))}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Your app cache folder holds {bytesToHuman(data.user_cache_bytes)} in
        total. Trace doesn't clear it wholesale because apps keep logins and
        offline data there — use the Disk Usage Explorer on the History page
        to see which app owns what.
      </p>
    </section>
  );
}
