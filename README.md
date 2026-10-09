# Trace

A Linux machine observatory and control center. Trace watches your CPU, RAM,
GPU, disks, network, and processes in real time, keeps a local history so you
can see what changed and when, diagnoses slowness with rule-based event
correlation, scans your project folders for git/dependency/TODO status, and
gives you a privileged control layer (process/cgroup limits, systemd, Docker,
GPU tuning) driven by reversible, dry-runnable profiles.

Built with Tauri (Rust) + React/TypeScript.

## Features

- **Overview** — health score with the top issues, CPU (per-core), RAM, swap, stall time, GPU (NVIDIA/AMD), temps, network, battery, disks, top processes
- **Memory** — accounts for every byte of RAM by purpose (apps, file cache, kernel, graphics, compressed swap), per-app RAM + swap with shared pages split fairly, per-app growth over time, where swapped-out data lives, and restart / freeze / quit / cap actions on each app
- **Activity** — which apps are using the disk and the network right now, and which wake the CPU most
- **Health** — 0–100 score with plain-language issues, reboot-needed detection (replaced kernel, NVIDIA driver mismatch, apps running outdated binaries), failed services, battery wear, boot-time trend, drive SMART health, out-of-memory kills and recent journal errors
- **History** — hover-to-inspect charts for CPU, RAM, swap, GPU, memory/disk stall time and each disk over 1 hour to 30 days, with warnings marked on the timeline, plus a disk usage explorer
- **Logs** — a daily / weekly / monthly / yearly logbook: time recorded, screen time per app (focused window), apps that ran in the background, average and peak CPU/RAM, crashes and out-of-memory kills copied from the system journal, warnings, control actions and boots; any period can be exported as JSON
- **Investigate** — "What's slowing me down?" diagnosis from correlated event history, plus port → process lookup
- **Projects** — scans a folder for each project's branch, uncommitted and unpushed work, last commit, languages, dependencies, TODOs and size, with filters and one-click open in editor, terminal or file manager
- **Control**, in sections:
  - *Apps & Processes* — search, freeze/kill/limit processes, windows and workspaces, namespaces
  - *Power & Memory* — lock/suspend/log out/reboot/shut down, power profile, CPU governor, firmware fan/thermal profile, per-core on/off, fans, swappiness, drop caches, compact memory, empty swap, brightness/volume, GPU tuning
  - *Network* — Wi-Fi and Bluetooth, firewalld ports, bandwidth limits, per-app network blocking
  - *Display & Devices* — resolution, refresh rate, scale and night light (Hyprland), audio output/input selection, USB authorization, clipboard history
  - *Services* — systemd units, Docker containers, autostart apps, startup impact
  - *Automation* — undoable/dry-runnable profiles with a builder, memory rules ("if an app exceeds N GB, notify/freeze/restart/quit"), profiles scheduled by time or app launch, desktop alerts, and a full audit log
  - *Packages & Cleanup* — pending updates, package cache cleanup and orphan removal (pacman; updates and cache on apt), plus trash, journal, crash-dump and thumbnail cleanup

Actions that need root (governor, swap, fans, SMART, packages) go through polkit (`pkexec`), so the desktop's own password prompt appears each time; nothing runs as root otherwise. Rules, schedules, alerts and the logbook only run while Trace is open. Raw 10-second samples are kept for 90 days; the logbook's hourly and daily records are kept indefinitely.

## Updates

The version lives in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (keep the three in step). A push to `main` whose version has no [GitHub Release](../../releases) yet builds and publishes one; pushing without bumping the version publishes nothing new. The app checks that feed on launch and shows a dismissible banner if a newer version exists — it never downloads or installs anything on its own.

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
