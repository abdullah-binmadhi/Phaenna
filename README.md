```text
          .   |   .            ____  _   _    _    _____ _   _ _   _    _
       \   \  |  /   /        |  _ \| | | |  / \  | ____| \ | | \ | |  / \
     ---  .-/---\-.  ---      | |_) | |_| | / _ \ |  _| |  \| |  \| | / _ \
          | | * | |           |  __/|  _  |/ ___ \| |___| |\  | |\  |/ ___ \
     ---  '-\---/-'  ---      |_|   |_| |_/_/   \_\_____|_| \_|_| \_/_/   \_\
       /   /  |  \   \
          '   |   '         Terminal YouTube Studio  |  4K GLSL Mastering  |  v1.0.0
```

> *Phaenna (FFXIV) — radiant light & craftsmanship.*  
> Source: `https://github.com/abdullah-binmadhi/Phaenna`

**Phaenna** is a cinema-grade terminal YouTube client built in Rust with **Ratatui**, pairing an instant `<15ms` SQLite WAL engine and GPU-accelerated terminal graphics (`Kitty`, `WezTerm`, `iTerm2`, `Sixel`, or universal half-block) with a custom **Reference 4K Studio GLSL mastering pipeline** for `mpv` and `IINA`.

---

## Demo

[![Watch Phaenna 1660x1080 60fps Demo](https://github.com/abdullah-binmadhi/Phaenna/releases/download/media/phaenna-preview.png)](https://github.com/abdullah-binmadhi/Phaenna/releases/download/media/phaenna-demo.mp4)

> Click the preview above to watch the full **1660x1080 60fps Showcase Video** ([`phaenna-demo.mp4`](https://github.com/abdullah-binmadhi/Phaenna/releases/download/media/phaenna-demo.mp4)).

---

## Workspace Architecture

```text
+----------------------------------------------------------------------------------+
| PHAENNA | [1] Continue Watching  [2] Home  [3] Subscriptions  [4] History [5] WL |
+----------------------------------------------------------------------------------+
| +-----------------------+  +---------------------------------------------------+ |
| |                       |  | FEATURED SPOTLIGHT (AMBILIGHT)                    | |
| |  16:9 EDGE-TO-EDGE    |  | Title Marquee on Focus · Release Age · View Count | |
| |  CELL-EXACT ARTWORK   |  | [NEW] · 4K-AV1 · SponsorBlock Clean · Channels    | |
| +-----------------------+  +---------------------------------------------------+ |
+-----------------------------------+----------------------------------------------+
| [ Adaptive Responsive Grid ]      | [ Split-Pane Live 16:9 Preview Inspector ]   |
|  +--------------+ +-------------+ |  Upload Radar (c) / Channel / Search (/)     |
|  |  16:9 Cover  | | 16:9 Cover  | |  +--------------------+  Title Marquee       |
|  |  Full Width  | | Full Width  | |  |  16:9 Preview Art  |  Channel · Duration  |
|  |  Ticker >... | | Title Text  | |  |  (Auto-Queued)     |  Views · Release Age |
|  |  1.2M · 3h   | | [NEW] 42m   | |  +--------------------+  Synopsis & Quick Key|
|  +--------------+ +-------------+ +----------------------------------------------+
+----------------------------------------------------------------------------------+
| [/] Search  [p] Play Options  [s] Command Hub  [A] Wrapped  [L] Auth  [?] Help   |
+----------------------------------------------------------------------------------+
```

---

## Highlights & Capabilities

### Reference 4K Studio Mastering Pipeline (`mpv` & `IINA`)
Solves YouTube bitrate starvation, `4:2:0` chroma bleeding, and compression macroblocking inside a unified GPU shader chain (`src/services/shaders.rs` + `src/api/player.rs`):
- **High-Bitrate Stream Priority**: `Quality::Best` prioritizes `av01` > `vp9.2`/`vp9` > `4K/8K` with `format-sort=res:2160,fps,hdr,vcodec:av01:vp9.2:vp9,vbr,abr` and a `512MiB` (`128MiB` back-buffer) demuxer cache.
- **Stage 1 — `01_deblock_dering.glsl` (`LUMA`)**: Edge-preserving bilateral DCT macroblock & mosquito-noise filter that dissolves compression blocking in flat gradients without softening fine detail.
- **Stage 2 — `02_chroma_guided.glsl` (`CHROMA` + `LUMA`)**: Luma-guided `4:2:0 -> 4:4:4` chroma upsampler eliminating red/cyan edge fringing on text and high-contrast shots.
- **Stage 3 — `03_superres_4k_rcas.glsl` (`LUMA 2x`)**: Directional structure-tensor edge reconstruction paired with AMD FidelityFX RCAS and a strict 3x3 `[min, max]` anti-ringing clamp.
- **Selectable Studio Profiles (`VideoShaderMode`)**: Cycle between `Reference4K` (default), `NeuralCinema` (unlocks downloadable `FSRCNNX_x2_16-0-4-1.glsl` CNN weights directly inside Settings Hub `[s]`), `Anime4K` (`04_anime4k_restore.glsl` contour restoration), `Balanced`, and `Off`, backed by `ewa_lanczossharp` scaling (`0.7` antiring), 3-pass 16-bit debanding with fine film grain, temporal dithering, and `spline` HDR tone mapping.
- **Full macOS `IINA` Parity**: `build_iina_args` translates the entire shader stack, format sorting, debanders, cookies, subtitles, and resume offset into `--mpv-*` arguments for `iina-cli`.

### Visual Interface & Edge-to-Edge Media
- **Zero-Gap 100%-Width 16:9 Thumbnails**: Trims both legacy `4:3` black letterboxes and solid side pillars, then pre-scales every frame to exact cell pixel geometry (`width * font_w`, `height * font_h`) via `CatmullRom` / `Lanczos3` so thumbnails fill every card and inspector edge-to-edge.
- **YouTube-Style Metadata & Ticker Marquee**: Cards display compact view counts, release age (`2h ago`, `3d ago`), `[NEW]` indicators on recent uploads, `[LIVE]` status, and horizontal stock-tape marquee scrolling on the focused title so long titles never stay truncated.
- **58% / 42% Split-Pane Preview Inspectors**: Upload Radar (`c`), Channel Catalog (`c` from Detail), and Instant Search (`/`) feature live right-hand `16:9` thumbnail previews, metadata telemetry, synopsis excerpts, and instant `[Space]` / `[a]` playback & queueing.
- **Bento Detail Command Deck (`Enter` / `i`)**: Aspect-locked `16:9` stage, ReturnYouTubeDislike sentiment gauge, proportional SponsorBlock segment bar, active 4K mastering readout, wrapped description with extracted links/tags, and interactive `Tab`-switchable **Chapters** & **Up Next** drawer.
- **Dynamic HSV Ambilight**: Extracts saturated dominant hues from focused video art to tint active panes cleanly across 6 selectable themes (`T`).

### Infinite Scroll, Queueing, Identity & Intelligence
- **Infinite Scroll & Continue Watching Queue Deck**: Automatically paginates feeds as you scroll (`PgDn` / `^d` / `n`), paired with a persistent `[1] Continue Watching` watch-later & resume deck (`[a]`/`[+]` pin from any view, `[x]`/`[d]` remove, `[X]` clear finished, `[K]`/`[J]` reorder, `[v]` toggle Deck/Grid view).
- **In-App Google Identity Hub (`L`)**: Supports Kitty/WezTerm Split SSO, Chrome app-window login, QR OAuth Device Code, or in-TUI email & 2FA verification stored with `0600` vault permissions.
- **Intelligence & Companion Suite**:
  - `[A]` **Phaenna Wrapped Analytics**: 24-hour Braille circadian distribution, runtime histogram, 26-week thermal watch heatmap, SponsorBlock time saved, and Top Channels.
  - `[t]` **Discovery Crucible**: 6-step taste constellation wizard + Live Vibe Tuner.
  - `[m]` / `phaenna remote`: Mobile LAN WebSocket remote with terminal QR code.
  - `[C]` **AirPlay 2 & DLNA Smart TV Cast** via SSDP discovery.
  - `[D]` **Offline Vault Downloader** -> `~/Movies/Phaenna/{Channel}/{Title}.mp4` with chapter embedding.
  - `[s]` / `,` **Fuzzy Command Hub** (`SkimMatcherV2`), `[!]` **System Doctor**, and `[U]` **Self-Updater**.

---

## One-Command Installation (Everything in One Go)

Every installer below installs **GPU terminals** (`kitty`, `wezterm`), **video players & dependencies** (`mpv`, `vlc`, `iina` on macOS, `yt-dlp`, `ffmpeg`, `git`), the **Rust toolchain** (`cargo` if missing), builds & installs **`phaenna`**, and runs `phaenna doctor`.

### Universal One-Liner (Auto-Detects macOS & Linux Distros)

```bash
curl -fsSL https://raw.githubusercontent.com/abdullah-binmadhi/Phaenna/main/install.sh | bash
```

**Windows 10 / 11 (PowerShell — `WezTerm`, `Windows Terminal`, `mpv`, `VLC`, `yt-dlp`, `FFmpeg`, `Git`, `Rustup`, `Phaenna`):**

```powershell
irm https://raw.githubusercontent.com/abdullah-binmadhi/Phaenna/main/install.ps1 | iex
```

---

### Copy-Paste Per-OS Single Commands

> Prefer pasting an explicit OS command directly? Pick your platform below:

#### macOS (Apple Silicon / Intel via Homebrew)
Installs `kitty`, `wezterm`, `iina`, `vlc`, `mpv`, `yt-dlp`, `ffmpeg`, `git`, `rust`, compiles `phaenna`, self-signs on Apple Silicon, and runs `phaenna doctor`:

```bash
(command -v brew >/dev/null || NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)") && (eval "$(/opt/homebrew/bin/brew shellenv 2>/dev/null || /usr/local/bin/brew shellenv 2>/dev/null)" || true) && brew install mpv yt-dlp ffmpeg git && for c in kitty wezterm iina vlc; do brew install --cask "$c" || true; done && (command -v cargo >/dev/null || (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env")) && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && codesign -s - -f "$HOME/.cargo/bin/phaenna" && "$HOME/.cargo/bin/phaenna" doctor
```

#### Linux — Arch / Manjaro / EndeavourOS (`pacman`)
Installs `kitty`, `wezterm`, `mpv`, `vlc`, `yt-dlp`, `ffmpeg`, `git`, `base-devel`, `rust`, and builds `phaenna`:

```bash
sudo pacman -S --needed --noconfirm kitty wezterm mpv vlc yt-dlp ffmpeg git base-devel rust && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && "$HOME/.cargo/bin/phaenna" doctor
```

#### Linux — Ubuntu / Debian / Pop!_OS / Linux Mint (`apt`)
Installs `kitty`, official `wezterm` apt repo, `mpv`, `vlc`, `ffmpeg`, latest official `yt-dlp`, build tools, `rustup`, and builds `phaenna`:

```bash
sudo apt-get update && sudo apt-get install -y kitty mpv vlc ffmpeg git curl gnupg build-essential pkg-config libsqlite3-dev libssl-dev && curl -fsSL https://apt.fury.io/wez/gpg.key | sudo gpg --yes --dearmor -o /usr/share/keyrings/wezterm-fury.gpg && echo 'deb [signed-by=/usr/share/keyrings/wezterm-fury.gpg] https://apt.fury.io/wez/ * *' | sudo tee /etc/apt/sources.list.d/wezterm.list >/dev/null && sudo apt-get update && sudo apt-get install -y wezterm && sudo curl -fsSL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && sudo chmod a+rx /usr/local/bin/yt-dlp && (command -v cargo >/dev/null || (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env")) && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && "$HOME/.cargo/bin/phaenna" doctor
```

#### Linux — Fedora / Nobara / RHEL (`dnf`)
Enables RPM Fusion + WezTerm COPR, installs `kitty`, `wezterm`, `mpv`, `vlc`, `yt-dlp`, `ffmpeg`, `git`, `cargo`, `rust`, and builds `phaenna`:

```bash
sudo dnf install -y "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm" "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm" || true; sudo dnf copr enable -y wezfurlong/wezterm-nightly || true; sudo dnf install -y kitty wezterm mpv vlc yt-dlp ffmpeg git rust cargo gcc openssl-devel sqlite-devel && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && "$HOME/.cargo/bin/phaenna" doctor
```

#### Linux — openSUSE Tumbleweed / Leap (`zypper`)
Installs `kitty`, `wezterm`, `mpv`, `vlc`, `yt-dlp`, `ffmpeg`, `git`, `rust`, `cargo`, and builds `phaenna`:

```bash
sudo zypper --non-interactive install kitty wezterm mpv vlc yt-dlp ffmpeg git rust cargo gcc sqlite3-devel libopenssl-devel && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && "$HOME/.cargo/bin/phaenna" doctor
```

#### Linux — NixOS / Nix (`nix profile`)
Installs `kitty`, `wezterm`, `mpv`, `vlc`, `yt-dlp`, `ffmpeg`, `git`, `rustc`, `cargo` into your Nix profile and builds `phaenna`:

```bash
nix profile install nixpkgs#kitty nixpkgs#wezterm nixpkgs#mpv nixpkgs#vlc nixpkgs#yt-dlp nixpkgs#ffmpeg nixpkgs#git nixpkgs#rustc nixpkgs#cargo && rm -rf /tmp/phaenna-src && git clone https://github.com/abdullah-binmadhi/Phaenna.git /tmp/phaenna-src && cargo install --path /tmp/phaenna-src --force && "$HOME/.cargo/bin/phaenna" doctor
```

#### Windows 10 / 11 (PowerShell + `winget`)
> Note: `Kitty` does not ship native Windows builds (`WezTerm` and `Windows Terminal` provide native GPU rendering on Windows; Kitty runs via WSL2 + any Linux command above).

```powershell
foreach ($id in @("wez.wezterm","Microsoft.WindowsTerminal","shinchiro.mpv","VideoLAN.VLC","yt-dlp.yt-dlp","Gyan.FFmpeg","Git.Git","Rustlang.Rustup")) { winget install --id $id -e --silent --accept-package-agreements --accept-source-agreements }; $env:PATH = "$env:USERPROFILE\.cargo\bin;" + [Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [Environment]::GetEnvironmentVariable("Path","User"); rustup default stable; git clone https://github.com/abdullah-binmadhi/Phaenna.git "$env:TEMP\phaenna-src"; cargo install --path "$env:TEMP\phaenna-src" --force; & "$env:USERPROFILE\.cargo\bin\phaenna.exe" doctor
```

> Make sure `$HOME/.cargo/bin` (Unix) or `%USERPROFILE%\.cargo\bin` (Windows) is on your `PATH`, then launch inside **Kitty** or **WezTerm** with `phaenna`.

---

## Keybindings

| Context | Key | Action |
| :--- | :--- | :--- |
| **Navigation** | `h` / `l` (`Left`/`Right`) | Move card cursor left / right |
| | `j` / `k` (`Down`/`Up`) | Move row / queue item down / up |
| | `PgDn` / `Ctrl+d` / `n` | Scroll down & paginate next batch of videos |
| | `1` – `5` / `Tab` | Switch shelves (`Continue Watching`, `Home`, `Subscriptions`, `History`, `Watch Later`) |
| **Continue Watching** | `a` / `+` | Save focused video to `[1] Continue Watching` (works across Home, Detail, Search, Channel, Radar) |
| | `x` / `d` | Remove selected item from Continue Watching |
| | `K` / `J` | Reorder queued item up / down |
| | `X` / `v` | Clear finished items / toggle Deck vs. Grid layout |
| **Playback & Mastering** | `Space` | Play immediately in active player (`mpv` / `IINA` / `VLC`) |
| | `p` | Play Options (`[s]` cycles Reference 4K / Neural Cinema / Anime4K / Balanced / Off) |
| | `Enter` / `i` | Open Bento Detail Command Deck (`Tab` switches Chapters / Up Next, `c` drills into Channel) |
| | `w` / `D` | Toggle Watch Later / Download to Offline Vault |
| **Discovery & Hubs** | `/` | Split-pane Instant SQLite + Live YouTube Search (`Ctrl+p` Play, `Ctrl+a` +Continue Watching) |
| | `c` | 7-Day Upload Radar with live `16:9` preview (`h`/`l` Day, `[`/`]` Week, `Space` Play, `a` +CW, `w` WL) |
| | `f` / `r` | Filter (`All` / `Longform` / `Shorts`) & Sort / Force refresh feeds |
| | `A` / `t` | Phaenna Wrapped Analytics / Discovery Crucible (`Tailored`) |
| | `s` / `,` | Fuzzy Command Hub (includes one-click `FSRCNNX` 4K CNN pack install) |
| | `L` / `T` | Google Account & Identity Hub / Cycle 6 Themes |
| | `m` / `C` | Mobile LAN Remote QR / AirPlay & DLNA TV Cast |
| | `!` / `U` / `?` / `q` | System Doctor / Updater / Help Matrix / Quit |

---

## CLI Reference

```bash
phaenna                     # Launch TUI workspace
phaenna remote --port 8485  # Run standalone Mobile LAN Remote QR server
phaenna doctor              # Inspect yt-dlp, mpv, IINA, VLC & ffmpeg readiness
phaenna --cookies path.txt  # Override Netscape cookies file
phaenna --clear-cache       # Clear cached feed rows & thumbnail blobs
phaenna --no-remote         # Disable background LAN remote HTTP/WS server
```

---

## Local Build & Verification

```bash
cargo test -- --nocapture
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo install --path . --force
```
