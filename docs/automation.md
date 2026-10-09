# Automation

Everything here is under **Control → Automation** and only works while Trace is running, so switch on *Start at login* first.

## Profiles

A profile is an ordered list of steps that can be dry-run, applied and undone. Steps include freezing, resuming or quitting an app, setting the power profile, starting or stopping a service or container, applying a settings preset, choosing the sound output, and running a command of your own. Build one in the app or import a TOML file.

## Schedules and situations

Run a profile:

- at a time on chosen days,
- when an app starts,
- when the charger is plugged in or pulled,
- when the battery falls to a level,
- when a particular Wi-Fi network is joined,
- when a monitor is connected or disconnected.

Combined with the *run my own command* step, this is how you hook your own scripts to events.

## Memory rules

*If this app uses more than N MB, then notify me, freeze it, restart it or quit it.* Checked once a minute; each rule fires at most once every ten minutes.

## Allowance for a group of apps

One memory allowance, processor allowance, or both, shared by several apps: "browsers get at most 6 GB in total". Over the allowance Trace tells you, or acts: it freezes the heaviest app, or caps each app's processor share.

## Sleep apps you aren't using

Chosen apps are frozen after their window has gone unused for a while and wake the moment you click back.

## Focus time

Freezes chosen apps for a set time, then lets them run again. Frozen apps cannot use the processor, the network or send notifications. Trace holds back its own non-urgent alerts as well. The apps wake by themselves, including when Trace is quit or restarted.

## Game mode

When a Steam, Proton, Lutris or gamescope game starts, Trace applies a preset and puts every setting back when the game closes. Each game it has seen can be given its own preset.

## Temperature rules

On the Hardware page: above a temperature for a while, change a setting; below another, put it back.

## Reminders and small things

- A Monday summary of last week.
- A break suggestion after a chosen time at the screen.
- A nudge once an app has been in front for a chosen time today.
- Keyboard light off when idle, back at the next key press.
- An alert to a paired phone (KDE Connect) when a USB device is plugged in or a wrong password is entered while you are away.

These only tell you; nothing is closed or blocked.

## Alerts

Desktop notifications when something turns critical: memory nearly gone, a disk almost full, overheating, memory stalls. Each repeats at most once every thirty minutes. Every alert sent is listed under **Trace Itself**.

## Trace itself

Run in the tray, start at login, the mini monitor, and **Move to another machine**, which saves rules, schedules, profiles, presets, display layouts and kept settings to one file that can be loaded elsewhere.

## Action log

Every action Trace takes, by you or by a rule, is recorded with its result.
