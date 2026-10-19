import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface Platform {
  desktop: string;
  session: string;
  packages: string;
  firewall: string;
  network_manager: boolean;
  pipewire: boolean;
  laptop: boolean;
  distro: string;
  hyprland: boolean;
  features: { name: string; available: boolean; needs: string }[];
}

// Asked once: none of this changes while Trace is open.
let cached: Platform | null = null;
let pending: Promise<Platform> | null = null;

export function usePlatform(): Platform | null {
  const [platform, setPlatform] = useState<Platform | null>(cached);
  useEffect(() => {
    if (cached) return;
    pending ??= invoke<Platform>("get_platform");
    pending.then((p) => {
      cached = p;
      setPlatform(p);
    });
  }, []);
  return platform;
}

/** Shows its children only where the desktop can do what they need. */
export function NeedsHyprland({ children }: { children: React.ReactNode }) {
  const p = usePlatform();
  if (!p || !p.hyprland) return null;
  return <>{children}</>;
}

const DESKTOP_NAME: Record<string, string> = {
  hyprland: "Hyprland",
  kde: "KDE Plasma",
  gnome: "GNOME",
  sway: "Sway",
  xfce: "Xfce",
  cinnamon: "Cinnamon",
  other: "your desktop",
};

/** Which of Trace's features can work here, and what to install for the rest. */
export function WhatWorksHere() {
  const p = usePlatform();
  const [open, setOpen] = useState(false);
  if (!p) return null;
  const available = p.features.filter((f) => f.available);
  const missing = p.features.filter((f) => !f.available);
  return (
    <section className="panel wide" style={{ order: 998 }}>
      <h2>What Works On This Machine</h2>
      <p className="diagnose-hint">
        {p.distro} · {DESKTOP_NAME[p.desktop] ?? p.desktop} ({p.session}) · {available.length} of {p.features.length} features
        available{missing.length ? ". The rest are hidden until what they need is installed." : "."}{" "}
        {missing.length > 0 && (
          <button className="link-button" onClick={() => setOpen(!open)}>
            {open ? "Hide the list" : "Show what's missing"}
          </button>
        )}
      </p>
      {open && (
        <div className="event-scroll">
          {missing.map((f) => (
            <div key={f.name} className="plain-row">
              <span className="mem-muted">✗</span> {f.name} <span className="mem-muted">— needs {f.needs}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
