import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
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

interface PublicAddress {
  ip: string;
  country_code: string;
}

interface KillSwitchStatus {
  active: boolean;
  hostname_vpns: string[];
}

interface SplitStatus {
  active: boolean;
  interface: string;
  vpn_name: string;
  apps: string[];
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
        <button
          className="range-btn"
          disabled={busy}
          title="Tries the fastest servers one after another until one connects"
          onClick={() => run("connect_best_vpn_gate", { countryCode: country === "all" ? "any" : country })}
        >
          Connect to the fastest{country === "all" ? "" : ` in ${countries.find(([c]) => c === country)?.[1] ?? country}`}
        </button>
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

function KillSwitch({ vpnActive }: { vpnActive: boolean }) {
  const { data, reload } = usePoll<KillSwitchStatus>("get_vpn_kill_switch", undefined, 10000);
  const { result, busy, run } = useAction(reload);
  const [allowLan, setAllowLan] = useState(true);
  if (!data) return null;

  return (
    <>
      <h3 className="sub-heading">Kill switch</h3>
      <div className="control-row">
        <label>
          {data.active ? (
            <span className="status-ok">
              ● On — nothing leaves this machine except through a VPN
              {!vpnActive && " (no VPN is connected, so you are offline)"}
            </span>
          ) : (
            "Block all traffic that isn't going through a VPN, so a dropped connection can't expose you"
          )}
        </label>
        {!data.active && (
          <label className="check-row">
            <input type="checkbox" checked={allowLan} onChange={(e) => setAllowLan(e.target.checked)} />
            still allow my local network
          </label>
        )}
        <ConfirmButton
          label={data.active ? "Turn off 🔒" : "Turn on 🔒"}
          confirmLabel={data.active ? "Confirm: turn off" : vpnActive ? "Confirm: turn on" : "No VPN is up — you'll go offline. Confirm"}
          className="ctl-btn"
          disabled={busy}
          onConfirm={() => run("set_vpn_kill_switch", { enabled: !data.active, allowLan })}
        />
      </div>
      <StatusLine result={result} />
      <p className="diagnose-hint">
        While on, website lookups (DNS) outside the tunnel are blocked too, so they can't leak. A
        reboot always clears it.
        {data.hostname_vpns.length > 0 &&
          ` These saved VPNs are stored by name rather than address and can't reconnect while it is on: ${data.hostname_vpns.join(", ")}.`}
      </p>
    </>
  );
}

function SplitTunnel({ vpns }: { vpns: VpnConnection[] }) {
  const { data, reload } = usePoll<SplitStatus>("get_vpn_split", undefined, 8000);
  const { result, busy, run } = useAction(reload);
  const [uuid, setUuid] = useState("");
  const [command, setCommand] = useState("");
  if (!data) return null;
  const chosen = uuid || vpns[0]?.uuid || "";

  return (
    <>
      <h3 className="sub-heading">Only some apps through the VPN</h3>
      {data.active ? (
        <>
          <p className="status-ok">
            ● On — “{data.vpn_name}” ({data.interface}) carries only the programs started below.
          </p>
          <div className="control-row">
            <label>Start a program through the VPN</label>
            <input
              className="port-input"
              placeholder="firefox   or   qbittorrent"
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && command.trim()) {
                  run("launch_in_vpn", { command });
                  setCommand("");
                }
              }}
            />
            <button
              className="ctl-btn ctl-primary"
              disabled={busy || !command.trim()}
              onClick={() => {
                run("launch_in_vpn", { command });
                setCommand("");
              }}
            >
              Start
            </button>
            <ConfirmButton
              label="Turn off 🔒"
              className="ctl-btn"
              disabled={busy}
              onConfirm={() => run("stop_vpn_split")}
            />
          </div>
          <div className="chip-row">
            <span className="mem-muted">Using the VPN now:</span>
            {data.apps.length === 0 && <span className="mem-muted">nothing yet</span>}
            {data.apps.map((a, i) => (
              <span key={i} className="chip">
                {a}
              </span>
            ))}
          </div>
        </>
      ) : (
        <div className="control-row">
          <label>
            Leave everything else on your normal connection and send only programs you start from
            here through
          </label>
          <select
            className="port-input small-select"
            value={chosen}
            onChange={(e) => setUuid(e.target.value)}
            aria-label="VPN to use"
          >
            {vpns.map((v) => (
              <option key={v.uuid} value={v.uuid}>
                {v.name}
              </option>
            ))}
          </select>
          <button
            className="ctl-btn"
            disabled={busy || !chosen}
            onClick={() => run("start_vpn_split", { uuid: chosen })}
          >
            Turn on 🔒
          </button>
        </div>
      )}
      <StatusLine result={busy ? { ok: true, message: "Working… this connects the VPN first" } : result} />
      <p className="diagnose-hint">
        A program has to be started from here to use the VPN — one that is already open keeps its
        normal connection. Those programs' IPv6 is blocked (so it can't bypass the tunnel) and
        their website lookups still use your usual DNS server unless you changed it under Tweaks.
      </p>
    </>
  );
}

export function VpnControl() {
  const vpns = usePoll<VpnConnection[]>("list_vpns", undefined, 8000);
  const configs = usePoll<string[]>("find_vpn_configs");
  const [address, setAddress] = useState<PublicAddress | null>(null);
  const [previous, setPrevious] = useState<PublicAddress | null>(null);
  const [checking, setChecking] = useState(false);

  async function checkAddress() {
    setChecking(true);
    try {
      const next = await invoke<PublicAddress>("get_public_address");
      setAddress((current) => {
        if (current && current.ip !== next.ip) setPrevious(current);
        return next;
      });
    } catch {
      // Offline (or the kill switch is doing its job): nothing to show.
    } finally {
      setChecking(false);
    }
  }

  const { result, busy, run } = useAction(() => {
    vpns.reload();
    configs.reload();
    // After anything that may have changed the route, look again.
    if (address) checkAddress();
  });
  const [path, setPath] = useState("");
  const active = (vpns.data ?? []).find((v) => v.active);

  async function browse() {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "VPN configuration", extensions: ["ovpn", "conf"] }],
    });
    if (typeof picked === "string") run("import_vpn_file", { path: picked });
  }

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

      <div className="control-row">
        <label>
          {address ? (
            <>
              The internet sees you as <strong>{address.ip}</strong> ({address.country_code})
              {previous && (
                <span className="mem-muted">
                  {" "}
                  — before: {previous.ip} ({previous.country_code})
                </span>
              )}
            </>
          ) : (
            "Check which address and country the internet sees — do it before connecting to see the difference"
          )}
        </label>
        <button className="ctl-btn" disabled={checking} onClick={checkAddress}>
          {checking ? "Checking…" : address ? "Check again" : "Check my address"}
        </button>
      </div>

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
        <button className="ctl-btn" disabled={busy} onClick={browse}>
          Browse…
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

      <KillSwitch vpnActive={!!active} />
      <SplitTunnel vpns={vpns.data ?? []} />

      <h3 className="sub-heading">VPN Gate — free public servers</h3>
      <GateBrowser busy={busy} run={run} />
      <p className="diagnose-hint">
        Checking your address asks Cloudflare (1.1.1.1) and only happens when you press the
        button, or after a connect once you have. VPNs are saved in NetworkManager, so they also appear in your system's
        network menu. VPN Gate servers are run by volunteers: whoever runs
        the one you pick can see which sites you connect to (not the contents
        of HTTPS pages), most keep logs for about two weeks, and they go
        offline without notice. Fine for getting around a regional block, not
        for anything you need kept private.
      </p>
    </section>
  );
}
