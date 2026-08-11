import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface EventRow {
  ts: number;
  event_type: string;
  severity: string;
  message: string;
}

interface Diagnosis {
  cause: string;
  confidence: string;
  evidence: string[];
  events: EventRow[];
}

function formatTime(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export default function Diagnose() {
  const [diagnosis, setDiagnosis] = useState<Diagnosis | null>(null);
  const [loading, setLoading] = useState(false);
  const [portQuery, setPortQuery] = useState("");
  const [portResult, setPortResult] = useState<
    { pid: number; process_name: string; local_address: string }[] | null
  >(null);

  async function runDiagnosis() {
    setLoading(true);
    try {
      const d = await invoke<Diagnosis>("diagnose_slowness");
      setDiagnosis(d);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  async function lookupPort() {
    const port = parseInt(portQuery, 10);
    if (!port) return;
    try {
      const owners = await invoke<
        { pid: number; process_name: string; local_address: string }[]
      >("who_is_using_port", { port });
      setPortResult(owners);
    } catch (e) {
      console.error(e);
    }
  }

  return (
    <div>
      <section className="panel diagnose-panel">
        <button
          className="diagnose-btn"
          onClick={runDiagnosis}
          disabled={loading}
        >
          {loading ? "Investigating…" : "🔥 What's slowing me down?"}
        </button>
        <p className="diagnose-hint">
          Looks at the last hour of recorded activity for memory pressure,
          swap, CPU, and load spikes.
        </p>

        {diagnosis && (
          <div className="diagnosis-result">
            <div className="diagnosis-header">
              <span className="diagnosis-cause">{diagnosis.cause}</span>
              <span
                className={
                  "confidence-badge confidence-" +
                  diagnosis.confidence.toLowerCase()
                }
              >
                {diagnosis.confidence} confidence
              </span>
            </div>
            <ul className="evidence-list">
              {diagnosis.evidence.map((e, i) => (
                <li key={i}>{e}</li>
              ))}
            </ul>

            {diagnosis.events.length > 0 && (
              <div className="event-timeline">
                <h3>Timeline</h3>
                {diagnosis.events.map((e, i) => (
                  <div key={i} className={"event-row event-" + e.severity}>
                    <span className="event-time">{formatTime(e.ts)}</span>
                    <span className="event-message">{e.message}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </section>

      <section className="panel">
        <h2>Who's using this port?</h2>
        <div className="port-lookup">
          <input
            className="port-input"
            placeholder="e.g. 3000"
            value={portQuery}
            onChange={(e) => setPortQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && lookupPort()}
          />
          <button className="port-btn" onClick={lookupPort}>
            Look up
          </button>
        </div>
        {portResult && (
          <div className="port-result">
            {portResult.length === 0 ? (
              <p className="diagnose-hint">Nothing is listening there.</p>
            ) : (
              portResult.map((o, i) => (
                <div key={i} className="meta-row">
                  <span>
                    {o.process_name} (PID {o.pid})
                  </span>
                  <span>{o.local_address}</span>
                </div>
              ))
            )}
          </div>
        )}
      </section>
    </div>
  );
}
