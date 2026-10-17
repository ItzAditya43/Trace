# Getting started

## What you need

- Linux with a desktop session. Trace is developed on Arch Linux with Hyprland; most of it works on any distribution and desktop.
- WebKitGTK 4.1, which Tauri uses to draw the window.
- Optional tools unlock individual features. [Requirements](requirements.md) lists them; anything missing is simply hidden or says what to install.

## Installing

**One command**, on any 64-bit (x86_64) distribution with glibc:

```bash
curl -fsSL https://raw.githubusercontent.com/ItzAditya43/Trace/main/install.sh | sh
```

| Your system | What it does |
|---|---|
| Debian, Ubuntu, Mint, Pop!_OS… | installs the `.deb` with apt |
| Fedora, RHEL, CentOS… | installs the `.rpm` with dnf |
| Arch, Manjaro, EndeavourOS, CachyOS… | installs `webkit2gtk-4.1` and `gtk3` with pacman, and the program into `/usr/local` |
| openSUSE | installs WebKitGTK and GTK with zypper, and the program into `/usr/local` |
| anything else, or `--user` | unpacks the AppImage into `~/.local/share/trace-app`, with a `trace` command in `~/.local/bin` and an app-menu entry; no password needed |

Options go after `sh -s --`, for example `… | sh -s -- --user`:

- `--user`: home-folder install, whatever the distribution.
- `--version 1.0.1`: a particular version instead of the latest.
- `--uninstall`: remove Trace. Your history and settings in `~/.local/share/trace` are kept.

Not covered: ARM machines, musl-based systems such as Alpine, and NixOS (use `appimage-run` with the AppImage there).

**From a release by hand.** Download the build for your system from the Releases page and install it the usual way for its format.

**From source.**

```bash
npm install
npm run tauri build          # packages for your distribution
npm run tauri build -- --no-bundle   # just the program: src-tauri/target/release/trace
```

To run the bare program from your launcher, copy it somewhere on your path and add a desktop entry:

```bash
install -m 755 src-tauri/target/release/trace ~/.local/bin/trace
```

```ini
# ~/.local/share/applications/trace.desktop
[Desktop Entry]
Type=Application
Name=Trace
Comment=Observe, understand, and control your Linux machine
Exec=/home/YOU/.local/bin/trace
Icon=trace
Terminal=false
Categories=System;
StartupWMClass=trace
```

## First run

Trace opens on **Overview**. A **Quick Set-up** card asks four questions (battery, memory, development, games) and applies the matching presets. Answer it or press **Not now**; either way it does not come back.

Recording starts immediately. Pages that show history (History, Logs, leak detection, disk growth, battery drain) fill in as time passes.

## The tray

Closing the window sends Trace to the tray instead of quitting it, so the logbook, rules, schedules and alerts keep working. The window itself is shut down completely while it is away, which gives its memory back; reopening it takes a moment.

- **Left click** the tray icon to show or hide the window.
- **Right click** for live CPU, memory and temperature, the mini monitor, the power profile, keep-awake and Quit.

If your bar has no tray area, launching Trace again brings the window back. To make closing the window quit instead, switch off *Keep running when the window is closed* under **Control → Automation → Trace Itself**.

## Starting at login

**Control → Automation → Trace Itself → Start at login.** Trace adds a clearly marked block to your desktop's autostart file and removes it again when you switch this off. It starts hidden, in the tray.

Rules, schedules, alerts and the logbook only work while Trace is running, so this is worth switching on.

## The mini monitor

A small always-on-top window with CPU, memory, temperature, network and the six heaviest apps, each with Freeze and Quit. Open it from the tray menu, from **Trace Itself**, or with `trace mini`. `trace panic` opens it and never closes it, which makes it a good key binding for when the desktop is crawling.

## The stats overlay

A see-through panel of live numbers that sits on top of whatever you are doing, including a game. Clicks pass straight through it and it never takes the keyboard. Open it from the tray menu, with `trace overlay`, or from **Control → Display & Devices → Stats Overlay**, which is also where it is set up, since the overlay itself can't be clicked:

- **Shape**: a slim strip, a small panel, or automatic (a strip while a game runs, a panel otherwise).
- **Where**: any corner or the middle of the top or bottom edge, on a chosen monitor, or automatically: the screen a running game is on, otherwise whichever is in use. It keeps clear of bars and stays inside the screen; a strip too wide for a small screen becomes a panel.
- **Tint and text size**, and which readings to show: frame rate, CPU, GPU, memory, temperature, fan, network, battery, clock.

**Frame rate.** Only a game knows its own frame rate, so the game has to report it. The settings card gives a launch option to paste into the game's properties in Steam; it runs MangoHud inside the game, drawing nothing, and Trace shows the number. Without it the overlay shows everything except frame rate.

Staying above other windows and letting clicks through are done by Hyprland. On other desktops the overlay opens as an ordinary small window.

## Where things are kept

| What | Where |
|---|---|
| History and logbook | `~/.local/share/trace/history.sqlite3` |
| Rules, schedules, presets, other settings | JSON files in `~/.local/share/trace/` |
| Profiles | TOML files in the same folder |

Raw ten-second samples are kept for 90 days. The logbook's hourly and daily records are kept indefinitely.
