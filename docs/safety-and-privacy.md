# Safety and privacy

## Administrator rights

Trace runs as you. Nothing runs as administrator unless an action needs it, and then it goes through the desktop's own password prompt (polkit). Several changes made together share one prompt.

Values that reach an administrator command are checked first: numbers must be numbers in range, names must come from lists Trace built itself. Free text you type is never passed to an administrator shell.

### Things that install something on the system

Each is optional, asks for your password once, and can be removed from the same place.

| What | Why | Where to remove it |
|---|---|---|
| Boot script for kept settings | Re-applies settings you chose to keep that reset at boot | Un-keep the settings on the Tweaks page |
| Trusted helper | Lets schedules and rules change catalogue settings without a prompt | Tweaks → Trusted Helper |
| Weekly maintenance job | Runs maintenance by itself while on the charger | Control → Packages → Weekly Maintenance |

The trusted helper accepts only settings and values from the catalogue and logs every change it makes.

## Changes are reversible

- Every setting change is recorded with its previous value and can be undone.
- Files are moved to the trash, not deleted.
- Display and colour changes last for the session; reloading the desktop restores its own configuration.
- Frozen apps are woken if Trace quits or restarts.

The exceptions are named as such in the app: clearing caches and history, deleting a backup, forgetting a Wi-Fi network, and erasing a drive. Erasing a drive requires typing the drive's name, refuses any drive with something mounted, and always refuses the drive the system runs from.

## What is stored

Everything Trace records stays in `~/.local/share/trace/` on your machine: resource samples, the logbook, which apps were in front and for how long, which addresses each app connected to, when the microphone and camera were used, and your settings.

Wi-Fi passwords are read from NetworkManager only when you ask to see one or make a QR code, and are never written to Trace's logs.

## What leaves the machine

Nothing, unless you use one of these:

| Feature | What it contacts |
|---|---|
| Update banner | This project's release list on GitHub, on launch. Skipped on a network marked as limited data. |
| VPN Gate | The public VPN Gate server list, when you open it. |
| Public address check | Cloudflare (1.1.1.1), when you press it, and once after connecting a VPN if you have used it before. |
| Speed test | A 25 MB test file from Cloudflare, when you press it. |
| Before You Update | Your distribution's mirrors and news feed, when you press it. |
| Vulnerable packages | The Arch security tracker's public list, when you press it. Nothing about your system is sent. |
| Whose is it? | A name lookup and a registry lookup for that one address, when you press it. |
| Phone alerts | Your own paired phone through KDE Connect. Off by default. |
| ntfy alerts | The ntfy server and topic you enter, with each alert's title and text. Off until you set it. |

There are no accounts, no telemetry and no AI features.
