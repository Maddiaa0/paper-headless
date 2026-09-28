#!/usr/bin/env bash
# paper-headless installer for Debian/Ubuntu servers.
#
#   curl -fsSL https://raw.githubusercontent.com/Maddiaa0/paper-headless/main/install.sh | bash
#   curl -fsSL ... | bash -s -- --yes          # answer yes to every prompt
#   curl -fsSL ... | bash -s -- --from-source  # build with cargo instead of downloading a release
#
# What it does, asking before each step that touches the system:
#   1. installs the paper-headless binary into ~/.local/bin (release installer, checksum-verified)
#   2. installs Xvfb + dbus + xdg-utils from the distro repos (needed to run Paper without a screen)
#   3. adds Paper's apt repository and installs Paper Desktop (skipped if already installed)
#   4. installs and starts the background service, then registers the MCP endpoint with
#      Claude Code and Codex if they are on PATH
set -euo pipefail

REPO=${PAPER_HEADLESS_REPO:-Maddiaa0/paper-headless}
VERSION=${PAPER_HEADLESS_VERSION:-latest}
BIN_DIR=${PAPER_HEADLESS_BIN_DIR:-$HOME/.local/bin}
TOOLCHAIN=nightly-2026-06-21   # keep in sync with rust-toolchain.toml
YES=0
FROM_SOURCE=0

for arg in "$@"; do
  case "$arg" in
    -y|--yes) YES=1 ;;
    --from-source) FROM_SOURCE=1 ;;
    -h|--help) echo "usage: install.sh [--yes] [--from-source]"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

ask() {  # ask "question" -> 0 for yes
  if [ "$YES" = 1 ]; then return 0; fi
  if [ ! -r /dev/tty ]; then
    warn "no terminal to ask '$1'; skipping (re-run with --yes to accept every step)"
    return 1
  fi
  local reply
  read -r -p "$1 [y/N] " reply </dev/tty
  [[ "$reply" =~ ^[Yy] ]]
}

[ "$(uname -s)" = Linux ] || die "paper-headless only runs on Linux (Paper Desktop has native macOS and Windows apps)"

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  command -v sudo >/dev/null || die "not root and sudo is missing; re-run as root"
  SUDO=sudo
fi
HAVE_APT=0; command -v apt-get >/dev/null && HAVE_APT=1

# ---------------------------------------------------------------- 1. binary
install_from_release() {
  local url="https://github.com/$REPO/releases/latest/download/paper-headless-installer.sh"
  [ "$VERSION" = latest ] || url="https://github.com/$REPO/releases/download/$VERSION/paper-headless-installer.sh"
  curl --proto '=https' --tlsv1.2 -LsSf "$url" | PAPER_HEADLESS_INSTALL_DIR="$BIN_DIR" sh
}

install_from_source() {
  command -v rustup >/dev/null || die "rustup is required to build from source: https://rustup.rs"
  say "building paper-headless with cargo ($TOOLCHAIN)"
  rustup toolchain install "$TOOLCHAIN" --profile minimal >/dev/null
  cargo "+$TOOLCHAIN" install --git "https://github.com/$REPO" --locked --root "${BIN_DIR%/bin}" paper-headless
}

say "installing paper-headless into $BIN_DIR"
if [ "$FROM_SOURCE" = 1 ] || ! install_from_release; then
  [ "$FROM_SOURCE" = 1 ] || warn "no release available; building from source"
  install_from_source
fi
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) warn "$BIN_DIR is not on your PATH; add: export PATH=\"$BIN_DIR:\$PATH\"" ;; esac
PH="$BIN_DIR/paper-headless"
"$PH" --version

# ------------------------------------------------------- 2. distro packages
if [ "$HAVE_APT" = 1 ]; then
  missing=()
  command -v Xvfb >/dev/null        || missing+=(xvfb)
  command -v dbus-daemon >/dev/null || missing+=(dbus-daemon)
  command -v xdg-open >/dev/null    || missing+=(xdg-utils)
  if [ "${#missing[@]}" -gt 0 ]; then
    if ask "Install ${missing[*]} with apt-get (needed to run Paper without a display)?"; then
      $SUDO apt-get update -qq
      DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq "${missing[@]}"
    else
      warn "skipped; paper-headless serve will fail until ${missing[*]} are installed"
    fi
  fi
  if ! command -v x11vnc >/dev/null && ask "Also install x11vnc to view the Paper window over an SSH tunnel (optional)?"; then
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq x11vnc
  fi
else
  warn "no apt-get found; install Xvfb, dbus and xdg-utils with your package manager"
fi

# --------------------------------------------------------- 3. Paper Desktop
PAPER_BIN=${PAPER_DESKTOP_BIN:-/opt/Paper/paper-desktop}
if [ ! -x "$PAPER_BIN" ] && ! command -v paper-desktop >/dev/null; then
  if [ "$HAVE_APT" = 1 ] && ask "Add Paper's apt repository (download.paper.design) and install Paper Desktop?"; then
    $SUDO tee /etc/apt/sources.list.d/paper-desktop.sources >/dev/null <<'SOURCES'
Types: deb
URIs: https://download.paper.design/apt
Suites: stable
Components: main
Architectures: arm64 amd64
Signed-By:
 -----BEGIN PGP PUBLIC KEY BLOCK-----
 
 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
 72WkWd8QoDhXKtQ=
 =RyPW
 -----END PGP PUBLIC KEY BLOCK-----
SOURCES
    $SUDO apt-get update -qq
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq paper
  else
    warn "Paper Desktop is not installed; get the Linux .deb from https://paper.design/download and install it, then run: paper-headless install"
  fi
fi

# ---------------------------------------------------------------- 4. service
if ask "Install and start the paper-headless background service now?"; then
  "$PH" install
  echo
  say "done. Sign in once with:  paper-headless login"
else
  echo
  say "done. When ready:  paper-headless install && paper-headless login"
fi
