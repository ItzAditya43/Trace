import { useContext, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { NavContext } from "./nav";

interface Hit {
  kind: "page" | "setting" | "process" | "service" | "log";
  title: string;
  detail: string;
  page: string;
  section: string | null;
  query: string | null;
}

const KIND_LABEL: Record<Hit["kind"], string> = {
  page: "Pages",
  setting: "Settings",
  process: "Running programs",
  service: "Services",
  log: "From the logbook",
};
const KIND_ORDER: Hit["kind"][] = ["page", "setting", "process", "service", "log"];

function detailText(hit: Hit): string {
  if (hit.kind !== "log") return hit.detail;
  // "Label|unix-seconds"
  const [label, ts] = hit.detail.split("|");
  const when = new Date(parseInt(ts, 10) * 1000).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  return `${label} · ${when}`;
}

export default function SearchPalette({ onClose }: { onClose: () => void }) {
  const { go } = useContext(NavContext);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [selected, setSelected] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => input.current?.focus(), []);

  // Search a moment after typing stops, and ignore answers to stale queries.
  useEffect(() => {
    let cancelled = false;
    const id = setTimeout(() => {
      invoke<Hit[]>("global_search", { query })
        .then((found) => {
          if (cancelled) return;
          // Keep the on-screen order identical to the keyboard order.
          setHits(KIND_ORDER.flatMap((k) => found.filter((h) => h.kind === k)));
          setSelected(0);
        })
        .catch(() => !cancelled && setHits([]));
    }, 160);
    return () => {
      cancelled = true;
      clearTimeout(id);
    };
  }, [query]);

  function choose(hit: Hit) {
    go({ page: hit.page, section: hit.section ?? undefined, query: hit.query ?? undefined });
    onClose();
  }

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(s + 1, hits.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter" && hits[selected]) choose(hits[selected]);
  }

  return (
    <div className="palette-backdrop" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Search">
        <input
          ref={input}
          className="port-input palette-input"
          placeholder="Search settings, pages, programs, services, logs…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
        />
        <div className="palette-results">
          {query.trim() === "" && (
            <p className="diagnose-hint">
              Try "blur", "swap", "docker", "battery", "kill switch" or the name of a running program.
            </p>
          )}
          {query.trim() !== "" && hits.length === 0 && <p className="diagnose-hint">Nothing found.</p>}
          {hits.map((hit, i) => (
            <div key={i}>
              {(i === 0 || hits[i - 1].kind !== hit.kind) && (
                <h3 className="sub-heading">{KIND_LABEL[hit.kind]}</h3>
              )}
              <button
                className={i === selected ? "palette-hit selected" : "palette-hit"}
                onMouseEnter={() => setSelected(i)}
                onClick={() => choose(hit)}
              >
                <span className="palette-title">{hit.title}</span>
                <span className="mem-muted">{detailText(hit)}</span>
              </button>
            </div>
          ))}
        </div>
        <p className="diagnose-hint palette-foot">↑ ↓ to move · Enter to open · Esc to close</p>
      </div>
    </div>
  );
}
