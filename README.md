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

**Phaenna** is cinema-grade YouTube, reimagined for your terminal. Built in native Rust, it pairs an effortless, sub-second browsing experience and borderless `16:9` thumbnails with a custom **4K Studio Mastering** engine, **120Hz ProMotion** fluidity, Wayland/Hyprland Picture-in-Picture, and a live `mpv` playback cockpit.

---

## Demo

https://github.com/user-attachments/assets/4659ad96-2629-4fd2-a739-0ab5614630d4

---

## Workspace Architecture

```text
+----------------------------------------------------------------------------------+
| PHAENNA | Continue Watching  Home Feed  Shorts  Subscriptions  History  ♪ Music  |
+----------------------------------------------------------------------------------+
| +-----------------------+  +---------------------------------------------------+ |
| | 16:9 SPOTLIGHT COVER  |  | FEATURED SPOTLIGHT (AMBILIGHT)                    | |
| | (Adaptive Height)     |  | [NEW] · Channel · Duration · Views · Release Age  | |
| +-----------------------+  +---------------------------------------------------+ |
| +--------------------+  +--------------------+  +--------------------+           |
| |                    |  |                    |  |                    |           |
| | 16:9 BORDERLESS    |  | 16:9 BORDERLESS    |  | 16:9 BORDERLESS    |           |
| | FULL-WIDTH ARTWORK |  | FULL-WIDTH ARTWORK |  | FULL-WIDTH ARTWORK |           |
| +--------------------+  +--------------------+  +--------------------+           |
| | Title Marquee      |  | Title Marquee      |  | Title Marquee      |           |
| | Channel · Duration |  | Channel · Duration |  | Channel · Duration |           |
| +--------------------+  +--------------------+  +--------------------+           |
+----------------------------------------------------------------------------------+
```

---

## Highlights & Capabilities

### Cinema-Grade Video Mastery Powered by `mpv`
Web browsers compress, wash out, and muddy the videos you love. Phaenna restores clarity, crispness, and rich color straight on your GPU via **`mpv`** — while giving you total control over power and memory:
- **Reference 4K Studio Pipeline (`mpv`)**: Cleans up blocky streaming artifacts, sharpens soft edges, and restores true-to-life color detail automatically — with selectable looks including **Reference 4K**, **Neural Cinema**, **Anime4K**, **Balanced**, and **Off**.
- **Performance Modes That Adapt Live (`m`)**:
  - **Efficient**: Featherweight memory and battery footprint that still delivers a cleaner, crisper picture than browser playback.
  - **Balanced**: Everyday sweet spot with 1-pass debanding, dynamic thermal governor, and responsive buffering.
  - **Quality**: Maximum multi-pass 4K studio mastering — switchable live mid-video inside Playback Studio (`p`).
- **120Hz ProMotion Fluidity (`t`)**: Adaptive 120Hz cadence and selectable motion profiles — **Off** (native cadence), **Cadence** (dynamic governor), and **Smooth 120Hz** motion interpolation.
- **Dynamic Picture-in-Picture & Fullscreen (`p` / `P` / `f`)**: Watch in a borderless, pinned 16:9 floating corner overlay across Wayland (Hyprland/Sway), macOS, Windows, and X11, cycle corners/sizes (`P`), or jump seamlessly into Fullscreen (`f`).
- **No More Black Startup Screens**: An animated Phaenna loading card greets you the moment you press Play — showing the title, active quality tier, and buffer progress cleanly until the very first frame appears.

### Playback Studio & Original Audio Control (`p`)
- **Original Voice or Dubbed, One Key Away (`d`)**: Never get stuck listening to an unwanted auto-dubbed voice track. Lock in the creator's original language by default, or flip between Original and Dubbed tracks live on the fly.
- **Full Live Cockpit**: Hot-swap audio tracks and captions, normalize sudden volume spikes, fine-tune brightness/contrast/saturation/gamma, step speeds (`0.5x`–`2.5x`), scrub chapters, and cycle 4K shaders or Performance Modes — all updated live over `mpv` JSON-RPC IPC.
- **Complete Endings Protected**: Smart outro safety keeps SponsorBlock from prematurely skipping the final seconds of a video, so stories and credits finish cleanly.

### Universal YouTube Keybindings Inside `mpv`
- `k` / `Space` Play & Pause · `j` / `l` Jump `-10s` / `+10s` · `Left` / `Right` Step `-5s` / `+5s`
- `-` / `+` (`=`) Volume · `m` Mute · `f` Toggle Fullscreen · `Esc` Exit Fullscreen
- `p` Toggle PiP · `P` Cycle PiP Corner & Size · `i` / `I` Bento HUD (`6s` / `Pin 1Hz`) · `F1` Raw Stats
- `C` Transparent Live Comment Chat (`↑`/`↓`/Mouse Wheel scroll · Click select/expand/reply without pausing video · `;`/`\` Comment/Reply · `'` Replies · `Alt+c` Style · `Alt+r` VOD Sync · `Alt+t` Translate · Comment & Subcomment GIFs/Stickers)
- `c` Captions · `d` Switch Audio/Dub Track · `<` / `>` Playback Speed · `,` / `.` Frame Step
- `0`–`9` Jump to `0%`–`90%` · `[` / `]` Previous & Next Chapter

### Search Studio, YouTube Music & Companion Suite
- **YouTube Music Tab (`7` or click `♪ Music`)**: Native `music.youtube.com` workspace inside Phaenna with `Home` mixes (*Listen again*, *Quick picks*), `Library`, `Liked Music`, `Charts`, filtered catalog `Search` (`All`, `Songs`, `Videos`, `Albums`, `Playlists`, `Artists`), full Album / Playlist / Artist drill-down (`Enter` / `Backspace`), **windowless gapless background `mpv` audio**, `1:1` Ambilight square album covers, `Shuffle` (`z`) & `Repeat` (`R`), live **Song ↔ Video** switch (`v`), `♥ Like` (`l`), interactive seek bar, and **Up Next / Time-Synced Lyrics** (`y`).
- **Search Studio (`/`)**: Live YouTube autocomplete (`Tab`), precision scope/type/date/duration filters, paste any YouTube URL directly, and instant Play / Queue / Watch Later / Channel actions.
- **Bento Detail Deck (`Enter`)**: 16:9 preview, live dislike telemetry, SponsorBlock timeline, wrapped synopsis, clickable **Chapters**, **Comment Translator** (`t` / `T` / `g`), and live **[f] Fullscreen** control.
- **Isolated Browser Popup Google Sign-In (`L`)**: Launches an isolated popup window using your installed Chromium-based browser (`Chromium`, `Google Chrome`, `Brave`, `Microsoft Edge`, or `Vivaldi`) with temporary profile auto-cleanup — zero daily browser SQLite scraping.
- **Companion Suite**: `[A]` **Phaenna Analysis**, `[c]` **7-Day Upload Radar**, `[m]` **Mobile LAN Remote QR**, `[C]` **Smart TV Cast**, `[D]` **Offline Vault**, `[s]` **Fuzzy Command Hub**, `[T]` **Themes**, and `[U]` **Auto-Updater**.

---

## One-Command Installation (Everything in One Go)

Every installer sets up **`mpv`**, **`yt-dlp`**, **`deno`** (YouTube JS challenge runtime), **`ffmpeg`**, a GPU terminal (`kitty` / `wezterm` if none is installed), and **Chromium** (only if no Chromium-based browser is already present for `[L]` Google Sign-In), then installs the **prebuilt `phaenna` release binary** with SHA256 verification (or compiles from source with `--from-source`) and runs `phaenna doctor`.

### Universal One-Liner (macOS & Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/abdullah-binmadhi/Phaenna/main/install.sh | bash
```

*(Pass `--from-source` to compile locally with Cargo instead of downloading the prebuilt binary: `curl -fsSL https://raw.githubusercontent.com/abdullah-binmadhi/Phaenna/main/install.sh | bash -s -- --from-source`.)*

### Windows 10 / 11 (PowerShell)

Installs `mpv`, `yt-dlp`, `deno`, `ffmpeg`, `WezTerm` / `Windows Terminal` (Edge is already built in for Google Sign-In), and `phaenna.exe`:

```powershell
irm https://raw.githubusercontent.com/abdullah-binmadhi/Phaenna/main/install.ps1 | iex
```

### Raspberry Pi / Linux ARM64 (Opt-In)

Optional scripts for 64-bit Raspberry Pi OS / Debian ARM64 (`--dry-run`, low-RAM `--jobs 1` build, Foot/Kitty choice, USB UUID download guard, and `--uninstall`):

```bash
bash scripts/raspberry-pi/install.sh --dry-run
bash scripts/raspberry-pi/install.sh --terminal foot
```

See [docs/raspberry-pi.md](docs/raspberry-pi.md) for full instructions, USB storage setup, checksum verification, and recovery steps.

---

### Manual Per-OS Packages

If you prefer installing dependencies manually and building from source:

- **macOS (Homebrew):** `brew install mpv yt-dlp deno ffmpeg git && brew install --cask kitty` *(plus `--cask chromium` if you don't have Chrome/Brave/Edge)*
- **Arch / Asahi / EndeavourOS (`pacman`):** `sudo pacman -S --needed mpv yt-dlp deno ffmpeg git kitty chromium`
- **Ubuntu / Debian (`apt`):** `sudo apt-get install -y mpv ffmpeg git curl kitty chromium` + latest `yt-dlp` & `deno` (`install.sh` handles both automatically)
- **Fedora (`dnf`):** enable RPM Fusion, then `sudo dnf install -y mpv yt-dlp ffmpeg git kitty chromium` + `deno`
- **Windows (`winget`):** `shinchiro.mpv`, `yt-dlp.yt-dlp`, `DenoLand.Deno`, `Gyan.FFmpeg`, `wez.wezterm`

> Make sure `$HOME/.local/bin` (Unix) or `%LOCALAPPDATA%\Programs\phaenna` (Windows) is in your `PATH`, then launch inside **Kitty**, **WezTerm**, or **Ghostty** with `phaenna`.

---

## Keybindings

| Context | Key | Action |
| :--- | :--- | :--- |
| **Navigation** | `h` / `l` (`Left`/`Right`) | Move card cursor left / right |
| | `j` / `k` (`Down`/`Up`) | Move row / queue item down / up |
| | `PgDn` / `Ctrl+d` / `n` | Scroll down & paginate next batch of videos |
| | `1` – `5` / `Tab` | Switch shelves (`Continue Watching`, `Home`, `Subscriptions`, `History`, `Watch Later`) |
| **Continue Watching** | `a` / `+` | Save focused video to `[1] Continue Watching` (across Home, Detail, Search, Channel, Radar) |
| | `x` / `d` | Remove selected item from Continue Watching |
| | `K` / `J` | Reorder queued item up / down |
| | `X` / `v` | Clear finished items / toggle Deck vs. Grid layout |
| **Playback & Mastering** | `Space` | Play immediately in `mpv` with YouTube keybindings |
| | `p` | Playback Studio (`d` Dubbed/Original, `m` Performance Mode, `t` 120Hz Motion, `s` 4K Shaders, `f` Fullscreen) |
| | `Enter` / `i` | Open Bento Detail Command Deck (`f` Fullscreen when playing, `Tab` Chapters, `t`/`T` Translate, `g` Pick language, `c` Channel) |
| | `w` / `D` | Toggle Watch Later / Download to Offline Vault |
| **Discovery & Hubs** | `/` | Search Studio (Live Suggestions `Tab`, `Ctrl+t/d/u/s/w/g` Filters & Sort, `Ctrl+p` Play, `Ctrl+a` Queue) |
| | `c` | 7-Day Upload Radar with live `16:9` preview (`h`/`l` Day, `[`/`]` Week, `Space` Play, `a` +CW, `w` WL) |
| | `f` / `r` | Filter (`All` / `Longform` / `Shorts`) & Sort / Force refresh feeds |
| | `A` | Phaenna Analysis (Daily Budget, Finish Rate, Channels & Session Ledger) |
| | `s` / `,` | Fuzzy Command Hub (includes one-click `FSRCNNX` 4K CNN pack install) |
| | `L` / `T` | Google Account Sign-In (`[L]` Isolated Browser Popup) / Cycle 6 Themes |
| | `m` / `C` | Mobile LAN Remote QR / Smart TV Cast |
| | `U` / `?` / `q` | Updater / Help Matrix / Quit |

---

## CLI Reference

```bash
phaenna                     # Launch TUI workspace
phaenna remote --port 8485  # Run standalone Mobile LAN Remote QR server
phaenna doctor              # Inspect mpv, yt-dlp, deno, ffmpeg & Sign-In browser readiness
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
