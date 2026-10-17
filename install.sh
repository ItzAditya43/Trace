#!/bin/sh
# Installs Trace on any x86_64 Linux distribution.
#
#   curl -fsSL https://raw.githubusercontent.com/ItzAditya43/Trace/main/install.sh | sh
#
# Debian, Ubuntu and their relatives get the .deb, Fedora and its relatives the
# .rpm. Arch, openSUSE and their relatives get the two libraries Trace needs
# (WebKitGTK and GTK) from their own repositories and the program in /usr/local.
# These ask for your password once. Anything else, or with --user, gets the
# AppImage unpacked into your home folder: no password, no FUSE needed.
#
# Options (after `sh -s --` when piping):
#   --user        install into your home folder only, whatever the distribution
#   --version X   install version X instead of the latest (e.g. 1.0.1)
#   --uninstall   remove Trace (your recorded history in ~/.local/share/trace is kept)
set -eu

REPO="ItzAditya43/Trace"
MODE=auto
WANT_VERSION=latest

while [ $# -gt 0 ]; do
    case "$1" in
        --user) MODE=user ;;
        --version) shift; WANT_VERSION="${1:?--version needs a number, e.g. 1.0.1}" ;;
        --uninstall) MODE=uninstall ;;
        -h|--help) echo "Usage: install.sh [--user] [--version X] [--uninstall]   https://github.com/$REPO"; exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

say() { printf '\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA/trace-app"
BIN_DIR="$HOME/.local/bin"
DESKTOP_FILE="$DATA/applications/trace.desktop"
ICON_FILE="$DATA/icons/hicolor/128x128/apps/trace.png"
# Where the Arch / openSUSE route puts things, out of the package manager's way.
LOCAL_BIN=/usr/local/bin/trace
LOCAL_DESKTOP=/usr/local/share/applications/trace.desktop
LOCAL_ICON=/usr/local/share/icons/hicolor/128x128/apps/trace.png

# Root needs nothing; anyone else needs sudo or doas for the system routes.
if [ "$(id -u)" -eq 0 ]; then
    AS_ROOT=""
elif have sudo; then
    AS_ROOT="sudo"
elif have doas; then
    AS_ROOT="doas"
else
    AS_ROOT="none"
fi

desktop_entry() { # $1 = the command to start Trace with, $2 = icon
    printf '%s\n' "[Desktop Entry]" "Type=Application" "Name=Trace" "GenericName=Machine Control" \
        "Comment=Observe, understand, and control your Linux machine" "Exec=$1" "Icon=$2" "Terminal=false" \
        "Categories=System;Monitor;" "StartupWMClass=trace" "X-Installed-By=trace-install"
}

# ---- uninstall ----
if [ "$MODE" = uninstall ]; then
    removed=no
    if [ -d "$APP_DIR" ]; then
        rm -rf "$APP_DIR" "$ICON_FILE"
        grep -q "trace-app" "$BIN_DIR/trace" 2>/dev/null && rm -f "$BIN_DIR/trace"
        grep -q "X-Installed-By=trace-install" "$DESKTOP_FILE" 2>/dev/null && rm -f "$DESKTOP_FILE"
        removed=yes
    fi
    if grep -q "X-Installed-By=trace-install" "$LOCAL_DESKTOP" 2>/dev/null; then
        [ "$AS_ROOT" = none ] && fail "Removing Trace from /usr/local needs sudo."
        $AS_ROOT rm -f "$LOCAL_BIN" "$LOCAL_DESKTOP" "$LOCAL_ICON"
        removed=yes
    fi
    if have dpkg && dpkg -s trace >/dev/null 2>&1; then
        [ "$AS_ROOT" = none ] && fail "Removing the system package needs sudo."
        $AS_ROOT apt-get remove -y trace || $AS_ROOT dpkg -r trace
        removed=yes
    elif have rpm && rpm -q trace >/dev/null 2>&1; then
        [ "$AS_ROOT" = none ] && fail "Removing the system package needs sudo."
        if have dnf; then $AS_ROOT dnf remove -y trace; else $AS_ROOT rpm -e trace; fi
        removed=yes
    fi
    if [ "$removed" = yes ]; then
        say "Trace removed. Your history and settings are still in ~/.local/share/trace; delete that folder to remove them too."
    else
        say "Trace isn't installed."
    fi
    exit 0
fi

# ---- can it run here? ----
[ "$(uname -s)" = Linux ] || fail "Trace is for Linux."
case "$(uname -m)" in
    x86_64|amd64) ;;
    *) fail "Trace is only built for 64-bit Intel/AMD processors (x86_64); this is $(uname -m). It can be built from source: https://github.com/$REPO" ;;
esac
if [ -e /lib/ld-musl-x86_64.so.1 ] && ! [ -e /lib64/ld-linux-x86-64.so.2 ]; then
    fail "This system uses musl (Alpine and similar). Trace's builds need glibc; build it from source instead."
fi

ID=""
ID_LIKE=""
# shellcheck disable=SC1091
[ -r /etc/os-release ] && . /etc/os-release
DISTRO=" $ID $ID_LIKE "

if [ "$ID" = nixos ]; then
    fail "On NixOS, download the AppImage from https://github.com/$REPO/releases and run it with appimage-run."
fi

if have curl; then
    fetch() { curl -fsSL --retry 3 "$1"; }
    download() { curl -fL --retry 3 --progress-bar -o "$2" "$1"; }
elif have wget; then
    fetch() { wget -qO- "$1"; }
    download() { wget -q -O "$2" "$1"; }
else
    fail "Needs curl or wget to download Trace."
fi

# ---- which release ----
if [ "$WANT_VERSION" = latest ]; then
    API="https://api.github.com/repos/$REPO/releases/latest"
else
    API="https://api.github.com/repos/$REPO/releases/tags/v${WANT_VERSION#v}"
fi
RELEASE=$(fetch "$API") || fail "Couldn't reach GitHub to find the release ($API)."
asset() { printf '%s' "$RELEASE" | grep -o "\"browser_download_url\": *\"[^\"]*$1\"" | head -n 1 | sed 's/.*"\(https[^"]*\)"/\1/'; }
TAG=$(printf '%s' "$RELEASE" | grep -o '"tag_name": *"[^"]*"' | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')
[ -n "$TAG" ] || fail "That release wasn't found."

# ---- which route ----
KIND=appimage
if [ "$MODE" = auto ] && [ "$AS_ROOT" != none ]; then
    case "$DISTRO" in
        *" debian "*|*" ubuntu "*) have apt-get && KIND=deb ;;
        *" fedora "*|*" rhel "*|*" centos "*) have dnf && KIND=rpm ;;
        *" arch "*) have pacman && KIND=pacman ;;
        *" suse "*|*" opensuse "*) have zypper && KIND=zypper ;;
    esac
fi
case "$KIND" in
    deb) URL=$(asset '_amd64\.deb') ;;
    rpm) URL=$(asset '\.x86_64\.rpm') ;;
    *) URL=$(asset '\.AppImage') ;;
esac
if [ -z "$URL" ] && { [ "$KIND" = deb ] || [ "$KIND" = rpm ]; }; then
    KIND=appimage
    URL=$(asset '\.AppImage')
fi
[ -n "$URL" ] || fail "Release $TAG has no file for this system."

WORK=$(mktemp -d "${TMPDIR:-/tmp}/trace-install.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
FILE="$WORK/$(basename "$URL")"

say "Installing Trace $TAG …"
download "$URL" "$FILE" || fail "Download failed: $URL"

unpack_appimage() {
    chmod +x "$FILE"
    (cd "$WORK" && "$FILE" --appimage-extract >/dev/null) || fail "Couldn't unpack the AppImage."
}

find_icon() { # $1 = unpacked AppImage
    for candidate in "$1"/usr/share/icons/hicolor/128x128/apps/trace.png "$1/trace.png" "$1/.DirIcon"; do
        [ -f "$candidate" ] && { printf '%s' "$candidate"; return 0; }
    done
    return 0
}

# The program from the AppImage, using the system's own WebKitGTK and GTK.
install_into_usr_local() {
    unpack_appimage
    root="$WORK/squashfs-root"
    icon=$(find_icon "$root")
    desktop_entry "$LOCAL_BIN" trace > "$WORK/trace.desktop"
    $AS_ROOT install -Dm755 "$root/usr/bin/trace" "$LOCAL_BIN"
    $AS_ROOT install -Dm644 "$WORK/trace.desktop" "$LOCAL_DESKTOP"
    if [ -n "$icon" ]; then $AS_ROOT install -Dm644 "$icon" "$LOCAL_ICON"; fi
    if have update-desktop-database; then $AS_ROOT update-desktop-database /usr/local/share/applications >/dev/null 2>&1 || true; fi
}

case "$KIND" in
    deb)
        # apt brings the libraries Trace needs from the distribution's own repositories.
        $AS_ROOT apt-get update -qq || true
        $AS_ROOT env DEBIAN_FRONTEND=noninteractive apt-get install -y "$FILE"
        ;;
    rpm)
        $AS_ROOT dnf install -y "$FILE"
        ;;
    pacman)
        $AS_ROOT pacman -S --needed --noconfirm webkit2gtk-4.1 gtk3
        install_into_usr_local
        ;;
    zypper)
        $AS_ROOT zypper -n --quiet install libwebkit2gtk-4_1-0 libgtk-3-0
        install_into_usr_local
        ;;
    appimage)
        # Unpacked once rather than mounted at every start, so FUSE isn't needed.
        unpack_appimage
        rm -rf "$APP_DIR"
        mkdir -p "$DATA" "$BIN_DIR" "$(dirname "$DESKTOP_FILE")" "$(dirname "$ICON_FILE")"
        mv "$WORK/squashfs-root" "$APP_DIR"
        printf '%s\n' "#!/bin/sh" "# Starts Trace (installed by install.sh into $APP_DIR)." "exec \"$APP_DIR/AppRun\" \"\$@\"" > "$BIN_DIR/trace"
        chmod +x "$BIN_DIR/trace"
        icon=$(find_icon "$APP_DIR")
        if [ -n "$icon" ]; then cp -L "$icon" "$ICON_FILE"; fi
        desktop_entry "$BIN_DIR/trace" "$ICON_FILE" > "$DESKTOP_FILE"
        if have update-desktop-database; then update-desktop-database "$(dirname "$DESKTOP_FILE")" >/dev/null 2>&1 || true; fi
        # The AppImage carries most of what it needs but expects a desktop's
        # basics (fonts, graphics) from the system. Say plainly if any are missing.
        if have ldd; then
            missing=$(LD_LIBRARY_PATH="$APP_DIR/usr/lib:$APP_DIR/usr/lib/x86_64-linux-gnu" ldd "$APP_DIR/usr/bin/trace" 2>/dev/null | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')
            if [ -n "$missing" ]; then
                echo "Warning: this system is missing libraries Trace needs: $missing"
                echo "Installing your distribution's WebKitGTK 4.1 and GTK 3 packages brings them in."
            fi
        fi
        ;;
esac

say "Trace $TAG is installed."
echo "Start it from your app menu, or run: trace"
if [ "$KIND" = appimage ]; then
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) echo "Note: $BIN_DIR is not on your PATH, so 'trace' won't be found in a terminal until you add it. The app menu entry works either way." ;;
    esac
fi
echo "Some features use extra tools when they are present (NetworkManager, PipeWire, smartctl, ddcutil…): https://github.com/$REPO/blob/main/docs/requirements.md"
echo "To update, run the same command again. To remove: curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sh -s -- --uninstall"
