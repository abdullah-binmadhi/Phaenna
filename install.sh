#!/usr/bin/env bash
# Phaenna Universal All-In-One Installer (macOS & Linux)
# Installs: mpv, yt-dlp, deno (YouTube JS solver), ffmpeg, GPU terminal (Kitty if none),
#           Chromium (only if no Chromium-family browser is present for Google Sign-In),
#           and Phaenna (prebuilt release binary with SHA256 verification, or --from-source).
set -euo pipefail

REPO_SLUG="abdullah-binmadhi/Phaenna"
REPO_URL="https://github.com/${REPO_SLUG}.git"
RELEASE_BASE="https://github.com/${REPO_SLUG}/releases/latest/download"
SRC_DIR="${PHAENNA_SRC_DIR:-$HOME/.local/share/phaenna-src}"
BIN_DIR="${PHAENNA_BIN_DIR:-$HOME/.local/bin}"
FROM_SOURCE=0

for arg in "$@"; do
  case "$arg" in
    --from-source) FROM_SOURCE=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: install.sh [--from-source]
  --from-source   Compile Phaenna from source via Cargo instead of downloading the prebuilt release binary.
EOF
      exit 0
      ;;
  esac
done

log() { printf "\n[phaenna-setup] %s\n" "$*"; }
warn() { printf "\n[phaenna-setup] WARNING: %s\n" "$*" >&2; }

has_gpu_terminal() {
  command -v kitty >/dev/null 2>&1 \
    || command -v wezterm >/dev/null 2>&1 \
    || command -v ghostty >/dev/null 2>&1 \
    || { [ "$(uname -s)" = "Darwin" ] && { [ -d "/Applications/kitty.app" ] || [ -d "/Applications/WezTerm.app" ] || [ -d "/Applications/Ghostty.app" ]; }; }
}

has_chromium_browser() {
  local b
  for b in chromium chromium-browser google-chrome google-chrome-stable brave brave-browser microsoft-edge microsoft-edge-stable vivaldi vivaldi-stable; do
    if command -v "$b" >/dev/null 2>&1; then
      return 0
    fi
  done
  if [ "$(uname -s)" = "Darwin" ]; then
    for b in \
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser" \
      "/Applications/Chromium.app/Contents/MacOS/Chromium" \
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"; do
      [ -x "$b" ] && return 0
    done
  fi
  return 1
}

ensure_deno_fallback() {
  if command -v deno >/dev/null 2>&1 || [ -x "$HOME/.deno/bin/deno" ]; then
    mkdir -p "$BIN_DIR"
    if [ -x "$HOME/.deno/bin/deno" ] && [ ! -e "$BIN_DIR/deno" ]; then
      ln -sf "$HOME/.deno/bin/deno" "$BIN_DIR/deno"
    fi
    return 0
  fi
  log "Installing Deno runtime (required by yt-dlp for YouTube extraction)..."
  curl -fsSL https://deno.land/install.sh | DENO_INSTALL="$HOME/.deno" sh -s -- -y >/dev/null 2>&1 || true
  if [ -x "$HOME/.deno/bin/deno" ]; then
    mkdir -p "$BIN_DIR"
    ln -sf "$HOME/.deno/bin/deno" "$BIN_DIR/deno"
  else
    warn "Could not auto-install deno. Install deno manually if YouTube playback reports JS challenge errors."
  fi
}

ensure_latest_ytdlp_local() {
  mkdir -p "$BIN_DIR"
  if ! command -v yt-dlp >/dev/null 2>&1; then
    log "Installing latest standalone yt-dlp to $BIN_DIR/yt-dlp..."
    curl -fsSL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o "$BIN_DIR/yt-dlp"
    chmod a+rx "$BIN_DIR/yt-dlp"
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

  log "Installing media stack (mpv, yt-dlp, deno, ffmpeg, git)..."
  brew install mpv yt-dlp deno ffmpeg git

  if ! has_gpu_terminal; then
    log "Installing Kitty GPU terminal..."
    brew install --cask kitty || warn "Optional cask 'kitty' skipped."
  fi
  if ! has_chromium_browser; then
    log "No Chromium-based browser detected — installing Chromium for Google Sign-In popup..."
    brew install --cask chromium || warn "Install Chrome/Chromium/Brave manually if you plan to use [L] Sign-In."
  fi
}

install_arch() {
  log "Detected Arch-based Linux (pacman)..."
  local pkgs=(mpv yt-dlp deno ffmpeg git curl unzip)
  has_gpu_terminal || pkgs+=(kitty)
  has_chromium_browser || pkgs+=(chromium)
  sudo pacman -S --needed --noconfirm "${pkgs[@]}"
}

install_debian_ubuntu() {
  log "Detected Debian/Ubuntu-based Linux (apt)..."
  local pkgs=(mpv ffmpeg git curl ca-certificates unzip)
  has_gpu_terminal || pkgs+=(kitty)
  sudo apt-get update
  sudo apt-get install -y "${pkgs[@]}"

  if ! has_chromium_browser; then
    log "No Chromium-based browser detected — installing Chromium for Google Sign-In popup..."
    sudo apt-get install -y chromium || sudo apt-get install -y chromium-browser || \
      warn "Could not install Chromium via apt; install Chrome/Chromium/Brave manually for [L] Sign-In."
  fi

  ensure_latest_ytdlp_local
  ensure_deno_fallback
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
  local pkgs=(mpv yt-dlp ffmpeg git curl unzip)
  has_gpu_terminal || pkgs+=(kitty)
  has_chromium_browser || pkgs+=(chromium)
  sudo dnf install -y "${pkgs[@]}" || sudo dnf install -y mpv yt-dlp ffmpeg git curl unzip
  ensure_deno_fallback
}

install_opensuse() {
  log "Detected openSUSE Linux (zypper)..."
  local pkgs=(mpv yt-dlp ffmpeg git curl unzip)
  has_gpu_terminal || pkgs+=(kitty)
  has_chromium_browser || pkgs+=(chromium)
  sudo zypper --non-interactive install "${pkgs[@]}" || sudo zypper --non-interactive install mpv yt-dlp ffmpeg git curl unzip
  ensure_deno_fallback
}

install_nix() {
  log "Detected Nix environment (nix profile)..."
  local pkgs=(nixpkgs#mpv nixpkgs#yt-dlp nixpkgs#deno nixpkgs#ffmpeg nixpkgs#git)
  has_gpu_terminal || pkgs+=(nixpkgs#kitty)
  has_chromium_browser || pkgs+=(nixpkgs#chromium)
  nix profile install "${pkgs[@]}"
}

install_alpine() {
  log "Detected Alpine Linux (apk)..."
  local pkgs=(mpv yt-dlp ffmpeg git curl unzip)
  has_gpu_terminal || pkgs+=(kitty)
  has_chromium_browser || pkgs+=(chromium)
  sudo apk add --no-cache "${pkgs[@]}"
  ensure_deno_fallback
}

install_build_deps_for_source() {
  local os_name="$1"
  if [ "$os_name" = "Linux" ]; then
    if command -v pacman >/dev/null 2>&1; then
      sudo pacman -S --needed --noconfirm base-devel rust
    elif command -v apt-get >/dev/null 2>&1; then
      sudo apt-get install -y build-essential pkg-config libsqlite3-dev libssl-dev
    elif command -v dnf >/dev/null 2>&1; then
      sudo dnf install -y gcc openssl-devel sqlite-devel
    elif command -v zypper >/dev/null 2>&1; then
      sudo zypper --non-interactive install gcc sqlite3-devel libopenssl-devel
    elif command -v apk >/dev/null 2>&1; then
      sudo apk add --no-cache build-base sqlite-dev openssl-dev
    fi
  fi
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

detect_release_asset() {
  local os_name="$1" arch_name="$2"
  case "${os_name}:${arch_name}" in
    Darwin:arm64|Darwin:aarch64) echo "phaenna-macos-aarch64" ;;
    Darwin:x86_64)               echo "phaenna-macos-x86_64" ;;
    Linux:x86_64|Linux:amd64)    echo "phaenna-linux-x86_64" ;;
    Linux:aarch64|Linux:arm64)   echo "phaenna-linux-aarch64" ;;
    *)                           echo "" ;;
  esac
}

verify_sha256_file() {
  local file_path="$1" expected="$2" actual=""
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file_path" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file_path" | awk '{print $1}')"
  else
    return 0
  fi
  [ "$actual" = "$expected" ]
}

install_prebuilt_binary() {
  local os_name="$1" arch_name="$2" asset tmp_dir bin_tmp sums_tmp expected_sha
  asset="$(detect_release_asset "$os_name" "$arch_name")"
  [ -n "$asset" ] || return 1

  tmp_dir="$(mktemp -d)"
  bin_tmp="$tmp_dir/$asset"
  sums_tmp="$tmp_dir/SHA256SUMS.txt"

  log "Downloading prebuilt release binary ($asset)..."
  if ! curl -fsSL "${RELEASE_BASE}/${asset}" -o "$bin_tmp"; then
    rm -rf "$tmp_dir"
    return 1
  fi
  if curl -fsSL "${RELEASE_BASE}/SHA256SUMS.txt" -o "$sums_tmp"; then
    expected_sha="$(grep " ${asset}\$" "$sums_tmp" | awk '{print $1}' | head -n1 || true)"
    if [ -n "$expected_sha" ] && ! verify_sha256_file "$bin_tmp" "$expected_sha"; then
      warn "Checksum mismatch on $asset; falling back to source build."
      rm -rf "$tmp_dir"
      return 1
    fi
  fi

  mkdir -p "$BIN_DIR"
  install -m 0755 "$bin_tmp" "$BIN_DIR/phaenna"
  rm -rf "$tmp_dir"

  if [ "$os_name" = "Darwin" ] && [ -f "$BIN_DIR/phaenna" ]; then
    codesign -s - -f "$BIN_DIR/phaenna" >/dev/null 2>&1 || true
  fi
  return 0
}

install_from_source() {
  local os_name="$1"
  install_build_deps_for_source "$os_name"

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
  local cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/phaenna"
  mkdir -p "$BIN_DIR"
  ln -sf "$cargo_bin" "$BIN_DIR/phaenna"
  if [ "$os_name" = "Darwin" ] && [ -f "$cargo_bin" ]; then
    codesign -s - -f "$cargo_bin" >/dev/null 2>&1 || true
  fi
}

main() {
  local os_name arch_name
  os_name="$(uname -s)"
  arch_name="$(uname -m)"

  case "$os_name" in
    Darwin) install_macos ;;
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
        warn "Unrecognized Linux package manager. Ensure kitty/wezterm, mpv, yt-dlp, deno, ffmpeg, and chromium are installed."
      fi
      ;;
    *)
      warn "Unsupported OS '$os_name' for install.sh (use install.ps1 on Windows)."
      exit 1
      ;;
  esac

  export PATH="$BIN_DIR:$HOME/.deno/bin:$HOME/.cargo/bin:$PATH"

  if [ "$FROM_SOURCE" -eq 1 ] || [ -f "./Cargo.toml" ] && grep -q 'name = "phaenna"' ./Cargo.toml 2>/dev/null; then
    install_from_source "$os_name"
  elif ! install_prebuilt_binary "$os_name" "$arch_name"; then
    warn "Prebuilt binary unavailable for ${os_name}/${arch_name}; building from source..."
    install_from_source "$os_name"
  fi

  log "Verifying installation with phaenna doctor..."
  "$BIN_DIR/phaenna" doctor || true

  cat <<EOF

Installation complete!
Binary installed at: $BIN_DIR/phaenna
Recommended launch: open Kitty (or WezTerm/Ghostty) and run:
  phaenna

Make sure $BIN_DIR is in your PATH:
  export PATH="$BIN_DIR:\$PATH"
EOF
}

main "$@"
