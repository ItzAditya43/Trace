# Development

```bash
npm install
npm run tauri dev
```

## Layout

| Path | What is there |
|---|---|
| `src-tauri/src/lib.rs` | Application state, the background loop, the tray, the command line, and every command the window can call |
| `src-tauri/src/*.rs` | One module per area: `memory_map`, `hardware`, `health`, `logbook`, `tweaks` and `tweak_catalog`, `vpn`, `automation`, `profiles`, and so on |
| `src/App.tsx` | The window's shell and the Overview page |
| `src/*.tsx` | One file per page or group of cards |
| `src/ui.tsx` | Shared pieces: polling, action results, confirm buttons, charts |

### Conventions

- **Commands are asynchronous.** Every `#[tauri::command]` is `command(async)` or an `async fn`. Plain commands run on the window's thread, where anything that starts a program or waits on a password prompt would freeze the window.
- **Polling stops when hidden.** Pages poll through `usePoll`, which pauses while the window is hidden. A hidden window that keeps polling is most of what an idle Trace would cost.
- **Administrator commands are built from checked values only.** Numbers are parsed and range-checked; names are matched against lists the code built itself.
- **Settings are data.** A tweak is an entry in `tweak_catalog.rs` describing where the value lives and what it accepts. Reading, validating, applying, verifying and undoing are generic.

## Tests

```bash
cd src-tauri
cargo test                 # the automated suite
cargo test -- --ignored    # tests that act on the real machine
```

The ignored tests change things for real (they toggle a Wi-Fi setting and put it back, start and stop the equaliser, run a backup into a scratch folder), so read one before running it. Some need hardware or tools that a given machine may not have.

`live_readers_do_not_panic` runs every read-only collector against the real machine. With `TRACE_DUMP_DIR` set it also writes each result as JSON, which is useful as realistic data when working on the window.

## Releases

The version lives in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. A push to `main` whose version has no GitHub release yet builds and publishes one. The workflow fails if the three files disagree.
