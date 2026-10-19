# Requirements

Trace needs Linux, systemd and WebKitGTK 4.1. Everything else is optional: a feature whose tool is missing is hidden or tells you what to install. The **What Works On This Machine** card at the bottom of the Overview lists exactly which features are available on your machine and what the rest need.

## Desktops

| Desktop | What works |
|---|---|
| Hyprland | everything |
| KDE Plasma, GNOME, Xfce, Cinnamon, Sway, others | everything except window and workspace control, monitor arrangement and colour, and the click-through overlay (it opens as an ordinary window). Those cards are hidden. |

Screen time per app and sleeping unused apps need to know which window is in front: built in on Hyprland and Sway, through `xprop` on any X11 session, through `kdotool` on KDE Wayland. GNOME on Wayland doesn't let other programs ask.

## Tools

| Feature | Uses |
|---|---|
| Password prompts for administrator actions | polkit (`pkexec`) and a running polkit agent |
| Tray icon | a bar with StatusNotifierItem support |
| Network, VPNs, Wi-Fi | NetworkManager (`nmcli`); OpenVPN or WireGuard plugins for those VPN types |
| VPN kill switch, per-app routing | nftables, and the `nft_socket` kernel module |
| Firewall section | firewalld or ufw |
| Window, workspace, display and colour control | Hyprland; display arrangement and colour need its Lua configuration |
| Stats overlay staying on top and click-through | Hyprland with its Lua configuration |
| Frame rate in the overlay | MangoHud, added to the game's launch options |
| External monitor brightness | `ddcutil` |
| Sound per app, noise filter, equaliser | PipeWire with `pipewire-pulse` (`pactl`) |
| Drive health and self-test | `smartctl` |
| NVIDIA details | `nvidia-smi` |
| Package updates, unneeded packages, cache cleanup | pacman (with `pacman-contrib`), apt, dnf or zypper |
| Update briefing with distribution news | Arch-based system |
| Vulnerable-package check | Arch-based system (`vercmp`) |
| Containers | Docker |
| Backups | `rsync` |
| Moving files to the trash | `gio` |
| Run with no file changes | `bubblewrap` (`bwrap`) |
| Wi-Fi QR codes | `qrencode` |
| Country of an address | `whois` |
| Phone alerts | KDE Connect (`kdeconnect-cli`) with a paired phone |
| Keyboard light timeout | `brightnessctl` |
| Clipboard history | `cliphist` |
| Snapshot before updating | Timeshift or Snapper |

On Arch Linux most of these come from:

```bash
sudo pacman -S --needed networkmanager nftables smartmontools ddcutil pacman-contrib \
    rsync bubblewrap qrencode whois brightnessctl cliphist
```
