#!/usr/bin/env bash
# Additive Raspberry Pi installer. Does not change upstream's install.sh.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: install.sh [OPTIONS]
  --terminal foot|kitty|keep  Install Foot (Wayland), Kitty, or keep your terminal
  --download-dir /PATH        Optional directory for offline downloads
  --usb-uuid UUID             Require that directory to reside on this USB filesystem
  --jobs N                    Build jobs (default: 1)
  --prefix /PATH              Installation prefix (default: ~/.local)
  --binary /PATH              Install an existing native ARM64 binary instead of building
  --skip-deps                 Skip apt and yt-dlp installation (dependencies already installed)
  --dry-run                   Print the validated change plan without changing the system
  --sha256 HEX                Verify a supplied --binary with SHA256
  --verify                    Check the existing installation and exit
  --repair                    Reinstall/repair the existing installation
  --uninstall                 Remove Phaenna Pi files without deleting configuration or downloads
  --purge-config              With --uninstall, also remove Phaenna config and database
  --yt-dlp-sha256 HEX         Verify yt-dlp download when fetched outside apt
  --rustup-sha256 HEX         Verify rustup download
  --replace-vault-link        Replace an existing ~/Movies/Phaenna symlink to a different target
  -h, --help                  Show this help

Run as your regular user. sudo is used only to install Debian packages.
Without --terminal: prompt on a terminal; otherwise keep the existing terminal.
Existing Phaenna configuration is preserved. New configuration disables self-updates.
EOF
}
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }
resolve_realpath() {
    realpath -m -- "$1" 2>/dev/null || python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"
}
resolve_readlink() {
    readlink -f -- "$1" 2>/dev/null || python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"
}
sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | awk '{print $1}'
    else
        shasum -a 256 -- "$1" | awk '{print $1}'
    fi
}
verify_sha256() {
    local file=$1 expected=$2 actual expected_lc actual_lc
    actual=$(sha256_file "$file") || die "could not calculate SHA256 for $file"
    expected_lc=$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')
    actual_lc=$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')
    [[ $actual_lc == "$expected_lc" ]] || die "SHA256 mismatch for $file (expected $expected, got $actual)"
}
download_verified() {
    local url=$1 expected=$2 output=$3
    [[ $url == https://* ]] || die 'download URLs must use HTTPS'
    curl --fail --location --silent --show-error --retry 3 -o "$output" -- "$url" || die 'download failed'
    if [[ -n $expected ]]; then verify_sha256 "$output" "$expected"; else
        printf 'Warning: no SHA256 checksum supplied for %s\n' "$url" >&2
    fi
}
terminal=ask
download_dir=
download_dir_set=false
usb_uuid=
jobs=1
prefix=$HOME/.local
binary=
skip_deps=false
dry_run=false
binary_sha=
yt_dlp_sha=
rustup_sha=
verify_only=false
repair=false
uninstall=false
purge_config=false
replace_vault_link=false
while (($#)); do
    case "$1" in
        --terminal|--download-dir|--usb-uuid|--jobs|--prefix|--binary|--sha256|--yt-dlp-sha256|--rustup-sha256)
            [[ $# -ge 2 && -n $2 ]] || die "$1 needs a value"
            case "$1" in
                --terminal) terminal=$2 ;;
                --download-dir) download_dir=$2; download_dir_set=true ;;
                --usb-uuid) usb_uuid=$2 ;;
                --jobs) jobs=$2 ;;
                --prefix) prefix=$2 ;;
                --binary) binary=$2 ;;
                --sha256) binary_sha=$2 ;;
                --yt-dlp-sha256) yt_dlp_sha=$2 ;;
                --rustup-sha256) rustup_sha=$2 ;;
            esac
            shift 2 ;;
        --skip-deps) skip_deps=true; shift ;;
        --dry-run) dry_run=true; shift ;;
        --verify) verify_only=true; shift ;;
        --repair) repair=true; shift ;;
        --uninstall) uninstall=true; shift ;;
        --purge-config) purge_config=true; shift ;;
        --replace-vault-link) replace_vault_link=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
[[ $jobs =~ ^[1-9][0-9]*$ ]] || die '--jobs must be a positive integer'
if $verify_only && $uninstall || $verify_only && $repair || $uninstall && $repair; then
    die '--verify, --repair, and --uninstall are mutually exclusive'
fi
if $purge_config && ! $uninstall; then die '--purge-config requires --uninstall'; fi
if [[ -n $binary_sha && ! $binary_sha =~ ^[[:xdigit:]]{64}$ ]]; then die '--sha256 must be a 64-character hexadecimal value'; fi
if [[ -n $yt_dlp_sha && ! $yt_dlp_sha =~ ^[[:xdigit:]]{64}$ ]]; then die '--yt-dlp-sha256 must be a 64-character hexadecimal value'; fi
if [[ -n $rustup_sha && ! $rustup_sha =~ ^[[:xdigit:]]{64}$ ]]; then die '--rustup-sha256 must be a 64-character hexadecimal value'; fi
[[ $prefix != *[$'\n\r\t']* ]] || die '--prefix contains unsupported control characters'
[[ -z $download_dir || $download_dir != *[$'\n\r\t']* ]] || die '--download-dir contains unsupported control characters'
[[ $prefix == /* ]] || die '--prefix must be an absolute path'
[[ -z $download_dir || $download_dir == /* ]] || die '--download-dir must be an absolute path'
[[ -z $usb_uuid || $usb_uuid =~ ^[[:alnum:]-]+$ ]] || die 'invalid UUID'
[[ -z $usb_uuid || -n $download_dir ]] || die '--usb-uuid requires --download-dir'
if [[ ${PHAENNA_RPI_ALLOW_TEST_ARCH:-0} != 1 ]] && ! $verify_only && ! $uninstall; then
    [[ $(uname -s) == Linux && $(uname -m) == aarch64 ]] || die '64-bit Linux ARM64 (aarch64) is required'
fi
((EUID != 0)) || die 'run as your regular user, without sudo'

prefix=$(resolve_realpath "$prefix")
settings_file=$prefix/share/phaenna-rpi/launcher.conf

if $verify_only; then
    ok=true
    for path in "$prefix/share/phaenna-rpi/phaenna" "$prefix/bin/phaenna" "$settings_file"; do
        if [[ ! -e $path && ! -L $path ]]; then
            printf 'Missing: %s\n' "$path" >&2
            ok=false
        fi
    done
    if [[ -x $prefix/share/phaenna-rpi/phaenna ]]; then
        printf 'OK: native binary present\n'
        if [[ -n $binary_sha ]]; then
            verify_sha256 "$prefix/share/phaenna-rpi/phaenna" "$binary_sha" || ok=false
        fi
    else
        printf 'Missing or non-executable: %s\n' "$prefix/share/phaenna-rpi/phaenna" >&2
        ok=false
    fi
    if [[ -f $settings_file ]]; then
        # shellcheck source=/dev/null
        source "$settings_file"
        if [[ -n ${PHAENNA_RPI_DOWNLOAD_DIR:-} ]]; then
            [[ -d $PHAENNA_RPI_DOWNLOAD_DIR && -w $PHAENNA_RPI_DOWNLOAD_DIR ]] || {
                printf 'Invalid download directory: %s\n' "$PHAENNA_RPI_DOWNLOAD_DIR" >&2
                ok=false
            }
        fi
    fi
    $ok && { echo 'Phaenna Raspberry Pi installation verification passed.'; exit 0; }
    exit 1
fi

if $uninstall; then
    echo 'Removing Phaenna Raspberry Pi files...'
    rm -f -- "$prefix/bin/phaenna" "$prefix/share/phaenna-rpi/phaenna"
    rm -rf -- "$prefix/share/phaenna-rpi"
    if $purge_config; then
        config_dir=${XDG_CONFIG_HOME:-$HOME/.config}/phaenna
        rm -rf -- "$config_dir"
        echo "Purged configuration directory: $config_dir"
    else
        echo 'Configuration and downloaded content were preserved.'
    fi
    echo 'Phaenna Raspberry Pi files removed.'
    exit 0
fi

if [[ $terminal == ask ]]; then
    if [[ -t 0 ]]; then
        printf 'Terminal: 1) Foot (Wayland)  2) Kitty  3) Keep current [3]: '
        read -r answer || answer=
        case "$answer" in 1) terminal=foot ;; 2) terminal=kitty ;; 3|'') terminal=keep ;; *) die 'choose 1, 2 or 3' ;; esac
    else
        terminal=keep
    fi
fi
case "$terminal" in foot|kitty|keep) ;; *) die '--terminal must be foot, kitty or keep' ;; esac
if [[ $terminal == foot && -z ${WAYLAND_DISPLAY:-} ]]; then
    echo 'Foot requires a Wayland session; it will not run in a plain X11 session.'
fi
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
prefix=$(resolve_realpath "$prefix")
settings_file=$prefix/share/phaenna-rpi/launcher.conf

preview_install() {
    local existing_binary=no existing_config=no existing_launcher=no
    [[ ! -e $prefix/share/phaenna-rpi/phaenna && ! -L $prefix/share/phaenna-rpi/phaenna ]] || existing_binary=yes
    [[ -f ${XDG_CONFIG_HOME:-$HOME/.config}/phaenna/config.toml ]] && existing_config=yes
    [[ -e $prefix/bin/phaenna || -L $prefix/bin/phaenna ]] && existing_launcher=yes

    cat <<EOF
Raspberry Pi ARM64 installer preview
  Architecture: Linux ARM64 (aarch64)
  Prefix: $prefix
  Terminal: $terminal
  Download storage: ${download_dir:-not configured}
  USB UUID: ${usb_uuid:-none}
  Existing native binary: $existing_binary
  Existing launcher: $existing_launcher
  Existing config: $existing_config
  Build jobs: $jobs
  Supplied binary SHA256: ${binary_sha:-not supplied}
  Dependency mode: $($skip_deps && printf 'skip apt/yt-dlp' || printf 'apt + yt-dlp/rustup as needed')
EOF
}

if ! $download_dir_set && [[ -f $settings_file ]]; then
    # Preserve USB guards when reinstalling without download options.
    # shellcheck source=/dev/null
    source "$settings_file"
    download_dir=${PHAENNA_RPI_DOWNLOAD_DIR:-}
    [[ -n $usb_uuid ]] || usb_uuid=${PHAENNA_RPI_USB_UUID:-}
fi

# Validate the download destination before installing packages or compiling.
if [[ -n $download_dir ]]; then
    download_dir=$(resolve_realpath "$download_dir")
    vault=$HOME/Movies/Phaenna
    if [[ -e $vault || -L $vault ]]; then
        if [[ -L $vault ]] && [[ $(resolve_readlink "$vault") == "$download_dir" ]]; then
            :
        elif $replace_vault_link; then
            printf '  Vault link: will be replaced only after the installer passes preflight\n' >&2
        elif $dry_run; then
            printf '  Vault link: existing and will be preserved or require confirmation\n' >&2
        else
            die "$HOME/Movies/Phaenna already exists; move your downloads manually or rerun with --replace-vault-link"
        fi
    fi

    if [[ -n $usb_uuid ]]; then
        if ! $dry_run; then
            command -v findmnt >/dev/null || die 'findmnt is required to verify USB filesystem UUIDs'
            if ! findmnt -rn -S "UUID=$usb_uuid" >/dev/null; then
                command -v udisksctl >/dev/null || die 'udisksctl is required to mount the configured USB'
                udisksctl mount -b "/dev/disk/by-uuid/$usb_uuid" || die 'connect and mount the USB first'
            fi
        fi
        probe=$download_dir
        while [[ ! -d $probe && $probe != / ]]; do probe=$(dirname -- "$probe"); done
        if ! $dry_run; then
            findmnt -rn -S "UUID=$usb_uuid" >/dev/null || die 'configured USB filesystem did not mount'
            [[ $(findmnt -rno UUID -T "$probe" 2>/dev/null || true) == "$usb_uuid" ]] ||
                die 'download directory is not on the specified USB filesystem'
        fi
    fi

    if ! $dry_run; then
        mkdir -p -- "$download_dir"
        [[ -d $download_dir && -r $download_dir && -w $download_dir && -x $download_dir ]] || die 'download directory is unavailable or not writable/readable by your user'
    fi
fi

if [[ -n $binary ]]; then
    [[ -f $binary && -x $binary ]] || die 'binary must be an executable file'
    [[ -z $binary_sha ]] || verify_sha256 "$binary" "$binary_sha"
    if command -v file >/dev/null; then
        file --brief -- "$binary" | grep -q 'ELF 64-bit.*ARM aarch64' || die 'binary is not a Linux ARM64 executable'
    else
        grep -a -m1 -q 'ELF' "$binary" || die 'binary does not appear to be an ELF executable'
    fi
fi

# Validate destination writability and minimum build prerequisites before mutation.
mkdir_parent=$(dirname -- "$prefix")
if [[ -e $mkdir_parent || -L $mkdir_parent ]]; then
    [[ -w $mkdir_parent ]] || die "installation parent is not writable: $mkdir_parent"
else
    while [[ ! -e $mkdir_parent && $mkdir_parent != / ]]; do mkdir_parent=$(dirname -- "$mkdir_parent"); done
    [[ -w $mkdir_parent ]] || die "installation parent is not writable: $mkdir_parent"
fi

preview_install
if $dry_run; then
    printf '\nDry run complete. No files, packages, symlinks, or configuration were changed.\n'
    exit 0
fi

if ! $skip_deps; then
    command -v apt-get >/dev/null || die 'this installer supports Debian / Raspberry Pi OS; install dependencies manually and use --skip-deps'
    packages=(build-essential pkg-config libssl-dev libsqlite3-dev ca-certificates curl git mpv ffmpeg yt-dlp file)
    [[ $terminal == keep ]] || packages+=("$terminal")
    [[ -z $usb_uuid ]] || packages+=(udisks2)
    sudo apt-get update
    sudo apt-get install -y "${packages[@]}"
    if ! command -v yt-dlp >/dev/null; then
        tmp_ytdlp=$(mktemp)
        trap 'rm -f -- "${tmp_ytdlp:-}" "${tmp_rustup:-}"' EXIT
        download_verified "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp" "$yt_dlp_sha" "$tmp_ytdlp"
        mkdir -p -- "$prefix/bin"
        install -m755 "$tmp_ytdlp" "$prefix/bin/yt-dlp"
    fi
fi

if [[ -z $binary ]]; then
    rustup_bin=$(command -v rustup || true)
    [[ -n $rustup_bin || ! -x $HOME/.cargo/bin/rustup ]] || rustup_bin=$HOME/.cargo/bin/rustup
    if [[ -z $rustup_bin ]]; then
        tmp_rustup=$(mktemp)
        trap 'rm -f -- "${tmp_ytdlp:-}" "${tmp_rustup:-}"' EXIT
        download_verified "https://sh.rustup.rs" "$rustup_sha" "$tmp_rustup"
        sh "$tmp_rustup" -y --profile minimal --default-toolchain none --no-modify-path
        rustup_bin=$HOME/.cargo/bin/rustup
    fi
    "$rustup_bin" toolchain install stable --profile minimal
    bash "$root/scripts/raspberry-pi/build.sh" --jobs "$jobs"
    binary=$root/target/release/phaenna
fi
[[ -f $binary && -x $binary ]] || die 'binary must be an executable file'
[[ -z $binary_sha ]] || verify_sha256 "$binary" "$binary_sha"
if command -v file >/dev/null; then
    file --brief -- "$binary" | grep -q 'ELF 64-bit.*ARM aarch64' || die 'binary is not a Linux ARM64 executable'
fi

mkdir -p -- "$prefix/bin" "$prefix/share/phaenna-rpi"
cp -f -- "$binary" "$prefix/share/phaenna-rpi/phaenna"
chmod 755 "$prefix/share/phaenna-rpi/phaenna"
if [[ -e $prefix/bin/phaenna || -L $prefix/bin/phaenna ]]; then
    backup=$prefix/bin/phaenna.backup-$(date +%Y%m%d-%H%M%S)-$$
    cp -Pp -- "$prefix/bin/phaenna" "$backup"
    printf 'Previous launcher saved as %s\n' "$backup"
fi
launcher_conf_tmp=$(mktemp "$prefix/share/phaenna-rpi/.launcher.conf.XXXXXX")
{
    printf 'PHAENNA_RPI_DOWNLOAD_DIR=%q\n' "$download_dir"
    printf 'PHAENNA_RPI_USB_UUID=%q\n' "$usb_uuid"
} > "$launcher_conf_tmp"
chmod 600 "$launcher_conf_tmp"
mv -f -- "$launcher_conf_tmp" "$settings_file"
cp -f -- "$root/scripts/raspberry-pi/launcher.sh" "$prefix/bin/phaenna"
chmod 755 "$prefix/bin/phaenna"

if [[ -n $download_dir ]]; then
    mkdir -p "$HOME/Movies"
    vault=$HOME/Movies/Phaenna
    if [[ -L $vault ]]; then
        if [[ $(resolve_readlink "$vault") == "$download_dir" ]]; then
            :
        elif $replace_vault_link; then
            rm -- "$vault"
            ln -s -- "$download_dir" "$vault"
        else
            die "$vault points to a different directory; rerun with --replace-vault-link to replace only this symlink"
        fi
    elif [[ -e $vault ]]; then
        die "$vault already exists as a real directory; move downloads or remove it manually"
    else
        ln -s -- "$download_dir" "$vault"
    fi
fi
config_dir=${XDG_CONFIG_HOME:-$HOME/.config}/phaenna
if [[ ! -e $config_dir/config.toml ]]; then
    mkdir -p "$config_dir"
    config_tmp=$(mktemp "$config_dir/.config.toml.XXXXXX")
    cat > "$config_tmp" <<'EOF'
quality = "p720"
player = "mpv"
shader_mode = "off"
performance_mode = "efficient"
motion_mode = "off"

[update]
check_on_startup = false
auto_install = false

[cache]
recommended_ttl = 1800
subscriptions_ttl = 900
history_ttl = 3600
watch_later_ttl = 21600
EOF
    chmod 600 "$config_tmp"
    mv -f -- "$config_tmp" "$config_dir/config.toml"
else
    if command -v python3 >/dev/null; then
        python3 "$root/scripts/raspberry-pi/merge_config.py" "$config_dir/config.toml"
    else
        config_backup=$config_dir/config.toml.backup-$(date +%Y%m%d-%H%M%S)-$$
        cp -p -- "$config_dir/config.toml" "$config_backup"
        echo "Existing config preserved. Backup saved as $config_backup"
        echo 'Python 3 was unavailable, so no defaults were merged.'
    fi
fi
printf '\nInstalled: %s/bin/phaenna\nTerminal choice: %s\n' "$prefix" "$terminal"
case ":$PATH:" in
    *":$prefix/bin:"*) echo 'Run: phaenna' ;;
    *) printf 'Add this directory to your PATH: %s/bin\nOr run: %q\n' "$prefix" "$prefix/bin/phaenna" ;;
esac
echo 'No desktop shortcuts, terminal defaults, or compositor settings were changed.'
