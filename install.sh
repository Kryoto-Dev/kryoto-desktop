#!/usr/bin/env bash
# Kryoto Desktop for Linux, in one line:
#
#   curl -fsSL https://kryo.to/install.sh | bash
#
# or from a clone of this repository:
#
#   git clone https://github.com/Kryoto-Dev/kryoto-desktop && cd kryoto-desktop && ./install.sh
#
# Picks the package your system installs natively, so Kryoto Desktop uses the
# system's own WebKit and graphics drivers:
#
#   Ubuntu, Debian, Mint, Pop!_OS, elementary, Zorin ...   the .deb (apt)
#   Fedora, Nobara, Bazzite (layered), RHEL, openSUSE ...   the .rpm (dnf / zypper)
#   anything else (Arch, Void, NixOS, Steam Deck ...)       the AppImage, in ~/.local
#
# Options:
#   --appimage    install the AppImage even where a package would fit
#   --uninstall   remove what this script installed
#   --yes         do not ask before using sudo
#
# Updates come through Kryoto Desktop itself afterwards (Settings > About).

set -euo pipefail

BASE="${KRYOTO_DOWNLOAD_BASE:-https://kryo.to/desktop/download}"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/kryoto-desktop"
BIN_DIR="$HOME/.local/bin"
DESKTOP_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/applications/kryoto-desktop.desktop"
ICON_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/kryoto-desktop.png"

want_appimage=0
uninstall=0
assume_yes=0
for arg in "$@"; do
  case "$arg" in
    --appimage) want_appimage=1 ;;
    --uninstall) uninstall=1 ;;
    --yes|-y) assume_yes=1 ;;
    -h|--help) sed -n '2,24p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

bold=$'\033[1m'; dim=$'\033[2m'; reset=$'\033[0m'
[ -t 1 ] || { bold=''; dim=''; reset=''; }
say() { printf '%s\n' "${bold}K//${reset} $*"; }
fail() { printf '%s\n' "${bold}K//${reset} $*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || fail "This installer is for Linux. Windows: https://kryo.to/desktop"
case "$(uname -m)" in
  x86_64|amd64) ;;
  *) fail "Kryoto Desktop is built for 64-bit x86 PCs; this one is $(uname -m)." ;;
esac
command -v curl >/dev/null || fail "curl is needed. Install it with your package manager and run this again."

# Root, or sudo, for the package managers. Asked once, up front.
SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null; then SUDO="sudo"; elif command -v doas >/dev/null; then SUDO="doas"; fi
fi
need_root() {
  [ "$(id -u)" -eq 0 ] && return 0
  [ -n "$SUDO" ] || fail "Installing a package needs root, and neither sudo nor doas is here. Run again with --appimage to install for just you."
  if [ "$assume_yes" -eq 0 ] && [ -t 0 ]; then
    say "Installing the package needs your password (${SUDO})."
  fi
}

# ---- Which family is this? ---------------------------------------------------
family="other"
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  ids=" ${ID:-} ${ID_LIKE:-} "
  case "$ids" in
    *" debian "*|*" ubuntu "*) command -v apt-get >/dev/null && family="deb" ;;
  esac
  if [ "$family" = "other" ]; then
    case "$ids" in
      *" fedora "*|*" rhel "*|*" centos "*|*" nobara "*) command -v dnf >/dev/null && family="dnf" ;;
      *" suse "*|*" opensuse "*|*" opensuse-tumbleweed "*|*" sles "*) command -v zypper >/dev/null && family="zypper" ;;
    esac
  fi
  # Image-based systems (Silverblue, Bazzite, SteamOS) cannot take a package
  # the usual way: the AppImage installs for the user instead.
  if [ -e /run/ostree-booted ] || [ "${ID:-}" = "steamos" ]; then family="other"; fi
fi
[ "$want_appimage" -eq 1 ] && family="other"

# ---- Uninstall --------------------------------------------------------------
if [ "$uninstall" -eq 1 ]; then
  removed=0
  if command -v dpkg >/dev/null && dpkg -s kryoto-desktop >/dev/null 2>&1; then
    need_root; $SUDO apt-get remove -y kryoto-desktop && removed=1
  fi
  if command -v rpm >/dev/null && rpm -q kryoto-desktop >/dev/null 2>&1; then
    need_root
    if command -v dnf >/dev/null; then $SUDO dnf remove -y kryoto-desktop; else $SUDO zypper --non-interactive remove kryoto-desktop; fi
    removed=1
  fi
  if [ -e "$APP_DIR/Kryoto-Desktop.AppImage" ]; then
    rm -f "$APP_DIR/Kryoto-Desktop.AppImage" "$BIN_DIR/kryoto-desktop" "$DESKTOP_FILE" "$ICON_FILE"
    rmdir "$APP_DIR" 2>/dev/null || true
    removed=1
  fi
  if [ "$removed" -eq 1 ]; then
    say "Kryoto Desktop is uninstalled. Your library, settings and downloaded games are kept in ~/.local/share/to.kryo.desktop."
  else
    say "Kryoto Desktop is not installed by this script here."
  fi
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

download() {
  # $1 = platform on kryo.to (deb, rpm, linux), $2 = file to write
  say "Downloading Kryoto Desktop ${dim}(${1})${reset}"
  curl -fL --retry 3 --proto '=https' --tlsv1.2 --progress-bar -o "$2" "$BASE/$1" \
    || fail "The download did not finish. Check your connection and run this again."
  [ -s "$2" ] || fail "The download came back empty."
  # When there is no such file kryo.to sends its download page instead: make
  # sure this is the package, not a web page.
  magic="$(head -c 8 "$2" | od -An -tx1 | tr -d ' \n')"
  case "$1:$magic" in
    deb:213c617263683e*|rpm:edabeedb*|linux:7f454c46*) ;;
    *) fail "kryo.to has no ${1} download right now. Try again in a minute, or with --appimage." ;;
  esac
}

case "$family" in
  deb)
    download deb "$tmp/kryoto-desktop.deb"
    need_root
    # apt installs what WebKitGTK needs alongside it.
    $SUDO apt-get install -y "$tmp/kryoto-desktop.deb" || {
      $SUDO apt-get update && $SUDO apt-get install -y "$tmp/kryoto-desktop.deb"
    } || fail "apt could not install the package. Try again with --appimage."
    ;;
  dnf)
    download rpm "$tmp/kryoto-desktop.rpm"
    need_root
    $SUDO dnf install -y "$tmp/kryoto-desktop.rpm" || fail "dnf could not install the package. Try again with --appimage."
    ;;
  zypper)
    download rpm "$tmp/kryoto-desktop.rpm"
    need_root
    $SUDO zypper --non-interactive install --allow-unsigned-rpm "$tmp/kryoto-desktop.rpm" \
      || fail "zypper could not install the package. Try again with --appimage."
    ;;
  other)
    download linux "$tmp/Kryoto-Desktop.AppImage"
    mkdir -p "$APP_DIR" "$BIN_DIR" "$(dirname "$DESKTOP_FILE")" "$(dirname "$ICON_FILE")"
    install -m 755 "$tmp/Kryoto-Desktop.AppImage" "$APP_DIR/Kryoto-Desktop.AppImage"
    ln -sf "$APP_DIR/Kryoto-Desktop.AppImage" "$BIN_DIR/kryoto-desktop"
    # The icon, out of the AppImage itself.
    ( cd "$tmp" && "$APP_DIR/Kryoto-Desktop.AppImage" --appimage-extract '*.png' >/dev/null 2>&1 ) || true
    icon="$(ls "$tmp"/squashfs-root/*.png 2>/dev/null | head -1 || true)"
    [ -n "$icon" ] && install -m 644 "$icon" "$ICON_FILE"
    cat > "$DESKTOP_FILE" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Kryoto Desktop
Comment=The kryo.to store, your game library and downloads
Exec=$APP_DIR/Kryoto-Desktop.AppImage %u
Icon=kryoto-desktop
Terminal=false
Categories=Game;
MimeType=x-scheme-handler/kryoto;
StartupWMClass=kryoto-desktop
DESKTOP
    command -v update-desktop-database >/dev/null && update-desktop-database "$(dirname "$DESKTOP_FILE")" >/dev/null 2>&1 || true
    if ! ldconfig -p 2>/dev/null | grep -q 'libfuse.so.2'; then
      say "${dim}Note: AppImages need FUSE 2 (the package is usually called fuse2, fuse or libfuse2). Without it, start it with --appimage-extract-and-run.${reset}"
    fi
    case ":$PATH:" in *":$BIN_DIR:"*) ;; *) say "${dim}Tip: add $BIN_DIR to your PATH to start it with \`kryoto-desktop\`.${reset}" ;; esac
    ;;
esac

say "Kryoto Desktop is installed. Find it in your apps menu, or run ${bold}kryoto-desktop${reset}."
