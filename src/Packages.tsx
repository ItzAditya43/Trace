import { ConfirmButton, StatusLine, bytesToHuman, useAction, usePoll } from "./ui";

interface PackageStatus {
  manager: string;
  updates: { name: string; current: string; new: string }[];
  orphans: string[];
  cache_bytes: number;
  can_clean_cache: boolean;
}

export default function Packages() {
  const { data, loading, reload } = usePoll<PackageStatus>("get_package_status");
  const { result, busy, run } = useAction(reload);

  if (!data) {
    return (
      <p className="loading">
        Checking for updates — this asks the package mirrors, so it can take a
        few seconds…
      </p>
    );
  }
  if (data.manager === "none") {
    return (
      <section className="panel">
        <h2>Packages</h2>
        <p className="diagnose-hint">
          No supported package manager (pacman or apt) was found.
        </p>
      </section>
    );
  }

  return (
    <>
      <section className="panel">
        <h2>
          Pending Updates ({data.updates.length}) — {data.manager}
        </h2>
        {data.updates.length === 0 ? (
          <p className="status-ok">✓ Everything is up to date.</p>
        ) : (
          <table className="proc-table">
            <thead>
              <tr>
                <th>Package</th>
                <th>Installed</th>
                <th>Available</th>
              </tr>
            </thead>
            <tbody>
              {data.updates.slice(0, 60).map((u) => (
                <tr key={u.name}>
                  <td>{u.name}</td>
                  <td>{u.current}</td>
                  <td>{u.new}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {data.updates.length > 60 && (
          <p className="diagnose-hint">…and {data.updates.length - 60} more.</p>
        )}
        <div className="control-actions">
          <button className="ctl-btn" onClick={reload} disabled={loading || busy}>
            {loading ? "Checking…" : "Check again"}
          </button>
        </div>
        <p className="diagnose-hint">
          Trace lists updates but doesn't install them: a full system upgrade
          can ask questions and should be run in a terminal (
          {data.manager === "pacman" ? "sudo pacman -Syu" : "sudo apt upgrade"}
          ). A kernel or driver update will then show up under Health →
          Restart needed.
        </p>
      </section>

      <section className="panel">
        <h2>Cleanup</h2>
        <div className="control-row">
          <label>
            Package cache — {bytesToHuman(data.cache_bytes)} of downloaded
            installers
          </label>
          <ConfirmButton
            label="Clean old versions"
            className="ctl-btn"
            disabled={busy || !data.can_clean_cache}
            onConfirm={() => run("clean_package_cache")}
          />
        </div>
        {data.manager === "pacman" && (
          <>
            <div className="control-row">
              <label>
                Orphans — {data.orphans.length} package(s) installed as
                dependencies that nothing needs any more
              </label>
              <ConfirmButton
                label="Remove orphans"
                confirmLabel={`Remove ${data.orphans.length} packages — confirm`}
                disabled={busy || data.orphans.length === 0}
                onConfirm={() => run("remove_orphan_packages")}
              />
            </div>
            <div className="chip-row">
              {data.orphans.map((o) => (
                <span key={o} className="chip">
                  {o}
                </span>
              ))}
            </div>
          </>
        )}
        <StatusLine result={result} />
        <p className="diagnose-hint">
          Both ask for your password. Cleaning keeps the two newest versions
          of each installed package so you can still downgrade. Read the
          orphan list before removing — a package you use directly can be
          listed if it was first pulled in as a dependency.
        </p>
      </section>
    </>
  );
}
