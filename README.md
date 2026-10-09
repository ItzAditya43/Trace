# Trace

A Linux machine observatory and control center. Trace watches your CPU, RAM,
GPU, disks, network, and processes in real time, keeps a local history so you
can see what changed and when, diagnoses slowness with rule-based event
correlation, scans your project folders for git/dependency/TODO status, and
gives you a privileged control layer (process/cgroup limits, systemd, Docker,
GPU tuning) driven by reversible, dry-runnable profiles.

Built with Tauri (Rust) + React/TypeScript.

## Features

- **Overview** — health score with the top issues, live tiles, CPU (per-core), RAM, swap, stall time, GPU, temps, network, battery, disks, top processes
- **Memory** — accounts for every byte of RAM by purpose, per-app RAM + swap with shared pages split fairly, growth over time, and per-app actions: restart, freeze, quit, and CPU / RAM limits (no password needed)
- **Activity** — which apps are using the disk and the network right now, and which wake the CPU most
- **Hardware** — every sensor the machine reports with live trends, fans, CPU power, both graphics cards with which apps are using them (and whether the dedicated one is powered down), built-in and external-monitor brightness, wireless device batteries, and display arrangement: resolution, refresh, scale, rotation, position, mirroring, adaptive sync, saved layouts that apply themselves, and a night-light schedule
- **Health** — 0–100 score with plain-language issues, reboot-needed detection, failed services, battery wear, boot-time trend, drive SMART health, out-of-memory kills and recent journal errors
- **History** — hover-to-inspect charts over 1 hour to 30 days with warnings marked on the timeline, plus a disk usage explorer
- **Logs** — a daily / weekly / monthly / yearly logbook: screen time per app, apps that ran, CPU/RAM/temperature, battery drain and cycles, network data used, crashes, sleep / lid / login times, package changes, warnings, alerts sent and control actions; any period exports as JSON
- **Investigate** — "What's slowing me down?" diagnosis, plus port → process lookup
- **Projects** — every project's branch, uncommitted and unpushed work, size and TODOs, with one-click open
- **Tweaks** — a curated catalogue of about 260 system settings (only the ones your machine actually has are shown — over 200 on a typical laptop), each with a plain explanation:
  - *what* — power and CPU, laptop and battery, memory and swap, disks, network, desktop effects and windows (Hyprland, GTK), mouse / keyboard / touchpad, audio, privacy and security, background services, developer limits, clock and logs
  - *how* — settings that need root are collected and applied with **one** password prompt; every change is read back and reported honestly if the system ignored it; anything can be undone or reset to the value Trace first saw; chosen settings can be kept across reboots
  - *presets* — Battery saver, Balanced, Performance, Gaming, Quiet and cool, zram tuning, Lightweight desktop, Security hardening, Developer workstation, plus your own
  - *suggestions* — a short list worked out from this machine's own state (for example "you run zram and zswap together")
  - deliberately excluded: anything that can't be undone or could stop the machine booting (bootloader, kernel command line, disk layout, sudo rules, one-way kernel lock-downs)
- **Control**, in sections:
  - *Apps & Processes* — search, freeze/kill/limit processes, windows and workspaces, namespaces
  - *Power & Memory* — lock/suspend/log out/reboot/shut down, power profile, governor, cores, fans, swap tuning, brightness/volume, GPU tuning
  - *Network* — VPNs (connect, rename, auto-connect, delete; add from a file or from the VPN Gate public list; connect to the fastest server in a country; kill switch; only-some-apps-through-the-VPN; public address check), Wi-Fi and Bluetooth, firewalld ports, bandwidth limits, per-app network blocking
  - *Display & Devices* — resolution, refresh rate, scale and night light (Hyprland), audio output/input, USB authorization, clipboard history
  - *Services* — systemd units and Docker containers with their logs and live stats, autostart apps, startup impact
  - *Automation* — profiles (with a builder, import, and "apply settings preset" steps), memory rules, schedules by time, app launch, charger, battery level, Wi-Fi network or monitor, sleeping apps you aren't using, timers, pinned switches, alerts and their history, start at login, run in the tray, and a full audit log
  - *Packages & Cleanup* — pending updates, package cache and orphan cleanup, trash / journal / crash-dump / thumbnail cleanup
- **Search** — Ctrl+K searches settings, pages, running programs, services and the logbook

From a terminal or a key binding: `trace preset gaming`, `trace freeze brave`, `trace page hardware`, `trace panic`.

Trace keeps running in the tray when its window is closed (so the logbook, rules and alerts keep working), and a second launch brings the existing window back instead of starting twice.

Actions that need root go through polkit (`pkexec`), so the desktop's own password prompt appears; nothing runs as root otherwise. Rules, schedules, alerts and the logbook only run while Trace is running. Raw 10-second samples are kept for 90 days; the logbook's hourly and daily records are kept indefinitely.

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
