#!/usr/bin/env bash
# Phaenna Universal All-In-One Installer (macOS & Linux)
# Installs: GPU terminals (Kitty, WezTerm), players (mpv, VLC, IINA on macOS),
#           yt-dlp, ffmpeg, Rust/Cargo toolchain, and Phaenna from source.
set -euo pipefail

REPO_URL="https://github.com/abdullah-binmadhi/Phaenna.git"
SRC_DIR="${PHAENNA_SRC_DIR:-$HOME/.local/share/phaenna-src}"

log() { printf "\n[phaenna-setup] %s\n" "$*"; }
warn() { printf "\n[phaenna-setup] WARNING: %s\n" "$*" >&2; }

ensure_cargo() {
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    log "Installing Rust toolchain via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
}

install_macos() {
  if ! command -v brew >/dev/null 2>&1; then
    log "Installing Homebrew..."
    NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    if [ -x /opt/homebrew/bin/brew ]; then
      eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [ -x /usr/local/bin/brew ]; then
      eval "$(/usr/local/bin/brew shellenv)"
    fi
  fi

  log "Installing CLI media stack (mpv, yt-dlp, ffmpeg, git)..."
  brew install mpv yt-dlp ffmpeg git

  log "Installing GPU terminals & GUI players (kitty, wezterm, iina, vlc)..."
  for cask in kitty wezterm iina vlc; do
    brew list --cask "$cask" >/dev/null 2>&1 || brew install --cask "$cask" || warn "Optional cask '$cask' skipped or already present."
  done
}

install_arch() {
  log "Detected Arch-based Linux (pacman)..."
  sudo pacman -S --needed --noconfirm kitty wezterm mpv vlc yt-dlp ffmpeg git base-devel
}

install_debian_ubuntu() {
  log "Detected Debian/Ubuntu-based Linux (apt)..."
  sudo apt-get update
  sudo apt-get install -y kitty mpv vlc ffmpeg git curl gnupg build-essential pkg-config libsqlite3-dev libssl-dev

  if ! command -v wezterm >/dev/null 2>&1; then
    log "Adding official WezTerm apt repository..."
    curl -fsSL https://apt.fury.io/wez/gpg.key | sudo gpg --yes --dearmor -o /usr/share/keyrings/wezterm-fury.gpg
    echo 'deb [signed-by=/usr/share/keyrings/wezterm-fury.gpg] https://apt.fury.io/wez/ * *' | sudo tee /etc/apt/sources.list.d/wezterm.list >/dev/null
    sudo chmod 644 /usr/share/keyrings/wezterm-fury.gpg
    sudo apt-get update && sudo apt-get install -y wezterm || warn "Could not install wezterm from apt; install manually from https://wezterm.org"
  fi

  if ! command -v yt-dlp >/dev/null 2>&1; then
    log "Installing latest yt-dlp binary to /usr/local/bin/yt-dlp..."
    sudo curl -fsSL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp
    sudo chmod a+rx /usr/local/bin/yt-dlp
  fi
}

install_fedora() {
  log "Detected Fedora/RHEL-based Linux (dnf)..."
  local fver
  fver="$(rpm -E %fedora 2>/dev/null || echo '')"
  if [ -n "$fver" ]; then
    sudo dnf install -y \
      "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-${fver}.noarch.rpm" \
      "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-${fver}.noarch.rpm" || true
  fi
  sudo dnf copr enable -y wezfurlong/wezterm-nightly || true
  sudo dnf install -y kitty wezterm mpv vlc yt-dlp ffmpeg git gcc openssl-devel sqlite-devel || \
    sudo dnf install -y kitty mpv yt-dlp ffmpeg git gcc
}

install_opensuse() {
  log "Detected openSUSE Linux (zypper)..."
  sudo zypper --non-interactive install kitty wezterm mpv vlc yt-dlp ffmpeg git gcc sqlite3-devel libopenssl-devel || \
    sudo zypper --non-interactive install kitty mpv vlc yt-dlp ffmpeg git gcc
}

install_nix() {
  log "Detected Nix environment (nix profile)..."
  nix profile install \
    nixpkgs#kitty \
    nixpkgs#wezterm \
    nixpkgs#mpv \
    nixpkgs#vlc \
    nixpkgs#yt-dlp \
    nixpkgs#ffmpeg \
    nixpkgs#git \
    nixpkgs#rustc \
    nixpkgs#cargo
}

install_alpine() {
  log "Detected Alpine Linux (apk)..."
  sudo apk add --no-cache kitty wezterm mpv vlc yt-dlp ffmpeg git build-base sqlite-dev openssl-dev curl
}

main() {
  local os_name
  os_name="$(uname -s)"

  case "$os_name" in
    Darwin)
      install_macos
      ;;
    Linux)
      if [ -f /etc/NIXOS ] || command -v nixos-version >/dev/null 2>&1; then
        install_nix
      elif command -v pacman >/dev/null 2>&1; then
        install_arch
      elif command -v apt-get >/dev/null 2>&1; then
        install_debian_ubuntu
      elif command -v dnf >/dev/null 2>&1; then
        install_fedora
      elif command -v zypper >/dev/null 2>&1; then
        install_opensuse
      elif command -v apk >/dev/null 2>&1; then
        install_alpine
      elif command -v nix >/dev/null 2>&1; then
        install_nix
      else
        warn "Unrecognized Linux package manager. Please ensure kitty, wezterm, mpv, vlc, yt-dlp, ffmpeg, git, and cargo are installed."
      fi
      ;;
    *)
      warn "Unsupported OS '$os_name' for install.sh (use install.ps1 on Windows)."
      exit 1
      ;;
  esac

  ensure_cargo

  local build_dir=""
  if [ -f "./Cargo.toml" ] && grep -q 'name = "phaenna"' ./Cargo.toml 2>/dev/null; then
    build_dir="$(pwd)"
    log "Building Phaenna from local checkout ($build_dir)..."
  else
    log "Fetching Phaenna source into $SRC_DIR..."
    if [ -d "$SRC_DIR/.git" ]; then
      git -C "$SRC_DIR" fetch --all --prune
      git -C "$SRC_DIR" pull --ff-only
    else
      mkdir -p "$(dirname "$SRC_DIR")"
      git clone "$REPO_URL" "$SRC_DIR"
    fi
    build_dir="$SRC_DIR"
  fi

  cargo install --path "$build_dir" --force

  local bin_path="${CARGO_HOME:-$HOME/.cargo}/bin/phaenna"
  if [ "$os_name" = "Darwin" ] && [ -f "$bin_path" ]; then
    codesign -s - -f "$bin_path" >/dev/null 2>&1 || true
  fi

  log "Verifying installation with phaenna doctor..."
  "$bin_path" doctor || true

  cat <<'EOF'

Installation complete!
Recommended launch: open Kitty or WezTerm and run:
  phaenna

Make sure Cargo's bin directory is on your PATH:
  export PATH="$HOME/.cargo/bin:$PATH"
EOF
}

main "$@"
