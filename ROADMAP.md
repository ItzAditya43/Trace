# Trace — feature ideas

A backlog of ideas, not a commitment. Items marked ✅ are built; struck-through items were dropped.

No AI features: everything Trace says comes from measurements and fixed rules.

Each idea is tagged **S**, **M** or **L** for how much work it is. The numbers are stable so an idea can be referred to by number.

Two things apply to several ideas:

- Anything under "Automate it" only works while Trace is running, so start-at-login needs to be on first.
- Ideas marked *(leaves the machine)* send some data to another device or service and should always be opt-in.

## Suggested next

Every idea here is built except two that are out by decision: **45** (Ask Trace: no AI features) and **49** (fan curves: specific to one laptop model).

Known limits inside built ideas: the frame-rate overlay in game mode is MangoHud's own launch option rather than something Trace switches on; spinning disks can't be erased from Trace; countries for addresses come from a registry lookup on request, not a local database.

---

## Understand what the machine is doing

1. ✅ **Built** (Overview → “Why is it like this right now?”) — **"Why is it hot / loud / slow right now?" button (S).** One click gives a plain answer from data Trace already has: which app, which power profile, whether it's charging, whether memory is stalling.
2. ✅ **Built** (Health → Battery Drain Per App; learns this machine's idle draw from time on battery) — **Battery drain per app (M).** Estimates how many watts each app costs and how much battery time closing it would buy back.
3. ✅ **Built** (Hardware → Graphics Cards) — **What's keeping the dedicated GPU awake (M).** Shows which apps hold the NVIDIA GPU on and what that costs in battery.
4. ✅ **Built** (Health → Battery Over Time → Recent sleeps) — **Sleep report (M).** After each sleep: how long, how much battery it lost, what woke it, and what blocked it from sleeping.
5. ✅ **Built** (screen time stops counting after 5 idle minutes unless sound is playing; hour-by-hour strip on the Logs page) — **Honest screen time (S).** Detects when you're away from the keyboard, so a window left focused overnight stops counting. Adds an hour-by-hour strip of what you used each day.
6. ✅ **Built** (Memory → Possible Memory Leaks (appears when an app has only grown for 3+ hours)) — **Leak detector (S).** Flags apps whose memory climbs steadily over days and offers a scheduled restart. The history is already recorded.
7. ✅ **Built** (Health → What Grew On The Disk; a snapshot each week on the charger, or on demand) — **"What grew on my disk?" (M).** Snapshots folder sizes weekly and shows the difference.
8. ✅ **Built** (shows up under Logs and History as warnings) — **Hardware trouble watcher (S).** Turns kernel log noise into plain alerts: drive errors, USB devices resetting, CPU throttling from heat, memory errors.
9. ✅ **Built** (Tweaks → Did That Setting Help?) — **Measure a tweak's effect (M).** Records stalls, temperature, power draw and responsiveness for a few minutes before and after a change, then says whether it helped.
10. ✅ **Built** (Control → Automation → Reminders & Small Things, off by default) — **Weekly digest (S).** One notification each Monday: hours used, top apps, crashes, battery wear change, disk growth, biggest memory grower.

## Automate it

11. ✅ **Built** (Control → Automation → Scheduled Profiles (charger, battery level, Wi-Fi, monitor)) — **Automatic presets by situation (M).** Switch preset on charger plug/unplug, lid, an external monitor, a full-screen game, or which Wi-Fi you're on. Example: unknown network → firewall "public" plus VPN on.
12. ✅ **Built** (three “battery falls to…” schedules) — **Low-battery ladder (S).** At 30 / 20 / 10% it steps through battery saver, dimming, freezing chosen apps, then sleep.
13. ✅ **Built** (Control → Automation → Sleep Apps You Aren't Using) — **Sleep apps you aren't using (M).** Auto-freezes chosen heavy apps after N minutes unfocused and wakes them when you click back. On a machine short of RAM this is likely the biggest single win.
14. ✅ **Built** (Control → Automation → Focus Time (freezes apps for a set time, which also stops their network and notifications, and holds back Trace's own alerts)) — **Focus mode (S).** One switch for a set time: freeze chosen apps, block their network, mute alerts, then restore everything.
15. ✅ **Built** (`trace panic` opens the always-on-top mini window with freeze and quit buttons; bind it to a key in Hyprland) — **Panic key (M).** A global shortcut that opens a tiny, always-responsive list of memory hogs to freeze or kill when the desktop is crawling. It can also trigger itself when stalls spike.
16. ✅ **Built** (Control → Apps & Processes → “Tell me when it's done”) — **"Tell me when it's done" (S).** Pick any running process, or "when the CPU goes idle", and get a notification when a build, download or render finishes.
17. ✅ **Built** (Control → Packages → Weekly Maintenance; by hand, or by itself weekly after asking for the password once) — **Weekly maintenance run (S).** Trim, package cache, journal, crash dumps, Docker leftovers and an update check, done while plugged in and idle, with a report.
18. ✅ **Built** (`trace preset <id>`, `trace freeze|resume|quit|restart <app>`, `trace page <name>`, `trace panic`) — **Keyboard shortcuts and a command line (S).** `trace preset gaming` or `trace freeze brave`, so presets and actions can be bound to keys.

## For development work

19. ✅ **Built** (Projects → Find build files) — **Developer junk cleaner (M).** Per project: size of `node_modules`, `target/`, virtual environments, plus Docker images, volumes and build cache, with one-click clearing.
20. ✅ **Built** (Control → Apps & Processes → Listening Ports) — **Dev server and port manager (M).** Every listening port with the project it belongs to, plus open in browser, stop, and conflict warnings.
21. ✅ **Built** (Projects → Work mode… on each project: compose containers plus a dev command) — **Project work mode (L).** Opening a project starts its containers, database and dev server; closing it stops them and frees the RAM.
22. ✅ **Built** (Control → Apps & Processes → Run With Restrictions (no network, no file changes, memory and CPU caps)) — **Run restricted (M).** Launch any program with no network, a RAM or CPU cap, or a read-only home folder. Useful for untrusted tools.

## Hardware and devices

23. ✅ **Built** (capacity history on Health; “Charge to full this once” appears there when a charge limit is set — or planned to be full by a time) — **Battery care (M).** A capacity-over-months chart, and "charge to 100% by 8 am tomorrow" so a charge limit doesn't catch you before travel.
24. ✅ **Built** (Hardware → Displays → Saved layouts) — **Display layouts (M).** Save docked, undocked and presentation layouts and apply them automatically when a monitor is connected.
25. ✅ **Built** (Health → Microphone & Camera (live view and a history of each use)) — **Mic and camera watch (M).** Shows which app is using the microphone or camera now, with a history of who used them and when.
26. ✅ **Built** (Control → Packages → Before You Update; takes a Timeshift or Snapper snapshot first when one is installed) — **Update assistant (L).** Pending updates with warnings (kernel or GPU driver means a reboot; distribution news that needs manual steps), a snapshot first where possible, then what needs restarting afterwards. Also helps merge leftover `.pacnew` config files.

## Safety

27. ✅ **Built** (Health → Security Check-up (fix buttons for the firewall and SSH; known-vulnerable packages checked against the Arch security tracker)) — **Security check-up (M).** One page: ports reachable from the network, SSH weaknesses, failed login attempts, packages with known vulnerabilities, disk encryption, firewall. Each item gets a fix button.
28. ✅ **Built** (Logs → At The Machine, Last 7 Days; optional phone alert through KDE Connect under Control → Automation → Reminders & Small Things) — **While-you-were-away log (S).** USB devices plugged in, failed unlocks and sudo attempts, lid opened. Optional alert to a phone through KDE Connect *(leaves the machine)*.

## Everyday shortcuts

29. ✅ **Built** (☆ on any setting in Tweaks; shown on Overview) — **Pinned switches (S).** Pin any setting from Tweaks (keep awake, blur, mic mute, a VPN) to the Overview page and the tray menu as a one-click switch.
30. ✅ **Built** (Control → Power & Memory → Timers) — **Timers (S).** "Sleep in 45 minutes", "shut down when this download finishes", "keep awake for 2 hours".
31. ✅ **Built** (Quick Set-up card on Overview until answered or dismissed) — **First-run tune-up (S).** Five questions (mostly plugged in? gaming? development? battery or speed?) and Trace applies the presets and suggestions that fit.
32. ✅ **Built** (mini window from the tray menu or `trace mini`; “Choose and arrange these cards” at the bottom of Overview) — **Mini monitor and your own Overview (M).** A small always-on-top window showing CPU, RAM, temperature and network; plus choosing and reordering the cards on Overview.

## Internet

33. ✅ **Built** (Control → Network → Connection Doctor and Internet Speed) — **Connection doctor and speed history (M).** Tests the router, the internet and DNS separately to say where a problem is, and keeps a chart of speed and delay per network over time.
34. ✅ **Built** (Control → Network → Saved Wi-Fi Networks (password, QR, auto-join, limited data, forget) and Wi-Fi Around You (channels); signal over the last day) — Signal strength over time, how crowded each channel is, and saved networks with show-password, share by QR code, forget and priority.
35. ✅ **Built** (Control → Network → Data Used This Month, and “Limited data” on each saved Wi-Fi network) — **Data budget and hotspot mode (S).** A monthly data allowance per network with a warning at 80%, and on a phone hotspot it pauses updates and other heavy background traffic. Builds on the data-used record that already exists.
36. ✅ **Built** (Control → Network → Who Each App Is Talking To; “Whose is it?” gives the name and the country the address is registered in) — **Who is my computer talking to? (L).** Per app: which sites and countries it contacted, with new or unusual ones flagged.

## Files and backup

37. ✅ **Built** (Control → Packages → Duplicates & Forgotten Files) — **Duplicates and forgotten files (M).** Duplicate files, the largest files, things not opened in a year and old downloads, removed to the trash rather than deleted.
38. ✅ **Built** (Control → Packages → Backups; restore is by opening a snapshot and copying out) — **Backups with restore (L).** Pick folders and a destination (external drive or another machine), a schedule, a check that the backup is readable, and a browser to restore single files.
39. ✅ **Built** (Control → Automation → Trace Itself → Move to another machine) — **Move to a new machine (S).** One file holding everything Trace manages — kept settings, presets, rules, profiles, schedules — that can be loaded on a fresh install.

## Gaming and graphics

40. ✅ **Built** (Control → Automation → Game Mode; each game can have its own preset; the overlay is MangoHud's launch option, not automated) — **Per-game profiles (M).** Notices a game starting (Steam, Proton, Lutris), applies the Gaming preset and that game's own options, shows a frame-rate overlay if wanted, and puts everything back afterwards.
41. ✅ **Built** (Hardware → Graphics Cards) — **Graphics use per app (M).** Which app is using how much GPU and video memory, which GPU each app runs on, and a "start this on the dedicated GPU" option.

## Privacy and wellbeing

42. ✅ **Built** (Control → Packages → Privacy Cleaner (recent files, browser caches, clipboard; thumbnails not included)) — **Privacy cleaner (M).** Recent-file lists, thumbnails, browser caches and clipboard history, with a preview of exactly what will be removed.
43. ✅ **Built** (Control → Automation → Reminders & Small Things; nudges only, nothing is blocked) — **Breaks and app time limits (S).** A reminder after a long stretch at the keyboard, and optional daily limits per app with a gentle nudge. Builds on the screen-time record.

## Explain and ask

44. ✅ **Built** (Control → Apps & Processes → What Is This?) — **Explain this (M).** On any process, service, log error or setting: what it is, whether it is safe to stop, and what usually causes it. From a built-in reference for common ones.
45. ~~**Ask Trace (L).** Questions in plain language over your own logbook: "why was it slow yesterday at 3 pm?", "what changed since last week?". Can use a model running on the machine (Ollama) so nothing leaves it; a hosted model would be optional *(leaves the machine)*.~~ *(cut: no AI features, by decision)*

## Power user

46. ✅ **Built** (Control → Services → Scheduled Jobs) — **Scheduled jobs overview (S).** Every timer and cron job on the system in one list: when it last ran, how long it took, when it runs next, with run-now and disable.
47. ✅ **Built** (Control → Automation → Memory Allowance For A Group; memory and processor) — **Budgets for groups of apps (M).** Limits for a whole group rather than one app: "browsers get at most 6 GB in total", "builds get half the CPU on battery".
48. ✅ **Built** (profile step “Run my own command”, usable from any schedule or situation trigger) — **Your own hooks (M).** Run a script of yours on any Trace event: an app started, battery under 20%, the VPN dropped, a disk passed 90%.

## Hardware and display ideas added later

49. **Fan speed and curves (M) — not part of this repository.** Setting a fan speed means talking to one laptop model's embedded controller directly, so it is not something to ship for every machine. Generic switches (full speed on or off where the kernel offers it, and temperature rules) are built.
50. ✅ **Built** (Hardware → Temperature Rules) — **Thermal rules (M).** "Above 85°C for a minute → quiet profile / cap CPU at 70%; below 70°C → back." Needs a way to act without a password prompt each time (see 53).
51. ✅ **Built** (Hardware → Cooling Test, optionally loading the graphics chip too) — **Stress test and cooling check (M).** A timed CPU or GPU load with a temperature and throttling curve, to judge paste, dust and fan health, and to compare before and after cleaning.
52. ✅ **Built** (Hardware → Colour Per Monitor: colour mode, saturation, brightness, and a colour profile file) — **Per-monitor colour (M).** Colour profiles (ICC), gamma and saturation per display, and HDR on/off where the panel supports it.
53. ✅ **Built** (Tweaks → Trusted Helper, with its own log) — **Trusted helper (L).** A small root-owned helper that only accepts the catalogue's own settings and values, so presets, thermal rules and schedules can change root-level settings without a password prompt each time. Opt-in.
54. ✅ **Built** (Tweaks → Laptop → Keyboard backlight; idle timeout under Control → Automation → Reminders & Small Things; this laptop has no per-zone lighting) — **Keyboard and lighting (S).** Backlight timeout, per-zone RGB where the laptop exposes it, and "light up on key press only".
55. ✅ **Built** (Hardware → Ports & Connections, with wake and power-saving switches per device) — **USB power per port (S).** Which ports wake the laptop, which keep charging while it sleeps, and per-device power saving.
56. ✅ **Built** (Health → Drives: self-test and wear over time; trim is in Weekly Maintenance; Erase A Drive for unused solid-state drives only) — Run a SMART self-test, see wear over time, secure-erase and trim on demand.
57. ✅ **Built** (Control → Devices → Sound Per App, with a noise-filtered microphone and an equaliser with tone presets) — **Audio hardware (M).** Per-app volume and output, an equaliser preset per device, and microphone noise suppression on/off.
58. ✅ **Built** (through monitor triggers: a profile can now set the sound output, power profile, layout and start apps) — **Dock memory (S).** Remember per dock or monitor: layout, audio output, power profile and which apps to start.
