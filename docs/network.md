# Network

Everything here is under **Control → Network** and uses NetworkManager.

## VPNs

- Connect, disconnect, rename, delete, and set a VPN to connect by itself.
- **Add from a file**: OpenVPN and WireGuard configuration files.
- **Add from VPN Gate**: pick a server from the public VPN Gate list, or connect to the fastest one in a country.
- **Public address check**: what the internet sees you as.
- **Kill switch**: blocks all traffic if the VPN drops.
- **Only some apps through the VPN**, and starting a single program inside the VPN.

The kill switch and per-app routing use nftables and need a kernel with the `nft_socket` module.

## Connection Doctor

Tests the link, the router, the internet and name lookups in turn and says where the problem is: your Wi-Fi, your router, or your provider.

## Internet Speed

A download test with a history per network. Each test downloads 25 MB and Trace never runs it by itself.

## Data Used This Month

Data used per network interface, with an optional monthly allowance and a warning at 80 percent.

## Wi-Fi

- **Saved Wi-Fi Networks**: show the password, a QR code a phone can scan to join, join automatically or not, mark as *limited data*, forget.
- **Wi-Fi Around You**: how crowded each channel is, and whether another channel would be better.
- **Wi-Fi Signal, Last 24 Hours**.

On a network marked as limited data Trace skips its own update check, and apps that respect the mark hold back downloads.

## Who Each App Is Talking To

Which addresses each app has open connections to. Trace notes these once a minute and marks addresses an app had not been seen talking to before today. *Whose is it?* looks up the name behind an address and the country it is registered in.

## Also here

Bluetooth and wireless switches, firewall ports, bandwidth limits and per-app network blocking.
