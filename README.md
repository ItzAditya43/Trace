# Trace

A Linux machine observatory and control center. Trace watches your CPU, RAM,
GPU, disks, network, and processes in real time, keeps a local history so you
can see what changed and when, diagnoses slowness with rule-based event
correlation, scans your project folders for git/dependency/TODO status, and
gives you a privileged control layer (process/cgroup limits, systemd, Docker,
GPU tuning) driven by reversible, dry-runnable profiles.

Built with Tauri (Rust) + React/TypeScript.

## Features

- **Live dashboard** — CPU (per-core), RAM, swap, GPU (NVIDIA/AMD), temps, network, battery, disks, top processes
- **History** — SQLite-backed timeline charts for resource and disk usage over time
- **Investigate** — "What's slowing me down?" diagnosis from correlated event history, plus port → process lookup
- **Projects** — scans a directory for git status, languages, dependencies, TODOs, and activity level
- **Control** — freeze/kill/limit processes, manage systemd services and Docker containers, tune GPU settings, all via undoable/dry-runnable profiles with a full audit log

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
