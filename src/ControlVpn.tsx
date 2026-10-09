import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ConfirmButton, StatusLine, useAction, usePoll } from "./ui";

interface VpnConnection {
  name: string;
  uuid: string;
  kind: string;
  active: boolean;
  autoconnect: boolean;
  remote: string;
}

interface GateServer {
  host: string;
  ip: string;
  country: string;
  country_code: string;
  ping_ms: number;
  speed_mbps: number;
  sessions: number;
  uptime_days: number;
  operator: string;
  protocol: string;
  port: number;
}

const SORTS: { key: string; label: string; compare: (a: GateServer, b: GateServer) => number }[] = [
  { key: "speed", label: "Fastest", compare: (a, b) => b.speed_mbps - a.speed_mbps },
  { key: "ping", label: "Lowest ping", compare: (a, b) => (a.ping_ms || 9999) - (b.ping_ms || 9999) },
  { key: "quiet", label: "Fewest users", compare: (a, b) => a.sessions - b.sessions },
  { key: "uptime", label: "Longest uptime", compare: (a, b) => b.uptime_days - a.uptime_days },
];

function fileName(path: string): string {
  return path.split("/").pop() ?? path;
}

function MyVpns({
  vpns,
  busy,
  run,
}: {
  vpns: VpnConnection[];
  busy: boolean;
  run: (command: string, args?: Record<string, unknown>) => void;
}) {
  const [renaming, setRenaming] = useState<string | null>(null);
  const [newName, setNewName] = useState("");

  return (
    <div className="unit-list vpn-list">
      {vpns.map((v) => (
        <div key={v.uuid} className="unit-row">
          {renaming === v.uuid ? (
            <input
              className="port-input"
              value={newName}
              autoFocus
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  run("rename_vpn", { uuid: v.uuid, name: newName });
                  setRenaming(null);
                }
                if (e.key === "Escape") setRenaming(null);
              }}
              aria-label={`New name for ${v.name}`}
            />
          ) : (
            <span className="unit-name" title={`${v.name} — ${v.remote}`}>
              {v.active ? "● " : ""}
              {v.name} <span className="mem-muted">{v.kind} {v.remote}</span>
            </span>
          )}
          <span className={"unit-state " + (v.active ? "unit-active" : "unit-inactive")}>
            {v.active ? "connected" : v.autoconnect ? "auto" : "off"}
          </span>
          <div className="unit-buttons">
            {renaming === v.uuid ? (
              <>
                <button
                  className="ctl-btn-sm"
                  onClick={() => {
                    run("rename_vpn", { uuid: v.uuid, name: newName });
                    setRenaming(null);
                  }}
                >
                  Save
                </button>
                <button className="ctl-btn-sm" onClick={() => setRenaming(null)}>
                  Cancel
                </button>
              </>
            ) : (
              <>
                <button
                  className={v.active ? "ctl-btn-sm" : "ctl-btn-sm ctl-primary"}
                  disabled={busy}
                  onClick={() => run("set_vpn_active", { uuid: v.uuid, up: !v.active })}
                >
                  {v.active ? "Disconnect" : "Connect"}
                </button>
                <button
                  className="ctl-btn-sm"
                  disabled={busy}
                  title="Connect automatically when the network comes up"
                  onClick={() =>
                    run("set_vpn_autoconnect", { uuid: v.uuid, enabled: !v.autoconnect })
                  }
                >
                  Auto: {v.autoconnect ? "on" : "off"}
                </button>
                <button
                  className="ctl-btn-sm"
                  disabled={busy}
                  onClick={() => {
                    setRenaming(v.uuid);
                    setNewName(v.name);
                  }}
                >
                  Rename
                </button>
                <ConfirmButton
                  label="Delete"
                  confirmLabel="Confirm"
                  className="ctl-btn-sm ctl-danger"
                  disabled={busy}
                  onConfirm={() => run("delete_vpn", { uuid: v.uuid })}
                />
              </>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

function GateBrowser({
  busy,
  run,
}: {
  busy: boolean;
  run: (command: string, args?: Record<string, unknown>) => void;
}) {
  const [servers, setServers] = useState<GateServer[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [country, setCountry] = useState("all");
  const [sortKey, setSortKey] = useState("speed");

  async function load() {
    setLoading(true);
    setError(null);
    try {
      setServers(await invoke<GateServer[]>("fetch_vpn_gate_servers"));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  if (!servers) {
    return (
      <>
        <div className="control-actions">
          <button className="ctl-btn ctl-primary" onClick={load} disabled={loading}>
            {loading ? "Loading servers…" : "Browse VPN Gate servers"}
          </button>
        </div>
        {error && <p className="status-fail">✗ {error}</p>}
      </>
    );
  }

  const countries = [...new Map(servers.map((s) => [s.country_code, s.country])).entries()].sort(
    (a, b) => a[1].localeCompare(b[1])
  );
  const sort = SORTS.find((s) => s.key === sortKey)!;
  const shown = servers
    .filter((s) => country === "all" || s.country_code === country)
    .sort(sort.compare);

  return (
    <>
      <div className="range-picker project-toolbar">
        <select
          className="port-input small-select"
          value={country}
          onChange={(e) => setCountry(e.target.value)}
          aria-label="Country"
        >
          <option value="all">All countries ({servers.length})</option>
          {countries.map(([code, name]) => (
            <option key={code} value={code}>
              {name} ({servers.filter((s) => s.country_code === code).length})
            </option>
          ))}
        </select>
        <select
          className="port-input small-select"
          value={sortKey}
          onChange={(e) => setSortKey(e.target.value)}
          aria-label="Sort servers"
        >
          {SORTS.map((s) => (
            <option key={s.key} value={s.key}>
              {s.label}
            </option>
          ))}
        </select>
        <span className="log-spacer" />
        <button className="range-btn" onClick={load} disabled={loading}>
          {loading ? "Reloading…" : "Reload list"}
        </button>
      </div>
      <div className="gate-scroll">
        <table className="proc-table activity-table">
          <thead>
            <tr>
              <th></th>
              <th>Country</th>
              <th>Server</th>
              <th>Speed</th>
              <th>Ping</th>
              <th>Users</th>
              <th>Up for</th>
              <th>Type</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((s) => (
              <tr key={s.host + s.ip}>
                <td>
                  <div className="unit-buttons">
                    <button
                      className="ctl-btn-sm ctl-primary"
                      disabled={busy}
                      onClick={() => run("add_vpn_gate_server", { host: s.host, connect: true })}
                    >
                      Connect
                    </button>
                    <button
                      className="ctl-btn-sm"
                      disabled={busy}
                      title="Add to your VPNs without connecting"
                      onClick={() => run("add_vpn_gate_server", { host: s.host, connect: false })}
                    >
                      Save
                    </button>
                  </div>
                </td>
                <td>{s.country}</td>
                <td title={`${s.ip} — run by ${s.operator}`}>{s.host}</td>
                <td>{s.speed_mbps.toFixed(0)} Mbps</td>
                <td>{s.ping_ms > 0 ? `${s.ping_ms} ms` : "—"}</td>
                <td>{s.sessions}</td>
                <td>
                  {s.uptime_days >= 1
                    ? `${s.uptime_days.toFixed(0)} d`
                    : `${(s.uptime_days * 24).toFixed(0)} h`}
                </td>
                <td>
                  {s.protocol.toUpperCase()} {s.port}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}

export function VpnControl() {
  const vpns = usePoll<VpnConnection[]>("list_vpns", undefined, 8000);
  const configs = usePoll<string[]>("find_vpn_configs");
  const { result, busy, run } = useAction(() => {
    vpns.reload();
    configs.reload();
  });
  const [path, setPath] = useState("");
  const active = (vpns.data ?? []).find((v) => v.active);

  return (
    <section className="panel wide">
      <h2>VPN</h2>
      {active ? (
        <p className="status-ok">
          ● Connected through <strong>{active.name}</strong> {active.remote && `(${active.remote})`}
        </p>
      ) : (
        <p className="diagnose-hint">Not connected to a VPN.</p>
      )}

      <h3 className="sub-heading">Your VPNs ({vpns.data?.length ?? 0})</h3>
      {vpns.data && vpns.data.length === 0 && (
        <p className="diagnose-hint">None saved yet — add one from a file or from VPN Gate below.</p>
      )}
      <MyVpns vpns={vpns.data ?? []} busy={busy} run={run} />
      <StatusLine result={busy ? { ok: true, message: "Working… connecting can take up to 40 seconds" } : result} />

      <h3 className="sub-heading">Add from a file</h3>
      <div className="control-row">
        <label>OpenVPN (.ovpn) or WireGuard (.conf) file</label>
        <input
          className="port-input"
          placeholder="/path/to/config.ovpn"
          value={path}
          onChange={(e) => setPath(e.target.value)}
        />
        <button
          className="ctl-btn"
          disabled={busy || !path.trim()}
          onClick={() => {
            run("import_vpn_file", { path: path.trim() });
            setPath("");
          }}
        >
          Add
        </button>
      </div>
      {(configs.data ?? []).length > 0 && (
        <div className="chip-row">
          <span className="mem-muted">Found in Downloads:</span>
          {(configs.data ?? []).map((c) => (
            <button
              key={c}
              className="chip chip-add"
              disabled={busy}
              title={`Add ${c}`}
              onClick={() => run("import_vpn_file", { path: c })}
            >
              + {fileName(c)}
            </button>
          ))}
        </div>
      )}

      <h3 className="sub-heading">VPN Gate — free public servers</h3>
      <GateBrowser busy={busy} run={run} />
      <p className="diagnose-hint">
        VPNs are saved in NetworkManager, so they also appear in your system's
        network menu. VPN Gate servers are run by volunteers: whoever runs
        the one you pick can see which sites you connect to (not the contents
        of HTTPS pages), most keep logs for about two weeks, and they go
        offline without notice. Fine for getting around a regional block, not
        for anything you need kept private.
      </p>
    </section>
  );
}
