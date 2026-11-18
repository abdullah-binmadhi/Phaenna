#!/usr/bin/env bash
# Native Linux ARM64 build; upstream sources and Cargo.lock stay unchanged.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: build.sh [--jobs N] [--dry-run] [--help]
Build Phaenna natively on 64-bit Raspberry Pi OS / Debian ARM64.
Defaults to one build job and opt-level=1 for the Phaenna package to reduce RAM use.
Requires Rust >= 1.88, a C compiler, pkg-config, OpenSSL, and SQLite3 development headers.
EOF
}

jobs=1
dry_run=false
while (($#)); do
    case "$1" in
        --jobs) [[ $# -ge 2 ]] || { usage >&2; exit 2; }; jobs=$2; shift 2 ;;
        --dry-run) dry_run=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ $jobs =~ ^[1-9][0-9]*$ ]] || { echo '--jobs must be a positive integer.' >&2; exit 2; }
[[ $(uname -s) == Linux && $(uname -m) == aarch64 ]] || {
    echo 'Use a 64-bit Linux ARM64 OS (aarch64), not 32-bit Raspberry Pi OS.' >&2; exit 1;
}

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
rustup_bin=$(command -v rustup || true)
[[ -n $rustup_bin || ! -x $HOME/.cargo/bin/rustup ]] || rustup_bin=$HOME/.cargo/bin/rustup
if [[ -n $rustup_bin ]]; then
    cargo_cmd=("$rustup_bin" run stable cargo)
    rustc_cmd=("$rustup_bin" run stable rustc)
else
    cargo_cmd=(cargo)
    rustc_cmd=(rustc)
fi
version=$("${rustc_cmd[@]}" --version) || {
    echo 'Install Rust stable first, or run scripts/raspberry-pi/install.sh.' >&2; exit 1;
}
read -r _ version_number _ <<< "$version"
IFS=. read -r major minor _ <<< "$version_number"
((major > 1 || (major == 1 && minor >= 88))) || {
    echo 'Rust >= 1.88 is required by the locked dependencies. Run the installer to install stable.' >&2; exit 1;
}
command -v cc >/dev/null || { echo 'Install build-essential (C compiler).' >&2; exit 1; }
pkg-config --exists openssl || { echo 'Install pkg-config, libssl-dev, and libsqlite3-dev.' >&2; exit 1; }
pkg-config --exists sqlite3 || [[ -f /usr/include/sqlite3.h ]] || {
    echo 'Install pkg-config, libssl-dev, and libsqlite3-dev.' >&2; exit 1;
}

available_kb=$(df -Pk -- "$root" 2>/dev/null | awk 'NR==2 {print $4}')
if [[ $available_kb =~ ^[0-9]+$ && $available_kb -lt 2097152 ]]; then
    echo 'Warning: less than 2 GB free in the build filesystem; native builds may fail.' >&2
fi
if command -v free >/dev/null; then
    available_mb=$(free -m 2>/dev/null | awk '/^Mem:/ {print $7}')
    if [[ $available_mb =~ ^[0-9]+$ && $available_mb -lt 512 ]]; then
        echo 'Warning: less than 512 MB memory available; a one-job build is recommended.' >&2
    fi
fi

printf 'Build plan: rust stable, --locked, jobs=%s, opt-level=1 for phaenna\n' "$jobs"
if $dry_run; then
    echo 'Dry run complete. No build or generated files will change.'
    exit 0
fi

cd -- "$root"
# Only this invocation uses the lower optimization level; no Cargo.toml changes.
"${cargo_cmd[@]}" build --release --locked --jobs "$jobs" --target-dir "$root/target" \
    --config 'profile.release.package.phaenna.opt-level=1'
printf '\nBuilt: %s/target/release/phaenna\n' "$root"
