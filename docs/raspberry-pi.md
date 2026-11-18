# Raspberry Pi / uConsole: Linux ARM64

These optional scripts build the existing Phaenna application natively. The original
`install.sh`, `install.ps1`, Cargo files, Rust sources, and release workflow are unchanged.
All project credit and the MIT license remain with the upstream authors.

## Requirements

- A 64-bit Raspberry Pi OS / Debian installation (`uname -m`: `aarch64`).
- A graphical session for `mpv`. Foot needs Wayland; Kitty can also run under X11.
- Internet access and `sudo` for Debian packages. Run the installer as your normal user.
- Rust stable >= 1.88. The installer installs the official minimal stable toolchain
  through `rustup`; it does not rely on an older distribution Rust package.
- Allow several GB of disk space for Rust, dependencies, and build artifacts. A Pi with
  8 GB of RAM is recommended for building. Even an 8 GB device may need swap when
  browsers or other applications use substantial RAM. Smaller devices are unverified.

The build defaults to **one job** and optimization level **1 for the Phaenna package**
to reduce compilation pressure. This is a command-line Cargo setting; it does not
change upstream build profiles or dependency versions. The installer does not alter
swap, boot settings, Bluetooth, Wi-Fi, compositor bindings, or the default terminal.

## Install

Use a checkout of this repository. Review the scripts before running them:

```sh
bash scripts/raspberry-pi/install.sh
```

Choose Foot, Kitty, or keep the terminal you already use. Explicit alternatives:

```sh
bash scripts/raspberry-pi/install.sh --terminal foot
bash scripts/raspberry-pi/install.sh --terminal kitty
bash scripts/raspberry-pi/install.sh --terminal keep
```

The installer uses `apt` for build/playback dependencies (`mpv`, `ffmpeg`, `yt-dlp`,
`libssl-dev`, `libsqlite3-dev`), builds with the committed `Cargo.lock`, and installs into
`~/.local` by default:

| Path | Purpose |
| --- | --- |
| `~/.local/bin/phaenna` | Launcher, including optional USB checks |
| `~/.local/share/phaenna-rpi/phaenna` | Native ARM64 executable |
| `~/.local/share/phaenna-rpi/launcher.conf` | Download directory and optional USB UUID |
| `~/.config/phaenna/config.toml` | Phaenna preferences (respects `XDG_CONFIG_HOME`) |

An existing launcher is backed up before replacement. Existing Phaenna preferences,
watch history, SQLite cache, and Google account vaults are preserved. Reinstalling without
download options also preserves a previously configured download directory and USB guard.
For a new configuration the defaults are `mpv`, `p720`, `performance_mode = "efficient"`,
`shader_mode = "off"`, `motion_mode = "off"`, and self-updates disabled. For an existing
configuration, keep these values to prevent a local ARM64 build from being replaced by an
upstream auto-update:

```toml
[update]
check_on_startup = false
auto_install = false
```

Put `~/.local/bin` on your `PATH` if your shell does not already include it. Then run
`phaenna` from Foot or Kitty. Direct launch examples:

```sh
foot -e "$HOME/.local/bin/phaenna"
kitty "$HOME/.local/bin/phaenna"
```

Foot and Kitty use different image protocols; Phaenna probes the terminal automatically.
Cover art can take a moment to appear while feeds and thumbnails load. Maximize the
window; upstream recommends at least `125` columns by `38` rows. Videos open in `mpv`,
so changing terminals does not change video decoding.

## Download to a USB drive (optional)

Discover the filesystem UUID and its actual mount point:

```sh
lsblk -o NAME,TRAN,LABEL,UUID,MOUNTPOINTS
```

Pass **your own** values. A UUID identifies the filesystem rather than a changing
device name such as `/dev/sda1`:

```sh
bash scripts/raspberry-pi/install.sh --terminal foot \
  --download-dir /media/your-user/your-drive/Phaenna \
  --usb-uuid YOUR-FILESYSTEM-UUID
```

The installer links Phaenna's `~/Movies/Phaenna` Offline Vault to the chosen directory.
Downloads remain organized by channel. Press **uppercase D** on any video or in the Detail
Command Deck to download offline copies via `yt-dlp`. The launcher checks that the download
path is actually backed by the configured UUID and attempts a normal `udisksctl` mount if
needed. If the USB is disconnected, mounted at a different path, or unwritable, it refuses
to start rather than silently writing downloads to the system microSD card. Mount the drive
at the configured path or re-run the installer with the new path.

An existing download directory or different symlink at `~/Movies/Phaenna` is never
replaced automatically. Move existing downloads manually before configuring a new
destination. The chosen USB directory must be writable by your normal user. The
installer does not format drives or change their ownership.

For a regular folder without USB mount checks, omit `--usb-uuid`:

```sh
bash scripts/raspberry-pi/install.sh --terminal keep --download-dir "$HOME/Videos/Phaenna"
```

## Build or reuse a binary

```sh
bash scripts/raspberry-pi/build.sh --jobs 1
bash scripts/raspberry-pi/build.sh --dry-run --jobs 1
```

The build preview reports the locked Cargo command, job count, and low disk/memory warnings.

With dependencies already installed and a trusted native ARM64 executable:

```sh
bash scripts/raspberry-pi/install.sh --terminal keep --skip-deps \
  --binary /absolute/path/to/phaenna
```

- `--prefix /absolute/path` installs under another user-owned prefix.
- `--skip-deps` skips `apt` and `yt-dlp` installation, not compilation. Pass `--binary` as
  well to skip building. Do not run as `root`.
- `--dry-run` previews every change before touching files, packages, or symlinks:

```sh
bash scripts/raspberry-pi/install.sh --dry-run --terminal keep
```

A supplied binary or downloaded helper can be checksum-verified:

```sh
bash scripts/raspberry-pi/install.sh --binary /absolute/path/to/phaenna \
  --sha256 <64-character-SHA256>
```

Use `--yt-dlp-sha256` or `--rustup-sha256` when those helpers are downloaded outside `apt`.
If no checksum is supplied, the script prints an explicit warning rather than failing silently.

Lifecycle commands:

```sh
bash scripts/raspberry-pi/install.sh --verify
bash scripts/raspberry-pi/install.sh --repair
bash scripts/raspberry-pi/install.sh --uninstall
bash scripts/raspberry-pi/install.sh --uninstall --purge-config
```

- `--verify` checks the installed binary, launcher, launcher config, and download directory.
- `--repair` refreshes the installation while preserving existing configuration and downloads.
- `--uninstall` removes Phaenna's Raspberry Pi installation files while preserving user
  configuration, SQLite history, and downloads. `--purge-config` is an explicit opt-in
  removal of the Phaenna config directory.

Existing `config.toml` files are preserved and only missing Raspberry Pi defaults are
inserted. Existing keys, update preferences, comments, and unknown settings are not
overwritten. A backup named `config.toml.backup-<timestamp>` is created only when a merge
modifies the file.

If `~/Movies/Phaenna` is already a symlink to a different location, the installer refuses to
replace it unless `--replace-vault-link` is supplied. Real directories are never replaced
automatically:

```sh
bash scripts/raspberry-pi/install.sh --replace-vault-link \
  --download-dir /absolute/path/to/downloads
```

## Validation and limits

The installer scripts are verified against Debian 13 ARM64 (`linux/arm64`) builds and the
temporary-`HOME` offline regression suite in CI:

```sh
bash -n scripts/raspberry-pi/*.sh
shellcheck scripts/raspberry-pi/*.sh
python3 scripts/raspberry-pi/check_install.py /absolute/path/to/phaenna
```

These checks cover installation, reinstallation, preservation of configuration and
existing downloads, paths with spaces, and refusal to launch on the wrong USB. Hardware
video decoding on Raspberry Pi depends on the codec (`h264` vs `av1`/`vp9`), kernel/Mesa
drivers, and Wayland/X11 session configuration.

## En español

Ejecuta `bash scripts/raspberry-pi/install.sh` desde esta copia del repositorio y elige
**Foot**, **Kitty** o conservar tu terminal. Se necesita un sistema Linux ARM64 de 64 bits;
la primera compilación tarda y consume bastante memoria. El instalador no cambia tus atajos
del escritorio.

Para guardar descargas en un USB, utiliza `--download-dir` con la ruta real de tu carpeta y
`--usb-uuid` con el UUID mostrado por `lsblk`. No se borra ni formatea el disco. Si el USB
está desconectado, el lanzador se detiene para evitar escribir descargas en la tarjeta del
sistema. Desde el terminal escribe `phaenna`; sobre cualquier video, **D mayúscula** guarda
una copia local.
