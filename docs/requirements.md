# Requirements

Trace needs Linux, systemd and WebKitGTK 4.1. Everything else is optional: a feature whose tool is missing is hidden or tells you what to install.

| Feature | Uses |
|---|---|
| Password prompts for administrator actions | polkit (`pkexec`) and a running polkit agent |
| Tray icon | a bar with StatusNotifierItem support |
| Network, VPNs, Wi-Fi | NetworkManager (`nmcli`); OpenVPN or WireGuard plugins for those VPN types |
| VPN kill switch, per-app routing | nftables, and the `nft_socket` kernel module |
| Firewall section | firewalld |
| Window, workspace, display and colour control | Hyprland; display arrangement and colour need its Lua configuration |
| External monitor brightness | `ddcutil` |
| Sound per app, noise filter, equaliser | PipeWire with `pipewire-pulse` (`pactl`) |
| Drive health and self-test | `smartctl` |
| NVIDIA details | `nvidia-smi` |
| Packages, update briefing | `pacman` with `pacman-contrib` (`checkupdates`, `paccache`), or `apt` for the basics |
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
