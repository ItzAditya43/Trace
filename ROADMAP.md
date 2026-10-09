# Trace — feature ideas

A backlog of ideas, not a commitment. Items marked ✅ are built; struck-through items were dropped.

No AI features: everything Trace says comes from measurements and fixed rules.

Each idea is tagged **S**, **M** or **L** for how much work it is. The numbers are stable so an idea can be referred to by number.

Two things apply to several ideas:

- Anything under "Automate it" only works while Trace is running, so start-at-login needs to be on first.
- Ideas marked *(leaves the machine)* send some data to another device or service and should always be opt-in.

## Suggested next

**49 first** (fan speed — everything is in place except the experiment after a reboot).

**15** (panic overlay — the command exists, the always-on-top mini window doesn't), **2**, **5**, **7**, **9**, **17**, **26**, then finishing the partly built ones (**23**, **33**, **34**, **35**, **56**).

---

## Understand what the machine is doing

1. ✅ **Built** (Overview → “Why is it like this right now?”) — **"Why is it hot / loud / slow right now?" button (S).** One click gives a plain answer from data Trace already has: which app, which power profile, whether it's charging, whether memory is stalling.
2. **Battery drain per app (M).** Estimates how many watts each app costs and how much battery time closing it would buy back.
3. ✅ **Built** (Hardware → Graphics Cards) — **What's keeping the dedicated GPU awake (M).** Shows which apps hold the NVIDIA GPU on and what that costs in battery.
4. ✅ **Built** (Health → Battery Over Time → Recent sleeps) — **Sleep report (M).** After each sleep: how long, how much battery it lost, what woke it, and what blocked it from sleeping.
5. **Honest screen time (S).** Detects when you're away from the keyboard, so a window left focused overnight stops counting. Adds an hour-by-hour strip of what you used each day.
6. ✅ **Built** (Memory → Possible Memory Leaks (appears when an app has only grown for 3+ hours)) — **Leak detector (S).** Flags apps whose memory climbs steadily over days and offers a scheduled restart. The history is already recorded.
7. **"What grew on my disk?" (M).** Snapshots folder sizes weekly and shows the difference.
8. ✅ **Built** (shows up under Logs and History as warnings) — **Hardware trouble watcher (S).** Turns kernel log noise into plain alerts: drive errors, USB devices resetting, CPU throttling from heat, memory errors.
9. **Measure a tweak's effect (M).** Records stalls, temperature, power draw and responsiveness for a few minutes before and after a change, then says whether it helped.
10. ~~**Weekly digest (S).** One notification each Monday: hours used, top apps, crashes, battery wear change, disk growth, biggest memory grower.~~ *(cut: the Logs page already gives the weekly view on demand)*

## Automate it

11. ✅ **Built** (Control → Automation → Scheduled Profiles (charger, battery level, Wi-Fi, monitor)) — **Automatic presets by situation (M).** Switch preset on charger plug/unplug, lid, an external monitor, a full-screen game, or which Wi-Fi you're on. Example: unknown network → firewall "public" plus VPN on.
12. ✅ **Built** (three “battery falls to…” schedules) — **Low-battery ladder (S).** At 30 / 20 / 10% it steps through battery saver, dimming, freezing chosen apps, then sleep.
13. ✅ **Built** (Control → Automation → Sleep Apps You Aren't Using) — **Sleep apps you aren't using (M).** Auto-freezes chosen heavy apps after N minutes unfocused and wakes them when you click back. On a machine short of RAM this is likely the biggest single win.
14. ✅ **Built** (Control → Automation → Focus Time (freezes apps for a set time; does not block network or mute alerts)) — **Focus mode (S).** One switch for a set time: freeze chosen apps, block their network, mute alerts, then restore everything.
15. ✅ **Built** (Memory → Quick Rescue; `trace panic` lands there) — **Panic key (M).** A global shortcut that opens a tiny, always-responsive list of memory hogs to freeze or kill when the desktop is crawling. It can also trigger itself when stalls spike.
16. ✅ **Built** (Control → Apps & Processes → “Tell me when it's done”) — **"Tell me when it's done" (S).** Pick any running process, or "when the CPU goes idle", and get a notification when a build, download or render finishes.
17. **Weekly maintenance run (S).** Trim, package cache, journal, crash dumps, Docker leftovers and an update check, done while plugged in and idle, with a report.
18. ✅ **Built** (`trace preset <id>`, `trace freeze|resume|quit|restart <app>`, `trace page <name>`, `trace panic`) — **Keyboard shortcuts and a command line (S).** `trace preset gaming` or `trace freeze brave`, so presets and actions can be bound to keys.

## For development work

19. ✅ **Built** (Projects → Find build files) — **Developer junk cleaner (M).** Per project: size of `node_modules`, `target/`, virtual environments, plus Docker images, volumes and build cache, with one-click clearing.
20. ✅ **Built** (Control → Apps & Processes → Listening Ports) — **Dev server and port manager (M).** Every listening port with the project it belongs to, plus open in browser, stop, and conflict warnings.
21. **Project work mode (L).** Opening a project starts its containers, database and dev server; closing it stops them and frees the RAM.
22. ✅ **Built** (Control → Apps & Processes → Run With Restrictions (no network, memory and CPU caps; read-only home is not built)) — **Run restricted (M).** Launch any program with no network, a RAM or CPU cap, or a read-only home folder. Useful for untrusted tools.

## Hardware and devices

23. ✅ **Built** (capacity history on Health; the timed full charge is not built) — **Battery care (M).** A capacity-over-months chart, and "charge to 100% by 8 am tomorrow" so a charge limit doesn't catch you before travel.
24. ✅ **Built** (Hardware → Displays → Saved layouts) — **Display layouts (M).** Save docked, undocked and presentation layouts and apply them automatically when a monitor is connected.
25. ✅ **Built** (Health → Microphone & Camera (live view; no history yet)) — **Mic and camera watch (M).** Shows which app is using the microphone or camera now, with a history of who used them and when.
26. **Update assistant (L).** Pending updates with warnings (kernel or GPU driver means a reboot; distribution news that needs manual steps), a snapshot first where possible, then what needs restarting afterwards. Also helps merge leftover `.pacnew` config files.

## Safety

27. ✅ **Built** (Health → Security Check-up (read-only: no fix buttons; vulnerable-package check needs `arch-audit` installed)) — **Security check-up (M).** One page: ports reachable from the network, SSH weaknesses, failed login attempts, packages with known vulnerabilities, disk encryption, firewall. Each item gets a fix button.
28. **While-you-were-away log (S).** USB devices plugged in, failed unlocks and sudo attempts, lid opened. Optional alert to a phone through KDE Connect *(leaves the machine)*.

## Everyday shortcuts

29. ✅ **Built** (☆ on any setting in Tweaks; shown on Overview) — **Pinned switches (S).** Pin any setting from Tweaks (keep awake, blur, mic mute, a VPN) to the Overview page and the tray menu as a one-click switch.
30. ✅ **Built** (Control → Power & Memory → Timers) — **Timers (S).** "Sleep in 45 minutes", "shut down when this download finishes", "keep awake for 2 hours".
31. ~~**First-run tune-up (S).** Five questions (mostly plugged in? gaming? development? battery or speed?) and Trace applies the presets and suggestions that fit.~~ *(cut: a wizard adds little once presets and suggestions exist)*
32. **Mini monitor and your own Overview (M).** A small always-on-top window showing CPU, RAM, temperature and network; plus choosing and reordering the cards on Overview.

## Internet

33. ✅ **Built** (Control → Network → Connection Doctor; speed history is not built) — **Connection doctor and speed history (M).** Tests the router, the internet and DNS separately to say where a problem is, and keeps a chart of speed and delay per network over time.
34. **Wi-Fi manager (M) — partly built** (Control → Network → Saved Wi-Fi Networks: show password, auto-join, forget; signal history, channel crowding and QR sharing are not built).** Signal strength over time, how crowded each channel is, and saved networks with show-password, share by QR code, forget and priority.
35. ✅ **Built** (Control → Network → Data Used This Month; hotspot mode is not built) — **Data budget and hotspot mode (S).** A monthly data allowance per network with a warning at 80%, and on a phone hotspot it pauses updates and other heavy background traffic. Builds on the data-used record that already exists.
36. **Who is my computer talking to? (L).** Per app: which sites and countries it contacted, with new or unusual ones flagged.

## Files and backup

37. **Duplicates and forgotten files (M).** Duplicate files, the largest files, things not opened in a year and old downloads, removed to the trash rather than deleted.
38. **Backups with restore (L).** Pick folders and a destination (external drive or another machine), a schedule, a check that the backup is readable, and a browser to restore single files.
39. ✅ **Built** (Control → Automation → Trace Itself → Move to another machine) — **Move to a new machine (S).** One file holding everything Trace manages — kept settings, presets, rules, profiles, schedules — that can be loaded on a fresh install.

## Gaming and graphics

40. **Per-game profiles (M).** Notices a game starting (Steam, Proton, Lutris), applies the Gaming preset and that game's own options, shows a frame-rate overlay if wanted, and puts everything back afterwards.
41. ✅ **Built** (Hardware → Graphics Cards) — **Graphics use per app (M).** Which app is using how much GPU and video memory, which GPU each app runs on, and a "start this on the dedicated GPU" option.

## Privacy and wellbeing

42. ✅ **Built** (Control → Packages → Privacy Cleaner (recent files, browser caches, clipboard; thumbnails not included)) — **Privacy cleaner (M).** Recent-file lists, thumbnails, browser caches and clipboard history, with a preview of exactly what will be removed.
43. ~~**Breaks and app time limits (S).** A reminder after a long stretch at the keyboard, and optional daily limits per app with a gentle nudge. Builds on the screen-time record.~~ *(cut: not what Trace is for)*

## Explain and ask

44. **Explain this (M).** On any process, service, log error or setting: what it is, whether it is safe to stop, and what usually causes it. From a built-in reference for common ones.
45. ~~**Ask Trace (L).** Questions in plain language over your own logbook: "why was it slow yesterday at 3 pm?", "what changed since last week?". Can use a model running on the machine (Ollama) so nothing leaves it; a hosted model would be optional *(leaves the machine)*.~~ *(cut: no AI features, by decision)*

## Power user

46. ✅ **Built** (Control → Services → Scheduled Jobs) — **Scheduled jobs overview (S).** Every timer and cron job on the system in one list: when it last ran, how long it took, when it runs next, with run-now and disable.
47. **Budgets for groups of apps (M).** Limits for a whole group rather than one app: "browsers get at most 6 GB in total", "builds get half the CPU on battery".
48. ✅ **Built** (profile step “Run my own command”, usable from any schedule or situation trigger) — **Your own hooks (M).** Run a script of yours on any Trace event: an app started, battery under 20%, the VPN dropped, a disk passed 90%.

## Hardware and display ideas added later

49. **Fan speed and curves (M) — next, needs a reboot first.** The firmware of the ASUS Vivobook M3500QC does have a real fan-duty call; Linux just never uses it. Found by decompiling the firmware tables:
    - `\_SB.PCI0.SBRG.EC0.ST84(fan, duty)` sets a fan's duty (fan 0 or 1, duty 0–255); `ST83(fan)` reads it back (0xFF appears to mean "automatic"); `TACH(fan)` reads RPM.
    - Nothing in the firmware tables calls `ST84`, and the ASUS WMI interface the kernel driver uses reports fan curves as unsupported, so it can't be reached through the driver.
    - The controller is driven through a firmware call (SMI) with a shared buffer, not ordinary controller registers — reading those returns all zeros.
    - Safe way to call it: the `acpi_call` kernel module (installed, loads after the reboot into the new kernel), which runs the method under the firmware's own locks.
    - Still to find out, with the machine in front of someone: which call returns a fan to automatic, and the lowest duty that keeps it spinning. Until that is known nothing should write to it unattended.
50. ✅ **Built** (Hardware → Temperature Rules) — **Thermal rules (M).** "Above 85°C for a minute → quiet profile / cap CPU at 70%; below 70°C → back." Needs a way to act without a password prompt each time (see 53).
51. ✅ **Built** (Hardware → Cooling Test (CPU only)) — **Stress test and cooling check (M).** A timed CPU or GPU load with a temperature and throttling curve, to judge paste, dust and fan health, and to compare before and after cleaning.
52. **Per-monitor colour (M).** Colour profiles (ICC), gamma and saturation per display, and HDR on/off where the panel supports it.
53. ✅ **Built** (Tweaks → Trusted Helper, with its own log) — **Trusted helper (L).** A small root-owned helper that only accepts the catalogue's own settings and values, so presets, thermal rules and schedules can change root-level settings without a password prompt each time. Opt-in.
54. **Keyboard and lighting (S).** Backlight timeout, per-zone RGB where the laptop exposes it, and "light up on key press only".
55. ✅ **Built** (read-only so far: Hardware → Ports & Connections) — **USB power per port (S).** Which ports wake the laptop, which keep charging while it sleeps, and per-device power saving.
56. **Drive tools (M) — partly built** (Health → Drives → Self-test; wear over time and secure-erase are not built).** Run a SMART self-test, see wear over time, secure-erase and trim on demand.
57. **Audio hardware (M).** Per-app volume and output, an equaliser preset per device, and microphone noise suppression on/off.
58. **Dock memory (S).** Remember per dock or monitor: layout, audio output, power profile and which apps to start.
