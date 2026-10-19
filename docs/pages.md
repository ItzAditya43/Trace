# The pages

## Overview

The live state of the machine.

- **Tiles** for CPU, available memory, swap, stall time, hottest sensor, fullest disk, network and battery.
- **Machine Health**: a score from 0 to 100 with the top issues in plain words.
- **Why is it like this right now?** One click gives a short answer from what Trace already knows: which app, which power profile, whether it is charging, whether memory is stalling.
- **Pinned switches**: any setting you starred on the Tweaks page.
- Cards for cores, GPUs, temperatures, network, disks and top processes.

At the bottom, **What Works On This Machine** says which of Trace's features this machine supports and what the rest need, and **Choose and arrange these cards** hides cards and changes their order.

*Stall time* is the share of time programs were stuck waiting for the processor, memory or disk. It is the most direct measure of "the machine feels slow".

## Memory

- **Quick Rescue**: the heaviest apps with Freeze and Quit, for when memory has run out.
- **Possible Memory Leaks**: appears when an app's memory has only climbed for three hours or more.
- **Where Your RAM Is**: every byte of RAM by purpose (apps, compressed swap, file cache, kernel, graphics, shared, free).
- **Memory by App**: per-app RAM and swap, with shared pages divided fairly between the apps sharing them. Each app can be restarted, frozen, quit, or given a CPU or memory limit without a password.
- **Growth Over Time**: how each app's memory moved over the last hour to seven days.
- **Swap**: swap devices and how well compressed memory is doing.

## Activity

Two cards. **Drives Right Now** shows how busy each drive is. **Who Is Busy** lists which apps are reading and writing the disk, sending and receiving on the network, and waking the processor, measured over the last few seconds.

## Hardware

- **Sensors**: everything the machine reports, with a short trend.
- **Ports & Connections**: charger, USB devices with their link speed and power draw, video outputs, network sockets, audio jacks. Each USB device has switches for *may wake the machine* and *may power down when idle*.
- **Fans**: each fan's speed, with automatic and full speed where the kernel offers that switch.
- **Temperature Rules**: when the hottest sensor stays above a temperature for 30 seconds, change a setting; undo it once it has cooled 10°C below.
- **Cooling Test**: loads every core (and optionally the graphics chip) for 30 to 120 seconds and reports the peak temperature and how much the processor slowed itself. It stops by itself at 98°C.
- **CPU & Power**: power profile, firmware thermal profile, governor, energy preference, and which cores are online.
- **Graphics Cards**: which apps use which card and how much video memory, and whether the dedicated card is powered down.
- **Brightness** for built-in and external monitors, and **Wireless Devices** with the battery level of mice, keyboards and headsets that report one.
- **Displays**: resolution, refresh rate, scale, rotation, position, mirroring, adaptive sync, saved layouts that apply themselves when a monitor is connected, and a night-light schedule.
- **Colour Per Monitor**: colour mode, saturation, brightness of normal content, and a colour profile file.

## Health

- **Machine Health**: the score with every issue listed, and **Restart Needed?**, which says whether an update replaced the running kernel or graphics driver.
- **Drives**: SMART health, wear, temperature, a self-test button, and how wear has moved since Trace first read it.
- **Battery Over Time**: capacity history, recent sleeps and what each cost, and *Charge to full this once* or *have it full by* a time when a charge limit is set.
- **Battery Drain Per App**: watches for three seconds and splits the power draw between apps. It learns this machine's own idle draw from time spent on battery.
- **What Grew On The Disk**: compares folder sizes between two measurements. One is taken each week while on the charger.
- **Erase A Drive**: wipes an unused solid-state drive. The drive the system runs from is always refused.
- **Share A Health Report**: saves one self-contained page with the score, issues, hardware, failed services and recent errors to your Downloads folder, for when you ask someone for help.
- **Security Check-up**: firewall, programs listening on the network, SSH, disk encryption, Secure Boot, failed logins, and installed packages with known vulnerabilities. Fix buttons appear where Trace can fix something.
- **Microphone & Camera**: who is using them now and a history of each use.
- **Boot time**, failed services, out-of-memory kills and recent journal errors.

## History

Charts for CPU, memory, swap, GPU, time stalled on memory and on disk, and how full each disk is, over one hour to thirty days. Hover to inspect; warnings are marked along the bottom and listed under **What Happened**. Below are a storage breakdown and a disk usage explorer.

## Logs

Pick a day, week, month or year.

- Totals, averages and peaks for the period.
- Screen time per app, and apps that ran in the background.
- **Screen Time, Hour By Hour**: what was in front of you during each hour. Time only counts while you are there: five minutes without touching anything stops the clock, unless sound is playing.
- Crashes and out-of-memory kills, battery drain and cycles, data used per network, sleep and lid times, package changes, warnings, alerts Trace sent, and actions it took.
- **Games Played**: each game session's length, average frame rate and 1% low, for games started with the frame-rate launch option.
- **At The Machine, Last 7 Days**: USB devices plugged in, wrong passwords, the lid.

Any period exports as JSON.

## Investigate

- **What's slowing me down?** Looks at the last few minutes and names the cause.
- **Who's using this port?** Which program is listening on a port.

## Projects

Point it at the folder your projects live in. Each project shows its branch, uncommitted files, unpushed commits, size, languages and TODO count, with buttons to open it in an editor, terminal or file manager.

- **Find build files**: sizes of `node_modules`, `target`, virtual environments and similar, with one-click clearing.
- **Work mode**: starts the project's Docker Compose containers and a dev command of yours, shows the command's output, and stops both again.
