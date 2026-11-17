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

**Phaenna** is cinema-grade YouTube, reimagined for your terminal. Built in native Rust, it pairs an effortless, sub-second browsing experience and edge-to-edge thumbnails with a custom **4K Studio Mastering** engine, **120Hz ProMotion** fluidity, intelligent performance modes, and a live playback cockpit for `mpv`, `IINA`, and `VLC`.

---

## Demo



https://github.com/user-attachments/assets/4659ad96-2629-4fd2-a739-0ab5614630d4


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

### Cinema-Grade Video Mastery. Now Adapted to You.
Web browsers compress, wash out, and muddy the videos you love. Phaenna restores clarity, crispness, and rich color straight on your GPU — while giving you total control over power and memory:
- **Reference 4K Studio Pipeline (`mpv` & `IINA`)**: Cleans up blocky streaming artifacts, sharpens soft edges, and restores true-to-life color detail automatically — with selectable looks including **Reference 4K**, **Neural Cinema**, **Anime4K**, **Balanced**, and **Off**.
- **Performance Modes That Adapt Live (`m`)**:
  - **Efficient (~350MB RAM)**: Featherweight memory and battery footprint that still delivers a cleaner, crisper picture than browser playback.
  - **Balanced (~650MB RAM)**: The everyday sweet spot with richer color cleanup and responsive buffering.
  - **Quality (~1.4GB RAM)**: Maximum multi-pass 4K studio mastering with zero compromises — switchable live mid-video inside Playback Studio (`p`).
- **120Hz ProMotion Fluidity (`t`)**: Built for modern high-refresh displays with an adaptive 120Hz interface and selectable motion profiles — **Off** (native cadence), **Cadence** (judder-free 120Hz lock), and **Smooth 120Hz** motion interpolation.
- **No More Black Startup Screens**: An animated Phaenna loading card greets you the moment you press Play — showing the title, active quality tier, and buffer progress cleanly until the very first frame appears.

### Playback Studio & Original Audio Control (`p`)
Take complete command of your viewing experience without ever leaving the video:
- **Original Voice or Dubbed, One Key Away (`d`)**: Never get stuck listening to an unwanted auto-dubbed voice track. Lock in the creator's original language by default, or flip between Original and Dubbed tracks live on the fly.
- **Full 5-Zone Live Cockpit**: Hot-swap audio tracks and captions, normalize sudden volume spikes, fine-tune brightness/contrast/saturation/gamma, step speeds (`0.5x`–`2.5x`), scrub chapters, and cycle 4K shaders or Performance Modes — all updated live via keyboard or mouse.
- **Complete Endings Protected**: Smart outro safety keeps SponsorBlock from prematurely skipping the final seconds of a video, so stories and credits finish cleanly.

### Universal YouTube Keybindings — Windowed or Fullscreen
Keep every shortcut in your muscle memory across **`mpv`**, **`IINA`**, and **`VLC`**, even in full-screen theater mode:
- `k` / `Space` Play & Pause · `j` / `l` Jump `-10s` / `+10s` · `Left` / `Right` Step `-5s` / `+5s`
- `Up` / `Down` Volume · `m` Mute · `f` Toggle Fullscreen · `Esc` Exit Fullscreen
- `c` Captions · `d` Switch Audio/Dub Track · `<` / `>` Playback Speed · `,` / `.` Frame Step
- `0`–`9` Jump to `0%`–`90%` · `[` / `]` Previous & Next Chapter *(mpv / IINA)*

### Search Studio (`/`) That Thinks With You
Finding the exact video is instant, accurate, and interactive from the first keystroke:
- **Live Autocomplete & Recent Chips**: YouTube query suggestions appear in real time — tap `Tab` to complete or click any chip directly.
- **Precision Filter & Sort Bar**: Narrow by **Scope** (`All`, `Live YT`, `Subscribed`, `Cached`), **Type** (`Videos`, `Shorts`), **Upload Date** (`Today`, `Week`, `Month`, `Year`), **Duration** (`<4m`, `4–20m`, `>20m`), **Hide Watched**, and **Sort** (`Relevance`, `Newest`, `Most Viewed`, `Duration`).
- **Instant Results & Quick Actions**: Paste any YouTube link directly, paginate endlessly, scroll with mouse or keyboard, double-click for Details, or trigger **Play**, **+Queue**, **Watch Later**, and **Channel** in one click.

### Visual Interface & Edge-to-Edge Media
- **Zero-Gap 100%-Width 16:9 Thumbnails**: Trims black bars and side pillars so every card and inspector renders crisp, edge-to-edge artwork.
- **YouTube-Style Metadata & Focus Marquee**: Cards highlight view counts, release age, `[NEW]` badges, `[LIVE]` status, and smoothly scroll long titles on focus.
- **Split-Pane Inspectors & Bento Detail Deck (`Enter` / `i`)**: Upload Radar (`c`), Channel Catalog, and Detail pair rich 16:9 artwork with sentiment meters, interactive SponsorBlock timelines, wrapped descriptions, **Chapters**, and **Up Next**.
- **Dynamic Ambilight**: Subtly tints active frames using the dominant palette of the video you're exploring across 6 themes (`T`).

### Queueing, Identity & Companion Suite
- **Infinite Scroll & Continue Watching Deck**: Seamlessly browse deeper feeds and pin favorites (`a` / `+`) to your persistent Continue Watching deck.
- **Effortless Google Sign-In (`L`)**: Split-window SSO, Chrome sign-in, QR Device Code, or in-app verification kept safely on your machine.
- **More Crafted Experiences**:
  - `[A]` **Phaenna Wrapped Analytics**: Visual watch habits, circadian rhythm, 26-week thermal heatmap, and SponsorBlock time saved.
  - `[t]` **Comment Translator** (Detail Deck): Translate comments or full threads into any language (`g` to pick/type any language).
  - `[m]` **Mobile LAN Remote** via instant QR code, `[C]` **AirPlay 2 & Smart TV Cast**, `[D]` **Offline Vault**, `[s]` **Fuzzy Command Hub**, and `[U]` **Updater**.

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
| **Playback & Mastering** | `Space` | Play immediately in active player (`mpv` / `IINA` / `VLC`) with YouTube keybindings |
| | `p` | Playback Studio (`d` Dubbed/Original, `m` Performance Mode, `t` 120Hz Motion, `s` 4K Shaders) |
| | `Enter` / `i` | Open Bento Detail Command Deck (`Tab` Chapters, `t`/`T` Translate comment/thread, `g` Pick language, `c` Channel) |
| | `w` / `D` | Toggle Watch Later / Download to Offline Vault |
| **Discovery & Hubs** | `/` | Search Studio (Live Suggestions `Tab`, `Ctrl+t/d/u/s/w/g` Filters & Sort, `Ctrl+p` Play, `Ctrl+a` Queue) |
| | `c` | 7-Day Upload Radar with live `16:9` preview (`h`/`l` Day, `[`/`]` Week, `Space` Play, `a` +CW, `w` WL) |
| | `f` / `r` | Filter (`All` / `Longform` / `Shorts`) & Sort / Force refresh feeds |
| | `A` | Phaenna Wrapped Analytics |
| | `s` / `,` | Fuzzy Command Hub (includes one-click `FSRCNNX` 4K CNN pack install) |
| | `L` / `T` | Google Account & Identity Hub / Cycle 6 Themes |
| | `m` / `C` | Mobile LAN Remote QR / AirPlay & DLNA TV Cast |
| | `U` / `?` / `q` | Updater / Help Matrix / Quit |

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
