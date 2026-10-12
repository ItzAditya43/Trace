# Tweaks

A catalogue of about 260 system settings. Only the ones your machine actually has are shown, typically over 200 on a laptop. Each has a plain explanation of what it does and when you would change it.

## What is covered

Power and CPU, laptop and battery, memory and swap, disks, network, desktop effects and windows (Hyprland, GTK), mouse, keyboard and touchpad, audio, privacy and security, background services, developer limits, clock and logs.

Deliberately left out: anything that cannot be undone or could stop the machine booting, such as the bootloader, the kernel command line, disk layout, sudo rules and one-way kernel lock-downs.

## Changing settings

Change as many settings as you like; they collect in a bar at the bottom. **Apply** then:

1. checks every value against what the setting accepts,
2. applies everything that needs administrator rights with **one** password prompt,
3. reads each setting back and reports honestly if the system ignored it,
4. records the change so it can be undone.

Every setting can be **undone**, or **reset to the value Trace first saw** on this machine.

## Keeping settings

Most settings last until the next restart. Ticking **keep after restart** on a setting makes it permanent:

- settings the system stores itself are simply left in place,
- settings that reset at boot are re-applied about 15 seconds after boot by a small system service Trace installs (`/etc/trace/tweaks.sh`),
- settings that belong to your desktop session are re-applied when Trace starts.

## Presets

Battery saver, Balanced, Performance, Gaming, Quiet and cool, zram tuning, Lightweight desktop, Security hardening and Developer workstation. A preset lists exactly what it would change before you apply it. **Save current as preset** stores your own.

## Suggestions

A short list worked out from this machine's own state, for example "you run zram and zswap together" or "the journal is using 4 GB".

## Pinned switches

Star any setting to put it on the Overview page as a one-click switch.

## Did that setting help?

Take a measurement, change a setting, do the same thing on the machine again, and take another. Trace compares processor use, temperature, battery draw and time spent waiting on memory and disk.

## Trusted helper

Presets applied by a schedule or a temperature rule cannot stop to ask for a password. The trusted helper is an optional, small administrator-owned script that accepts **only** settings and values from the catalogue. With it on, those changes happen without a prompt. It keeps its own log of everything it does, shown on the Tweaks page. It is off until you switch it on, and one click removes it.
