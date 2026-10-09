# Command line

When Trace is already running, running `trace` again with a command hands that command to the running copy. This makes every command usable from a terminal or a key binding.

| Command | What it does |
|---|---|
| `trace` | Shows the window. |
| `trace --hidden` | Starts in the tray without showing the window. |
| `trace page <name>` | Opens a page: `live`, `memory`, `activity`, `hardware`, `health`, `history`, `logs`, `diagnose`, `projects`, `tweaks`, `control`. |
| `trace page control:<section>` | Opens Control at a section: `apps`, `power`, `network`, `devices`, `services`, `automation`, `packages`. |
| `trace preset <id>` | Applies a settings preset, for example `trace preset gaming`. |
| `trace freeze <app>` | Freezes every process of an app. |
| `trace resume <app>` | Resumes it. |
| `trace quit <app>` | Asks it to quit. |
| `trace restart <app>` | Restarts it. |
| `trace mini` | Shows or hides the mini monitor. |
| `trace panic` | Opens the mini monitor and never closes it. Bind this to a key. |

## Key bindings

Hyprland, in a Lua configuration:

```lua
hl.bind("SUPER + SHIFT + Escape", hl.dsp.exec_cmd("trace panic"), { description = "Trace: rescue window" })
hl.bind("SUPER + G", hl.dsp.exec_cmd("trace preset gaming"))
```

Hyprland, in the classic configuration:

```
bind = SUPER SHIFT, Escape, exec, trace panic
bind = SUPER, G, exec, trace preset gaming
```
