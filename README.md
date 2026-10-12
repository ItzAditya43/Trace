# Trace

A machine observatory and control centre for Linux. Trace shows what your computer is doing right now, remembers what it did, explains why it is slow or hot, and puts the settings that matter behind plain switches that can always be undone.

Built with Tauri (Rust) and React. Everything it tells you comes from measurements and fixed rules. Nothing is sent anywhere unless you switch that on.

![Trace](logo.png)

## What it does

| Page | What you get |
|---|---|
| **Overview** | Health score, live tiles, per-core CPU, memory, GPUs, temperatures, network, disks, top processes. Cards can be hidden and reordered. |
| **Memory** | Every byte of RAM accounted for by purpose, per-app memory with shared pages split fairly, growth over time, leak detection, and a quick-rescue list for when the machine is crawling. |
| **Activity** | Which apps are using the disk and network right now, and which wake the processor most. |
| **Hardware** | Every sensor, both graphics cards and what is using them, ports and what is plugged in, displays (layout, colour, saved layouts), brightness, temperature rules, a cooling test. |
| **Health** | Battery wear and drain per app, drives and their wear, failed services, a security check-up, microphone and camera use, what grew on the disk. |
| **History** | Charts for CPU, memory, swap, GPU, stalls and disks from one hour to thirty days, with warnings marked on the timeline, and a disk usage explorer. |
| **Logs** | A daily, weekly, monthly and yearly logbook: screen time hour by hour, apps, crashes, battery, data used, sleep, package changes, and what happened at the machine while you were away. |
| **Investigate** | "What is slowing me down?" and which program owns a port. |
| **Projects** | Every project's branch, uncommitted and unpushed work, build-file cleanup, and a work mode that starts its containers and dev command. |
| **Tweaks** | About 260 system settings with plain explanations, presets, suggestions for this machine, undo, and a way to measure whether a change helped. |
| **Control** | Processes, power, network and VPNs, devices and sound, services, automation, packages, cleanup and backups. |

There is also a **stats overlay**: a see-through, click-through strip or panel of live numbers (including frame rate in games) that sits on top of whatever you are doing.

Press **Ctrl+K** anywhere to search settings, pages, running programs, services and the logbook.

## Install

Download the latest build from [Releases](../../releases), or build it yourself:

```bash
npm install
npm run tauri build
```

See [Getting started](docs/getting-started.md) for requirements, the launcher entry and starting at login.

## Documentation

- [Getting started](docs/getting-started.md) — installing, first run, the tray, starting at login
- [The pages](docs/pages.md) — what each page shows and how to read it
- [Tweaks](docs/tweaks.md) — the settings catalogue, presets, undo, keeping settings across reboots
- [Control](docs/control.md) — processes, power, devices, services, packages, cleanup, backups
- [Network](docs/network.md) — VPNs, connection doctor, Wi-Fi, data allowances, who each app talks to
- [Automation](docs/automation.md) — profiles, rules, schedules, focus time, game mode, reminders
- [Command line](docs/command-line.md) — driving Trace from a terminal or a key binding
- [Safety and privacy](docs/safety-and-privacy.md) — what needs your password, what is stored, what leaves the machine
- [Requirements](docs/requirements.md) — which optional tools each feature uses
- [Development](docs/development.md) — layout of the code, tests, releases
- [Roadmap](ROADMAP.md) — the list of ideas and where each one lives in the app

## How it behaves

- **It keeps running in the tray** when the window is closed, so the logbook, rules, schedules and alerts keep working. Launching it again brings the existing window back. A Trace in the tray costs about one percent of one processor core. Its window is shut down completely while it is away, so only the background process stays in memory.
- **Nothing runs as administrator by default.** Actions that need it go through the desktop's own password prompt, one prompt per batch. An optional trusted helper lets chosen settings change without a prompt; it accepts only values from the catalogue and logs everything it does.
- **Every change is read back.** If the system ignored a setting, Trace says so instead of showing it as applied.
- **It never updates itself.** It checks this repository's releases on launch and shows a banner when a newer version exists.

## Versions

The version lives in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`; keep the three in step. A push to `main` whose version has no release yet builds and publishes one.
