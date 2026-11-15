# Phaenna (`phaenna` v1.0.0)

> *Phaenna (FFXIV) — radiant light & craftsmanship.*  
> Repository: `https://github.com/abdullah-binmadhi/Phaenna.git`

A lightning-fast, cross-platform terminal YouTube workspace built in Rust with Ratatui, featuring **<15ms SQLite WAL Stale-While-Revalidate boot**, **non-blocking `mpv` JSON-RPC playback**, **SponsorBlock auto-skipping**, **HSV Ambilight borders**, **LAN Mobile Remote QR**, **AirPlay / DLNA TV casting**, **Phaenna Wrapped analytics**, and **native Zen/Firefox SQLite cookie sync**.

## Highlights

- **<15ms Instant Cold Boot (SQLite WAL + SWR)**: Hydrates Continue Watching, Home Feed, Subscriptions, History, Watch Later, and compressed thumbnails immediately from SQLite while refreshing stale feeds asynchronously in the background.
- **Non-Blocking Player & Live `mpv` IPC**: Never freezes the TUI while watching! Saves exact resume timestamps every 4 seconds, auto-marks videos watched at 85%, and supports **mpv, IINA, VLC, and Browser**.
- **SponsorBlock Auto-Skip & Visual Segment Timeline**: Queries SponsorBlock segments, renders proportional timelines on Video Detail screens, and automatically seeks past sponsored segments inside `mpv`.
- **Dynamic HSV Ambilight + HD 16:9 Art**: Crops legacy 4:3 letterboxed YouTube thumbnails edge-to-edge, renders crisp native Kitty/iTerm2/Sixel or half-block previews with modal occlusion guards, and extracts dominant colors for focused frames.
- **ReturnYouTubeDislike & Chapters (`Tab`)**: Displays community like/dislike ratios and lets you jump directly to any native or description-parsed chapter timestamp.
- **GLSL Video Mastering Shaders & Cinema Subtitles**: Optional real-time FidelityFX luma-only CAS sharpening shaders + 5 libass subtitle typography presets.
- **Intelligence Suite**:
  - `[a]` **Phaenna Wrapped Analytics**: 24-hour Braille circadian plot, runtime distribution, 26-week thermal habit heatmap, SponsorBlock time-saved telemetry, and Top Channels.
  - `[t]` **Discovery Crucible (`Tailored`)**: 6-step taste constellation questionnaire, Braille pentagon persona radar, and real-time Live Vibe Tuner.
  - `[c]` **7-Day Subscription Upload Radar**: Timezone-aware Mon–Sun upload timeline.
- **Companion Ecosystem**:
  - `[m]` / `phaenna remote`: Zero-dependency mobile WebSocket remote with terminal ASCII QR code.
  - `[C]` **AirPlay 2 & DLNA Smart TV Casting** via SSDP discovery.
  - `[D]` **Offline Vault Downloader** -> `~/Movies/Phaenna/{Channel}/{Title}.mp4` with chapter embedding and instant local playback.
  - `[s]` **Fuzzy Command Hub** powered by `SkimMatcherV2`.
  - `[L]` **Pure-Rust SQLite Cookie Extractor** (reads Zen / Firefox profiles safely without locking).

## Keybindings

| Key | Action |
| :--- | :--- |
| `h` / `l` (`←` / `→`) | Move grid cursor left / right |
| `j` / `k` (`↓` / `↑`) | Move grid row / switch shelf |
| `1` – `5` / `Tab` | Switch shelves (`Continue Watching`, `Home`, `Subscriptions`, `History`, `Watch Later`) |
| `Enter` / `i` | Open Video Detail (Chapters, SponsorBlock timeline, Sentiment, Channel drill-down) |
| `Space` | Start non-blocking playback immediately |
| `p` | Play Options modal (8 Quality presets up to 4K60, Player, SponsorBlock, Captions) |
| `/` | Instant SQLite + 350ms debounced live YouTube search |
| `f` | Filter format (`All`, `Longform`, `Shorts`) & Sort order |
| `w` | Toggle Watch Later |
| `D` | Download to Offline Vault |
| `n` / `]` | Stream next 16 videos into active shelf |
| `a` / `t` / `c` | Analytics (`Wrapped`) / Tailored Discovery / Upload Calendar |
| `m` / `C` | Mobile LAN Remote QR / TV Cast Scanner |
| `s` / `,` | Fuzzy Feature Command Hub |
| `L` / `T` | Account Cookie Vault Sync / Cycle 6 Themes |
| `!` / `U` / `?` | System Doctor / GitHub Self-Updater / Keybind Matrix |
| `q` / `Esc` | Quit / Back |

## CLI Usage

```bash
phaenna                   # Launch TUI workspace
phaenna remote --port 8485 # Run standalone Mobile LAN Remote QR server
phaenna doctor             # Inspect yt-dlp, mpv, IINA, VLC & ffmpeg readiness
phaenna --cookies path.txt # Override Netscape cookies file
phaenna --clear-cache      # Clear cached feed rows & thumbnail blobs
phaenna --no-remote        # Disable background LAN remote HTTP/WS server
```

## Building & Testing

```bash
cargo test
cargo build --release
cargo install --path . --force
```
