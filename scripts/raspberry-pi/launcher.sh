#!/usr/bin/env bash
set -euo pipefail

resolve_realpath() {
    realpath -m -- "$1" 2>/dev/null || python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

resolve_readlink() {
    readlink -f -- "$1" 2>/dev/null || python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

prefix=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
# Generated with shell quoting by install.sh, and owned by the installing user.
# shellcheck source=/dev/null
source "$prefix/share/phaenna-rpi/launcher.conf"
export PATH="$prefix/bin:$PATH"

if [[ -n $PHAENNA_RPI_USB_UUID ]]; then
    command -v findmnt >/dev/null || {
        echo 'findmnt is unavailable; cannot verify the configured USB filesystem.' >&2; exit 1;
    }
    if ! findmnt -rn -S "UUID=$PHAENNA_RPI_USB_UUID" >/dev/null; then
        command -v udisksctl >/dev/null || {
            echo 'udisksctl is unavailable; cannot mount the configured USB drive.' >&2; exit 1;
        }
        udisksctl mount -b "/dev/disk/by-uuid/$PHAENNA_RPI_USB_UUID" || {
            echo 'Connect the configured USB drive before starting Phaenna.' >&2; exit 1;
        }
    fi
    # Check the filesystem actually backing the path, not just any mounted USB.
    probe=$PHAENNA_RPI_DOWNLOAD_DIR
    while [[ ! -d $probe && $probe != / ]]; do probe=$(dirname -- "$probe"); done
    actual_uuid=$(findmnt -rno UUID -T "$probe" 2>/dev/null || true)
    findmnt -rn -S "UUID=$PHAENNA_RPI_USB_UUID" >/dev/null || {
        echo 'Configured USB filesystem is not mounted.' >&2; exit 1;
    }
    [[ $actual_uuid == "$PHAENNA_RPI_USB_UUID" ]] || {
        echo 'Download path is not on the configured USB. Refusing to use the system disk.' >&2; exit 1;
    }
fi

if [[ -n $PHAENNA_RPI_DOWNLOAD_DIR ]]; then
    vault=$HOME/Movies/Phaenna
    [[ -L $vault && $(resolve_readlink "$vault") == $(resolve_realpath "$PHAENNA_RPI_DOWNLOAD_DIR") ]] || {
        echo 'The ~/Movies/Phaenna download link changed. Re-run the installer or restore the link.' >&2; exit 1;
    }
    [[ -d $PHAENNA_RPI_DOWNLOAD_DIR && -r $PHAENNA_RPI_DOWNLOAD_DIR && -w $PHAENNA_RPI_DOWNLOAD_DIR && -x $PHAENNA_RPI_DOWNLOAD_DIR ]] || {
        echo 'The download directory is unavailable or not writable.' >&2; exit 1;
    }
    export PHAENNA_DOWNLOAD_DIR=$PHAENNA_RPI_DOWNLOAD_DIR
fi
exec "$prefix/share/phaenna-rpi/phaenna" "$@"
