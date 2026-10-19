import { useState } from "react";
import { Segmented, StatusLine, useAction, usePoll } from "./ui";

interface FirewallStatus {
  backend: string;
  available: boolean;
  running: boolean;
  responded: boolean;
  zone: string;
  services: string[];
  ports: string[];
}

interface Radio {
  kind: string;
  enabled: boolean;
  hard_blocked: boolean;
}

interface WifiNetwork {
  ssid: string;
  signal: number;
  security: string;
  connected: boolean;
  known: boolean;
}

interface BluetoothDevice {
  address: string;
  name: string;
  connected: boolean;
}

interface Monitor {
  name: string;
  description: string;
  mode: string;
  scale: number;
  available_modes: string[];
}

interface DisplayStatus {
  monitors: Monitor[];
  night_light_kelvin: number | null;
}

const RADIO_NAMES: Record<string, string> = {
  wlan: "Wi-Fi",
  bluetooth: "Bluetooth",
  wwan: "Mobile data",
  nfc: "NFC",
};

export function FirewallControl() {
  const { data, reload } = usePoll<FirewallStatus>("get_firewall_status");
  const { result, busy, run } = useAction(reload);
  const [port, setPort] = useState("");
  const [protocol, setProtocol] = useState("tcp");
  const [permanent, setPermanent] = useState(false);
  if (!data) {
    return (
      <section className="panel">
        <h2>Firewall</h2>
        <p className="loading">Asking the firewall — this takes a few seconds…</p>
      </section>
    );
  }

  const portNumber = parseInt(port, 10);
  const valid = portNumber > 0 && portNumber < 65536;
  const change = (open: boolean, p = portNumber, proto = protocol) =>
    run("set_firewall_port", { port: p, protocol: proto, open, permanent });

  return (
    <section className="panel">
      <h2>Firewall</h2>
      {!data.available ? (
        <p className="diagnose-hint">
          No firewall (firewalld or ufw) is installed, so Trace can't manage inbound ports here.
        </p>
      ) : !data.running ? (
        <p className="status-fail">
          ! The firewall is installed but not running — every port is
          reachable. Start it from Services ({data.backend === "ufw" ? "ufw" : "firewalld"}).
        </p>
      ) : !data.responded ? (
        <>
          <p className="diagnose-hint">
            The firewall is running, but it didn't answer when asked for its
            rules (it can take a long time when the machine is busy).
          </p>
          <div className="control-actions">
            <button className="ctl-btn" onClick={reload}>
              Ask again
            </button>
          </div>
        </>
      ) : (
        <>
          {data.backend === "ufw" ? (
            <p className="status-ok">
              ✓ ufw is on. Its rules can only be read by an administrator, so they aren't listed here; ports you open or
              close below stay that way after a reboot.
            </p>
          ) : (
            <p className="status-ok">
              ✓ Running — zone "{data.zone}" blocks inbound connections except:
            </p>
          )}
          {data.backend !== "ufw" && <div className="chip-row">
            {data.services.map((s) => (
              <span key={s} className="chip">
                {s}
              </span>
            ))}
            {data.ports.map((p) => {
              const [num, proto] = p.split("/");
              return (
                <button
                  key={p}
                  className="chip chip-button"
                  title="Close this port"
                  disabled={busy}
                  onClick={() => change(false, parseInt(num, 10), proto)}
                >
                  {p} ✕
                </button>
              );
            })}
            {data.services.length + data.ports.length === 0 && (
              <span className="mem-muted">nothing</span>
            )}
          </div>}
          <div className="control-row">
            <label>Open or close a port</label>
            <input
              className="port-input small"
              placeholder="8080"
              value={port}
              onChange={(e) => setPort(e.target.value)}
            />
            <Segmented options={["tcp", "udp"]} value={protocol} onChange={setProtocol} />
            <button className="ctl-btn" disabled={busy || !valid} onClick={() => change(true)}>
              Open
            </button>
            <button className="ctl-btn" disabled={busy || !valid} onClick={() => change(false)}>
              Close
            </button>
          </div>
          <label className="check-row">
            <input
              type="checkbox"
              checked={permanent}
              onChange={(e) => setPermanent(e.target.checked)}
            />
            Keep the change after reboot
          </label>
        </>
      )}
      <StatusLine result={result} />
    </section>
  );
}

export function WirelessControl() {
  const radios = usePoll<Radio[]>("list_radios", undefined, 10000);
  const wifi = usePoll<WifiNetwork[]>("list_wifi", { rescan: false });
  const bluetooth = usePoll<BluetoothDevice[]>("list_bluetooth", undefined, 10000);
  const { result, busy, run } = useAction(() => {
    radios.reload();
    wifi.reload();
    bluetooth.reload();
  });
  const [passwordFor, setPasswordFor] = useState<string | null>(null);
  const [password, setPassword] = useState("");

  function connect(n: WifiNetwork) {
    if (!n.known && n.security && passwordFor !== n.ssid) {
      setPasswordFor(n.ssid);
      setPassword("");
      return;
    }
    run("connect_wifi", { ssid: n.ssid, password: n.known ? null : password });
    setPasswordFor(null);
  }

  return (
    <section className="panel">
      <h2>Wi-Fi &amp; Bluetooth</h2>
      {(radios.data ?? []).map((r) => (
        <div key={r.kind} className="control-row">
          <label>
            {RADIO_NAMES[r.kind] ?? r.kind}
            {r.hard_blocked && (
              <span className="mem-muted"> — off at the hardware switch</span>
            )}
          </label>
          <Segmented
            options={["on", "off"]}
            value={r.enabled ? "on" : "off"}
            disabled={busy || r.hard_blocked}
            onChange={(v) => run("set_radio", { kind: r.kind, enabled: v === "on" })}
          />
        </div>
      ))}

      <h3 className="sub-heading">Networks</h3>
      {(wifi.data ?? []).slice(0, 8).map((n) => (
        <div key={n.ssid}>
          <div className="control-row">
            <label>
              {n.connected ? "● " : ""}
              {n.ssid}
              <span className="mem-muted">
                {" "}
                {n.signal}% · {n.security || "open"}
                {n.known ? " · saved" : ""}
              </span>
            </label>
            {n.connected ? (
              <span className="status-ok">Connected</span>
            ) : (
              <button className="ctl-btn" disabled={busy} onClick={() => connect(n)}>
                Connect
              </button>
            )}
          </div>
          {passwordFor === n.ssid && (
            <div className="control-row">
              <label>Password for {n.ssid}</label>
              <input
                className="port-input"
                type="password"
                value={password}
                autoFocus
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && connect(n)}
              />
              <button className="ctl-btn" disabled={busy || !password} onClick={() => connect(n)}>
                Join
              </button>
            </div>
          )}
        </div>
      ))}
      {wifi.data && wifi.data.length === 0 && (
        <p className="diagnose-hint">No networks in range (or Wi-Fi is off).</p>
      )}
      <div className="control-actions">
        <button className="ctl-btn" disabled={busy} onClick={() => run("list_wifi", { rescan: true })}>
          Rescan
        </button>
      </div>

      <h3 className="sub-heading">Bluetooth devices</h3>
      {(bluetooth.data ?? []).map((d) => (
        <div key={d.address} className="control-row">
          <label>
            {d.connected ? "● " : ""}
            {d.name}
          </label>
          <button
            className="ctl-btn"
            disabled={busy}
            onClick={() =>
              run("set_bluetooth_connected", { address: d.address, connect: !d.connected })
            }
          >
            {d.connected ? "Disconnect" : "Connect"}
          </button>
        </div>
      ))}
      {bluetooth.data && bluetooth.data.length === 0 && (
        <p className="diagnose-hint">No paired devices.</p>
      )}
      <StatusLine result={busy ? { ok: true, message: "Working…" } : result} />
    </section>
  );
}

export function DisplayControl() {
  const { data, reload } = usePoll<DisplayStatus>("get_display_status");
  const { result, busy, run } = useAction(reload);
  const [kelvin, setKelvin] = useState<number | null>(null);
  if (!data) return null;
  const temperature = kelvin ?? data.night_light_kelvin ?? 6500;

  return (
    <section className="panel">
      <h2>Display</h2>
      {data.monitors.length === 0 && (
        <p className="diagnose-hint">
          Display control needs Hyprland — no monitors were reported.
        </p>
      )}
      {data.monitors.map((m) => (
        <div key={m.name}>
          <div className="control-row">
            <label>
              {m.name} <span className="mem-muted">{m.description}</span>
            </label>
            <select
              className="port-input"
              value={m.available_modes.find((mode) => sameMode(mode, m.mode)) ?? ""}
              disabled={busy}
              onChange={(e) =>
                run("set_monitor_mode", { name: m.name, mode: e.target.value, scale: m.scale })
              }
            >
              {m.available_modes.map((mode) => (
                <option key={mode} value={mode}>
                  {mode.replace("@", " @ ")}
                </option>
              ))}
            </select>
          </div>
          <div className="control-row">
            <label>Scale</label>
            <Segmented
              options={["1", "1.25", "1.5", "2"]}
              value={String(m.scale)}
              disabled={busy}
              onChange={(scale) => {
                const mode = m.available_modes.find((x) => sameMode(x, m.mode));
                if (mode) {
                  run("set_monitor_mode", { name: m.name, mode, scale: parseFloat(scale) });
                }
              }}
            />
          </div>
        </div>
      ))}
      {data.night_light_kelvin != null && (
        <div className="control-row">
          <label>Night light ({temperature} K{temperature >= 6500 ? ", off" : ""})</label>
          <input
            type="range"
            min={2500}
            max={6500}
            step={100}
            value={temperature}
            onChange={(e) => setKelvin(parseInt(e.target.value, 10))}
            onMouseUp={() =>
              run("set_night_light", { kelvin: temperature >= 6500 ? null : temperature })
            }
          />
          <button
            className="ctl-btn"
            disabled={busy}
            onClick={() => {
              setKelvin(6500);
              run("set_night_light", { kelvin: null });
            }}
          >
            Off
          </button>
        </div>
      )}
      <StatusLine result={result} />
      <p className="diagnose-hint">
        Resolution and scale apply to this session only; your Hyprland config
        is left alone, so logging out restores it.
      </p>
    </section>
  );
}

/** "1920x1080@59.997Hz" and "1920x1080@60.00Hz" are the same advertised mode. */
function sameMode(a: string, b: string): boolean {
  const parse = (m: string) => {
    const [size, rate] = m.replace("Hz", "").split("@");
    return { size, rate: parseFloat(rate) };
  };
  const x = parse(a);
  const y = parse(b);
  return x.size === y.size && Math.abs(x.rate - y.rate) < 0.6;
}
