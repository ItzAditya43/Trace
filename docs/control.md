# Control

The Control page is split into sections.

## Apps & Processes

- **Processes**: search, freeze, resume, kill, change priority, limit CPU and memory. *Tell me when it's done* sends a notification when a chosen process finishes.
- **Listening Ports**: every listening port, the program behind it and the folder it was started in, whether other machines can reach it, and a stop button.
- **Run With Restrictions**: start a program with no network, unable to change any file, or with a memory or CPU cap. For tools you do not fully trust.
- **Process Details**: one process's command, folder, user, threads and memory; the chain of processes that started it ("why is this running"); what it has started; its open files, network connections and environment (values that look like secrets are hidden).
- **What Is This?**: a plain explanation of a process or service name and whether it is safe to stop. Unknown names fall back to the description of the package that installed them.
- **Windows** and workspaces, and namespaces.

## Power & Memory

Lock, suspend, log out, restart, shut down; timers such as *sleep in 45 minutes* or *keep awake for 2 hours*; power profile, governor, which cores are online, fans, memory and swap tuning, brightness and volume, GPU tuning.

## Network

See [Network](network.md).

## Display & Devices

- **Displays** and night light.
- **Audio**: default output and input.
- **Sound Per App**: volume, mute and output for each app that is playing; a noise-filtered microphone; an equaliser with five tone presets that all sound is routed through.
- **USB Devices**: allow or block individual devices. Blocking your own keyboard or mouse locks you out until it is replugged.
- **Clipboard history**, when there is any.

## Services

systemd units and Docker containers with their logs and live stats, every timer and cron job with run-now and disable, autostart apps, and what each service costs at startup.

## Automation

See [Automation](automation.md).

## Packages & Cleanup

- **Packages**: pending updates, package cache, packages nothing needs any more.
- **Before You Update**: how many packages are waiting, which need a restart of the machine, recent distribution news that asks for manual steps, apps still running an old version, and settings files where the package shipped a newer version next to yours (compare, keep yours, or use the new one). Takes a Timeshift or Snapper snapshot first when one is installed.
- **Weekly Maintenance**: trims drives, keeps the two newest versions of each package, cuts the system log to 500 MB, removes old crash dumps, and clears Docker's untagged images and build cache (containers and volumes are left alone). Run it by hand, or let it run by itself each week after asking for your password once.
- **Cleanup**: trash, journal, crash dumps, thumbnails.
- **Privacy Cleaner**: recent-file lists, browser caches and clipboard history, with sizes shown before anything is cleared.
- **Duplicates & Forgotten Files**: identical files, the largest files, and big files untouched for a year in a folder you pick. Files go to the trash, never straight to deletion.
- **Backups**: dated copies of chosen folders on another drive. Each backup is complete and browsable, but unchanged files take no extra space. *Check it* reads a backup back and compares it with your files. To restore, open a backup and copy out what you need.
