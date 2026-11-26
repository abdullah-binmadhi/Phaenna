//! Cross-platform binary detector & non-blocking external video player launcher.

use crate::api::ytdlp::SelectedStreamPair;
use crate::config::{
    AudioChannelMode, Config, DubPreference, LiveStreamMode, ManualMasteringConfig, MotionMode,
    NetworkMode, PerformanceMode, PictureGrade, Player, VideoShaderMode, EQ_BAND_FREQS_HZ,
};
use crate::error::{AppError, Result};
use crate::services::shaders::{
    mpv_mastering_flags_with_manual, prepare_video_shaders_with_mastering, VideoMasteringProfile,
};
use crate::services::substyler::SubtitleStyleConfig;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Default)]
pub struct DependencyStatus {
    pub has_ytdlp: bool,
    pub has_mpv: bool,
    pub has_mpvnet: bool,
    pub has_celluloid: bool,
    pub has_ffmpeg: bool,
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Cross-platform executable discovery (`PATH` + standard Homebrew / App bundle / WinGet paths).
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(path_os) = env::var_os("PATH") {
        for dir in env::split_paths(&path_os) {
            #[cfg(windows)]
            for ext in ["exe", "cmd", "bat"] {
                let cand = dir.join(format!("{}.{}", name, ext));
                if is_executable(&cand) {
                    return Some(cand);
                }
            }
            #[cfg(not(windows))]
            {
                let cand = dir.join(name);
                if is_executable(&cand) {
                    return Some(cand);
                }
            }
        }
    }

    // Standard fallback locations (useful when launched from GUI / minimal PATH)
    let mut extras = vec![
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ];
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        extras.push(home.join(".local/bin").join(name));
        extras.push(home.join(".cargo/bin").join(name));
    }

    #[cfg(target_os = "macos")]
    {
        if name == "mpv" {
            extras.push(PathBuf::from("/Applications/mpv.app/Contents/MacOS/mpv"));
        }
    }

    #[cfg(windows)]
    {
        if name == "mpvnet" || name == "mpv.net" {
            if let Some(local) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                extras.push(local.join("Programs/mpv.net/mpvnet.exe"));
            }
            extras.push(PathBuf::from(r"C:\Program Files\mpv.net\mpvnet.exe"));
        } else if name == "mpv" {
            extras.push(PathBuf::from(r"C:\Program Files\mpv\mpv.exe"));
        }
    }

    extras.into_iter().find(|p| is_executable(p))
}

pub fn detect_dependencies() -> DependencyStatus {
    let has_ytdlp = find_binary("yt-dlp").is_some();
    let has_mpv = find_binary("mpv").is_some();
    let has_mpvnet = find_binary("mpvnet").is_some() || find_binary("mpv.net").is_some();
    let has_celluloid = find_binary("celluloid").is_some();
    let has_ffmpeg = find_binary("ffmpeg").is_some();

    DependencyStatus {
        has_ytdlp,
        has_mpv,
        has_mpvnet,
        has_celluloid,
        has_ffmpeg,
    }
}

static DEPS_CACHE: OnceLock<DependencyStatus> = OnceLock::new();

/// Process-wide cached dependency status (avoids repeated `stat` calls inside 60fps UI frames).
pub fn cached_dependencies() -> &'static DependencyStatus {
    DEPS_CACHE.get_or_init(detect_dependencies)
}

/// Returns whether a playback engine (or at least one host backend for `AutoBest`) is installed.
pub fn is_player_installed(player: Player) -> bool {
    let d = cached_dependencies();
    match player {
        Player::AutoBest => {
            d.has_mpv
                || (cfg!(windows) && d.has_mpvnet)
                || (cfg!(target_os = "linux") && d.has_celluloid)
        }
        Player::Mpv => d.has_mpv,
        Player::MpvNet => d.has_mpvnet,
        Player::Celluloid => d.has_celluloid,
    }
}

#[derive(Debug, Clone)]
pub struct PlayOptions {
    pub video_id: String,
    pub title: String,
    /// Either a local file path (for offline vault playback) or None (uses YouTube watch URL).
    pub local_file: Option<PathBuf>,
    pub quality_format: String,
    pub player: Player,
    pub cookies_path: Option<PathBuf>,
    pub start_pos: Option<f64>,
    pub ipc_socket: Option<String>,
    pub subtitles_enabled: bool,
    pub sub_lang: Option<String>,
    pub sub_url: Option<String>,
    pub sub_style: SubtitleStyleConfig,
    pub shader_mode: VideoShaderMode,
    pub mastering_profile: VideoMasteringProfile,
    pub performance_mode: PerformanceMode,
    pub manual_mastering: ManualMasteringConfig,
    pub motion_mode: MotionMode,
    pub dub_preference: DubPreference,
    pub dub_lang: String,
    pub audio_multistreams: bool,
    pub audio_loudnorm: bool,
    pub audio_filter_chain: Option<String>,
    pub is_live: bool,
    pub live_mode: LiveStreamMode,
    pub live_from_start: bool,
    pub live_auto_catchup: bool,
    pub network_mode: NetworkMode,
    pub force_ipv4: bool,
    pub auto_fullscreen: bool,
    pub auto_pip: bool,
    pub loading_art_dir: Option<String>,
    pub loading_variant_idx: usize,
    pub loading_channel: String,
    pub loading_quality: String,
    pub picture: PictureGrade,
    pub direct_stream: Option<SelectedStreamPair>,
}

pub const LOUDNORM_FILTER: &str = "lavfi=[dynaudnorm=f=150:g=15:p=0.9]";

/// Compose one non-destructive FFmpeg/mpv `af` filter chain combining Preamp, Highpass,
/// 10-band parametric EQ, Night Loudnorm (`dynaudnorm`), and Spatial/Channel DSP.
pub fn build_mpv_audio_filter_chain(
    bands_db: [i8; 10],
    preamp_db: i8,
    eq_bypass: bool,
    channel_mode: AudioChannelMode,
    voice_highpass: bool,
    loudnorm: bool,
) -> Option<String> {
    let mut nodes: Vec<String> = Vec::new();
    let preamp = preamp_db.clamp(-6, 6);
    if !eq_bypass && preamp != 0 {
        nodes.push(format!("volume={preamp}dB"));
    }
    if voice_highpass {
        nodes.push("highpass=f=80".to_string());
    }
    if !eq_bypass {
        for (freq, gain) in EQ_BAND_FREQS_HZ.into_iter().zip(bands_db) {
            let g = gain.clamp(-12, 12);
            if g != 0 {
                nodes.push(format!("equalizer=f={freq}:t=q:w=1.4:g={g}"));
            }
        }
    }
    if loudnorm {
        nodes.push("dynaudnorm=f=150:g=15:p=0.9".to_string());
    }
    match channel_mode {
        AudioChannelMode::Stereo => {}
        AudioChannelMode::Mono => nodes.push("pan=mono|c0=0.5*c0+0.5*c1".to_string()),
        AudioChannelMode::Crossfeed => nodes.push("crossfeed=strength=0.45:range=0.5".to_string()),
        AudioChannelMode::WideStage => nodes.push("extrastereo=m=1.45".to_string()),
    }
    if nodes.is_empty() {
        None
    } else {
        Some(format!("lavfi=[{}]", nodes.join(",")))
    }
}

const YOUTUBE_INPUT_CONF: &str = r#"# Phaenna YouTube-Parity Player Keybindings (works windowed & fullscreen)
k cycle pause
SPACE cycle pause
j seek -10
l seek 10
LEFT seek -5
RIGHT seek 5
UP add volume 5
DOWN add volume -5
m cycle mute
f script-binding phaenna-toggle-fullscreen
F script-binding phaenna-toggle-fullscreen
ESC set fullscreen no
c cycle sub-visibility
< add speed -0.25
> add speed 0.25
. frame-step
, frame-back-step
[ add chapter -1
] add chapter 1
0 seek 0 absolute-percent
1 seek 10 absolute-percent
2 seek 20 absolute-percent
3 seek 30 absolute-percent
4 seek 40 absolute-percent
5 seek 50 absolute-percent
6 seek 60 absolute-percent
7 seek 70 absolute-percent
8 seek 80 absolute-percent
9 seek 90 absolute-percent
HOME seek 0 absolute
END seek 99 absolute-percent
d cycle aid
p script-binding phaenna-toggle-pip
P script-binding phaenna-cycle-pip-corner
i script-binding phaenna-toggle-hud
I script-binding phaenna-pin-hud
g script-binding phaenna-cycle-governor
F1 script-binding stats/display-stats-toggle
L script-binding phaenna-snap-live-edge
"#;

const PHAENNA_LIVE_LUA: &str = r#"-- Phaenna Live Edge Controller: automatic drift catch-up (with DVR rewind guard) & Shift+L snap
local catchup_on = (mp.get_opt("phaenna_live_catchup") or "1") == "1"
local trigger_lag = tonumber(mp.get_opt("phaenna_live_trigger") or "12.0") or 12.0
local target_lag = tonumber(mp.get_opt("phaenna_live_target") or "4.5") or 4.5
local catchup_spd = tonumber(mp.get_opt("phaenna_live_speed") or "1.04") or 1.04
local catching_up = false

local function max_seekable_end()
    local st = mp.get_property_native("demuxer-cache-state")
    if not st or not st["seekable-ranges"] then return nil end
    local best = nil
    for _, r in ipairs(st["seekable-ranges"]) do
        if r["end"] and (best == nil or r["end"] > best) then
            best = r["end"]
        end
    end
    return best
end

local function jump_to_live_edge()
    local max_end = max_seekable_end()
    if max_end then
        local dest = math.max(0.0, max_end - target_lag)
        mp.commandv("seek", dest, "absolute")
        mp.set_property_number("speed", 1.0)
        catching_up = false
        mp.osd_message(string.format("LIVE EDGE (-%.1fs)", target_lag), 2.0)
    else
        mp.osd_message("LIVE EDGE (buffering manifest...)", 1.5)
    end
end

mp.add_forced_key_binding("L", "phaenna-snap-live-edge", jump_to_live_edge)

mp.add_periodic_timer(1.5, function()
    if not catchup_on then return end
    if mp.get_property_bool("pause", false) then return end
    local pos = mp.get_property_number("time-pos", nil)
    local max_end = max_seekable_end()
    if not pos or not max_end then return end
    local lag = max_end - pos
    -- Respect intentional DVR rewinding (>30s behind): never rush user back against their will.
    if lag > 30.0 then
        if catching_up then
            mp.set_property_number("speed", 1.0)
            catching_up = false
        end
        return
    end
    if lag >= trigger_lag then
        if not catching_up then
            catching_up = true
            mp.set_property_number("speed", catchup_spd)
        end
    elseif catching_up and lag <= target_lag then
        catching_up = false
        mp.set_property_number("speed", 1.0)
    end
end)
"#;

const PHAENNA_LOADER_LUA: &str = r##"-- Phaenna Unified Player Overlay Engine:
--   1. Solus-Style Y2K Anime Loading Card & Buffering Overlay
--   2. Floating Borderless Picture-in-Picture Controller ([p] toggle, [P] cycle corner/size)
--   3. Multi-Color Curated Bento Info HUD ([i] 6s toggle, [I] 1Hz pin, [F1] raw stats)
local ov = mp.create_osd_overlay("ass-events")
ov.res_x = 1280
ov.res_y = 720

local hud_ov = mp.create_osd_overlay("ass-events")
hud_ov.res_x = 1280
hud_ov.res_y = 720

local title = mp.get_opt("phaenna_title") or "Preparing Stream"
local channel = mp.get_opt("phaenna_channel") or ""
local qual = mp.get_opt("phaenna_quality") or "HD"
local meta = mp.get_opt("phaenna_meta") or "PHAENNA STUDIO"
local card_dir = mp.get_opt("phaenna_card_dir") or ""
local card_count = math.max(1, tonumber(mp.get_opt("phaenna_card_count") or "11") or 11)
local card_idx = (tonumber(mp.get_opt("phaenna_card_idx") or "0") or 0) % card_count
local pip_active = (mp.get_opt("phaenna_auto_pip") or "0") == "1"

local function esc_ass(s)
    if not s then return "" end
    return tostring(s):gsub("[{}]", ""):gsub("[\r\n]+", " ")
end

title = esc_ass(title)
channel = esc_ass(channel)
qual = esc_ass(qual)
meta = esc_ass(meta)

local function wrap_two_lines(str, max_col)
    if #str <= max_col then return str, "" end
    local cut = max_col
    for i = max_col, math.max(12, max_col - 14), -1 do
        if string.sub(str, i, i) == " " then
            cut = i
            break
        end
    end
    local l1 = string.sub(str, 1, cut):gsub("%s+$", "")
    local l2 = string.sub(str, cut + 1):gsub("^%s+", "")
    if #l2 > max_col then l2 = string.sub(l2, 1, max_col - 3) .. "..." end
    return l1, l2
end

local title_l1, title_l2 = wrap_two_lines(title, 38)

-- =============================================================================
-- 1. Unique Y2K Anime Portrait Loader & Mid-Stream Buffering Overlay
--    Uses mpv `overlay-add` (bitmap above ASS) for the crisp left 460x460 Y2K
--    portrait and `ass-events` for the unified CRT frame, status & live bar.
--    No rectangular fallback box — every stall rotates to a unique anime card!
-- =============================================================================
local y2k_themes = {
    { tag = "CRT PHOSPHOR",  col = "&H87E76E&" },
    { tag = "CYBER AUDIO",   col = "&HFAA560&" },
    { tag = "INK SKETCH",    col = "&HEBE7E5&" },
    { tag = "MIDNIGHT DECK", col = "&HFCB4A5&" },
    { tag = "MINT STAR",     col = "&HACEF86&" },
    { tag = "NEON CAT",      col = "&HF88C81&" },
    { tag = "HALFTONE STAR", col = "&HE1D5CB&" },
    { tag = "CYBER VIOLET",  col = "&HFC84C0&" },
    { tag = "CYAN HALO",     col = "&HD4EA5E&" },
    { tag = "ICE MATRIX",    col = "&HFDC593&" },
    { tag = "TEAL SIGNAL",   col = "&HBFD42D&" },
}

local OVERLAY_ID = 61
local phase = 0.0
local cache_pct = nil
local cache_speed = 0
local cache_dur = 0
local is_loading = true
local is_initial_boot = true
local timer = nil
local overlay_shown = false
local shown_card = -1
local shown_x = -1
local shown_y = -1
local shown_sz = -1

local function fmt_speed(bps)
    if not bps or bps <= 0 then return "" end
    if bps >= 1048576 then
        return string.format("%.1f MB/s", bps / 1048576.0)
    elseif bps >= 1024 then
        return string.format("%d KB/s", math.floor(bps / 1024.0))
    end
    return string.format("%d B/s", math.floor(bps))
end

local function file_exists(p)
    if not p or p == "" then return false end
    local f = io.open(p, "rb")
    if f then f:close(); return true end
    return false
end

local function portrait_path_for(idx)
    if card_dir == "" then return nil end
    local p = string.format("%s/y2k_%d.bgra", card_dir, idx % card_count)
    if file_exists(p) then return p end
    return nil
end

local function advance_unique_card()
    for step = 1, card_count do
        local candidate = (card_idx + step) % card_count
        if portrait_path_for(candidate) then
            card_idx = candidate
            return
        end
    end
    card_idx = (card_idx + 1) % card_count
end

local function clear_bitmap_overlay()
    if overlay_shown then
        mp.command_native({ name = "overlay-remove", id = OVERLAY_ID })
        overlay_shown = false
        shown_card = -1
        shown_x = -1
        shown_y = -1
        shown_sz = -1
    end
end

local function render_loader()
    if not is_loading then
        clear_bitmap_overlay()
        ov:remove()
        return
    end

    local osd_w = mp.get_property_number("osd-width", 0) or 0
    local osd_h = mp.get_property_number("osd-height", 0) or 0
    if osd_w <= 0 or osd_h <= 0 then return end

    ov.res_x = osd_w
    ov.res_y = osd_h

    local sc = math.min(osd_w / 1280.0, osd_h / 720.0)
    local dw = math.floor(1280 * sc)
    local dh = math.floor(720 * sc)
    local ox = math.floor((osd_w - dw) / 2)
    local oy = math.floor((osd_h - dh) / 2)
    local function px(x) return math.floor(ox + x * sc) end
    local function py(y) return math.floor(oy + y * sc) end
    local function ps(v) return math.max(1, math.floor(v * sc)) end

    local th = y2k_themes[(card_idx % #y2k_themes) + 1]

    -- Show or reposition left 460x460 Y2K anime portrait via GPU overlay-add
    local p_path = portrait_path_for(card_idx)
    local art_x = px(84)
    local art_y = py(130)
    local art_sz = math.max(32, ps(460))
    if p_path then
        if (not overlay_shown) or shown_card ~= card_idx or shown_x ~= art_x or shown_y ~= art_y or shown_sz ~= art_sz then
            mp.command_native({
                name = "overlay-add",
                id = OVERLAY_ID,
                x = art_x,
                y = art_y,
                file = p_path,
                offset = 0,
                fmt = "bgra",
                w = 460,
                h = 460,
                stride = 460 * 4,
                dw = art_sz,
                dh = art_sz,
            })
            overlay_shown = true
            shown_card = card_idx
            shown_x = art_x
            shown_y = art_y
            shown_sz = art_sz
        end
    else
        clear_bitmap_overlay()
    end

    phase = (phase + 0.05) % 1.0

    local bar_x = px(594)
    local bar_y = py(392)
    local bar_w = math.max(40, ps(580))
    local bar_h = math.max(6, ps(22))
    local pad = math.max(2, ps(3))
    local track_w = bar_w - pad * 2
    local track_h = bar_h - pad * 2

    local fill_off = 0
    local fill_w = math.max(12, math.floor(track_w * 0.28))
    local spd_txt = fmt_speed(cache_speed)
    local spd_sfx = (spd_txt ~= "") and ("  ·  " .. spd_txt) or ""
    local dur_txt = (cache_dur and cache_dur > 0.1) and string.format("  ·  %.1fs cached", cache_dur) or ""
    local status_line = (is_initial_boot and "Connecting HD Stream" or "Re-buffering Stream") .. spd_sfx .. dur_txt .. "..."

    if cache_pct and cache_pct > 0 and cache_pct < 100 then
        fill_off = 0
        fill_w = math.max(12, math.floor(track_w * (cache_pct / 100.0)))
        status_line = string.format("Buffering %d%%%s%s", math.floor(cache_pct), spd_sfx, dur_txt)
    else
        local pulse_w = math.max(20, math.floor(track_w * 0.30))
        fill_off = math.floor(phase * (track_w - pulse_w))
        fill_w = pulse_w
    end

    local headline = is_initial_boot and "NOW LOADING..." or "BUFFERING STREAM..."
    local sub_info = ((channel ~= "") and (channel .. "  ·  ") or "") .. qual .. "  ·  " .. meta
    if #sub_info > 54 then sub_info = string.sub(sub_info, 1, 51) .. "..." end

    local bx0, by0 = px(24), py(24)
    local bx1, by1 = px(1256), py(696)
    local cl = ps(26)
    local ct = math.max(2, ps(3))
    local hud_x = px(594)
    local title_y2 = py(328)
    local card_badge = string.format("CARD %02d/%02d", (card_idx % card_count) + 1, card_count)

    local ass = string.format(
        -- Full-viewport CRT obsidian canvas
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H140E0B&\\1a&H0A&\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}\n" ..
        -- Thin CRT inner frame
        "{\\an7\\pos(0,0)\\bord1\\shad0\\1a&HFF&\\3c&H3D3630&\\p1}m %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        -- Four neon corner brackets
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c%s\\p1}m %d %d l %d %d l %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c%s\\p1}m %d %d l %d %d l %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c%s\\p1}m %d %d l %d %d l %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c%s\\p1}m %d %d l %d %d l %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        -- Top badge bar + card theme tag + sequence indicator
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c%s\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c&H100E0C&}PHAENNA // STUDIO\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c%s}%s   {\\1c&HA69A8E&}%s\n" ..
        -- Main Status Headline
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c&HFAF4F0&}%s\n" ..
        -- Channel + Quality + Playback Profile
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c%s}%s\n" ..
        -- Wrapped Video Title
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c&HFAF4F0&}%s\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\1c&HFAF4F0&}%s\n" ..
        -- Progress bar track outline + animated neon fill
        "{\\an7\\pos(%d,%d)\\bord1\\shad0\\1c&H1C1814&\\3c%s\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c%s\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}\n" ..
        -- Live Buffering Speed / Progress Text
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c&HC8B8A4&}%s\n" ..
        -- Bottom divider + Phaenna Player Keybindings Legend
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c&H3D3630&\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\b1\\1c%s}PHAENNA PLAYER KEYS:\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\1c&HFAF4F0&}K / SPACE  Play-Pause     J / L  -10s / +10s     I  Info HUD\n" ..
        "{\\an7\\pos(%d,%d)\\bord0\\shad0\\fs%d\\1c&HA69A8E&}P  PiP Float          Shift+P  Corner          F  Fullscreen",
        osd_w, osd_w, osd_h, osd_h,
        bx0, by0, bx1, by0, bx1, by1, bx0, by1,
        th.col, bx0, by0, bx0 + cl, by0, bx0 + cl, by0 + ct, bx0 + ct, by0 + ct, bx0 + ct, by0 + cl, bx0, by0 + cl,
        th.col, bx1 - cl, by0, bx1, by0, bx1, by0 + cl, bx1 - ct, by0 + cl, bx1 - ct, by0 + ct, bx1 - cl, by0 + ct,
        th.col, bx0, by1 - cl, bx0 + ct, by1 - cl, bx0 + ct, by1 - ct, bx0 + cl, by1 - ct, bx0 + cl, by1, bx0, by1,
        th.col, bx1 - ct, by1 - cl, bx1, by1 - cl, bx1, by1, bx1 - cl, by1, bx1 - cl, by1 - ct, bx1 - ct, by1 - ct,
        hud_x, py(146), th.col, ps(270), ps(270), ps(34), ps(34),
        hud_x + ps(14), py(152), math.max(11, ps(20)),
        hud_x + ps(288), py(154), math.max(10, ps(17)), th.col, th.tag, card_badge,
        hud_x, py(198), math.max(16, ps(36)), headline,
        hud_x, py(252), math.max(10, ps(18)), th.col, sub_info,
        hud_x, py(288), math.max(12, ps(24)), title_l1,
        hud_x, title_y2, math.max(12, ps(24)), title_l2,
        bar_x, bar_y, th.col, bar_w, bar_w, bar_h, bar_h,
        bar_x + pad + fill_off, bar_y + pad, th.col, fill_w, fill_w, track_h, track_h,
        hud_x, py(426), math.max(10, ps(18)), status_line,
        hud_x, py(486), bar_w, bar_w, math.max(1, ps(2)), math.max(1, ps(2)),
        hud_x, py(504), math.max(10, ps(16)), th.col,
        hud_x, py(530), math.max(9, ps(16)),
        hud_x, py(556), math.max(9, ps(16))
    )
    ov.data = ass
    ov:update()
end

local function start_anim()
    if not timer then
        timer = mp.add_periodic_timer(0.06, render_loader)
    end
    render_loader()
end

local function stop_anim()
    is_loading = false
    is_initial_boot = false
    if timer then
        timer:kill()
        timer = nil
    end
    clear_bitmap_overlay()
    ov:remove()
end

mp.register_event("playback-restart", function()
    local pfc = mp.get_property_bool("paused-for-cache", false)
    if not pfc then stop_anim() end
end)

mp.observe_property("paused-for-cache", "bool", function(_, val)
    if val then
        if not is_loading then
            is_initial_boot = false
            advance_unique_card()
            is_loading = true
            phase = 0.0
            start_anim()
        end
    else
        local t = mp.get_property_number("time-pos", 0)
        if t and t > 0.02 then stop_anim() end
    end
end)

mp.observe_property("time-pos", "number", function(_, val)
    if val and val > 0.04 and not mp.get_property_bool("paused-for-cache", false) then
        stop_anim()
    end
end)

mp.observe_property("osd-dimensions", "native", function()
    if is_loading then render_loader() end
end)

mp.observe_property("cache-buffering-state", "number", function(_, val) cache_pct = val end)
mp.observe_property("cache-speed", "number", function(_, val) cache_speed = val or 0 end)
mp.observe_property("demuxer-cache-duration", "number", function(_, val) cache_dur = val or 0 end)

-- =============================================================================
-- 2. Picture-in-Picture (PiP) & Fullscreen Controller ([p] / [P] / [f])
-- =============================================================================
local pip_idx = 1
local pip_presets = {
    { label = "Bottom-Right 30%", scale = 0.30, geom = "30%x30%-24-24" },
    { label = "Top-Right 30%",    scale = 0.30, geom = "30%x30%-24+24" },
    { label = "Bottom-Left 30%",  scale = 0.30, geom = "30%x30%+24-24" },
    { label = "Top-Left 30%",     scale = 0.30, geom = "30%x30%+24+24" },
    { label = "Compact 22%",      scale = 0.22, geom = "22%x22%-20-20" },
    { label = "Theater 38%",      scale = 0.38, geom = "38%x38%-28-28" },
}

local function apply_pip_preset()
    local pr = pip_presets[pip_idx]
    mp.set_property_bool("fullscreen", false)
    mp.set_property_bool("ontop", true)
    mp.set_property_bool("border", false)
    mp.set_property_bool("keepaspect-window", true)
    mp.set_property_number("window-scale", pr.scale)
    mp.set_property("geometry", pr.geom)
    mp.osd_message("◈ PiP Floating: " .. pr.label .. " ([P] corner, [p] exit)", 2.2)
end

local function exit_pip_window_props()
    pip_active = false
    mp.set_property_bool("ontop", false)
    mp.set_property_bool("border", true)
    mp.set_property_number("window-scale", 1.0)
    mp.set_property("geometry", "50%:50%")
end

local function toggle_pip()
    pip_active = not pip_active
    if pip_active then
        apply_pip_preset()
    else
        exit_pip_window_props()
        mp.osd_message("◈ PiP Floating: OFF (Standard Window)", 2.0)
    end
    mp.commandv("script-message", "phaenna-ipc-toggle-pip", pip_active and "1" or "0", tostring(pip_idx - 1))
end

local function cycle_pip_corner()
    if not pip_active then
        pip_active = true
    else
        pip_idx = (pip_idx % #pip_presets) + 1
    end
    apply_pip_preset()
    mp.commandv("script-message", "phaenna-ipc-cycle-pip", tostring(pip_idx - 1))
end

local function toggle_fullscreen()
    local next_fs = not mp.get_property_bool("fullscreen", false)
    if pip_active then
        next_fs = true
        exit_pip_window_props()
    end
    mp.set_property_bool("fullscreen", next_fs)
    mp.commandv("script-message", "phaenna-ipc-toggle-fullscreen", next_fs and "1" or "0")
end

mp.register_script_message("phaenna-pip-sync", function(active_s, idx_s)
    local next_active = (active_s == "1" or active_s == "true")
    local idx_num = tonumber(idx_s)
    if idx_num ~= nil then
        pip_idx = (math.floor(idx_num) % #pip_presets) + 1
    end
    pip_active = next_active
    if hud_visible then render_info_hud() end
end)

mp.add_forced_key_binding("p", "phaenna-toggle-pip", toggle_pip)
mp.add_forced_key_binding("P", "phaenna-cycle-pip-corner", cycle_pip_corner)
mp.add_forced_key_binding("f", "phaenna-toggle-fullscreen", toggle_fullscreen)
mp.add_forced_key_binding("F", "phaenna-toggle-fullscreen-upper", toggle_fullscreen)

-- =============================================================================
-- 3 & 4. Dynamic Thermal/BPP/Cadence Governor + Multi-Color Bento HUD
-- =============================================================================
local gov_mode = mp.get_opt("phaenna_gov") or "auto"
local perf_mode = mp.get_opt("phaenna_perf") or "BAL"
local base_grain = tonumber(mp.get_opt("phaenna_grain") or "0.008") or 0.008
local has_shader_opts = (mp.get_opt("phaenna_shader_opts") or "1") == "1"
local motion_mode = mp.get_opt("phaenna_motion") or "cadence"
local device_lbl = esc_ass(mp.get_opt("phaenna_device") or "Host Auto")

local host_on_battery = false
local host_batt_pct = nil
local host_therm_tier = 0
local host_temp_c = nil

local gov_tier = 0
local last_drops = 0
local last_step_up_time = 0
local live_bpp = 0.0
local live_scale_ratio = 1.0
local live_deb_amt = 0.25
local live_grain_amt = base_grain
local live_cadence_on = false
local live_gov_reason = "COOL"

local prev_shader_opts = nil
local prev_deband = nil
local prev_deband_iters = nil
local prev_interp = nil
local prev_sync = nil
local prev_peak = nil

local function set_prop_diff(cache_val, prop, new_val, is_bool)
    if cache_val == new_val then return cache_val end
    if is_bool then
        mp.set_property_bool(prop, new_val)
    else
        mp.set_property(prop, tostring(new_val))
    end
    return new_val
end

local function governor_tier_badge()
    if gov_mode == "off" then return "GOV: OFF" end
    if gov_tier == 3 then return "GOV: PiP/MIN SLEEP" end
    if gov_tier == 2 then return "GOV: TIER 2 THROTTLE" end
    if gov_tier == 1 then return "GOV: TIER 1 COOL" end
    return "GOV: TIER 0 · OPTIMAL"
end

local function governor_tick()
    local vw = mp.get_property_number("width", 0) or 0
    local vh = mp.get_property_number("height", 0) or 0
    local fps = mp.get_property_number("estimated-vf-fps", 0) or 0
    if fps <= 0 then fps = mp.get_property_number("container-fps", 0) or 0 end
    local vbit = mp.get_property_number("video-bitrate", 0) or 0
    local disp_fps = mp.get_property_number("display-fps", 0) or 0
    local osd_w = mp.get_property_number("osd-width", 0) or 0
    local sig_peak = mp.get_property_number("video-params/sig-peak", 1.0) or 1.0
    local pixfmt = tostring(mp.get_property("video-params/pixelformat", "") or ""):lower()

    if vw > 0 and vh > 0 and fps > 0 and vbit > 0 then
        live_bpp = vbit / (vw * vh * fps)
    else
        live_bpp = 0.085
    end
    live_scale_ratio = (vw > 0 and osd_w > 0) and (osd_w / vw) or 1.0

    if gov_mode == "off" then
        live_gov_reason = "Static"
        return
    end

    local minimized = mp.get_property_bool("window-minimized", false)
    local drops_now = (mp.get_property_number("decoder-frame-drop-count", 0) or 0)
                    + (mp.get_property_number("frame-drop-count", 0) or 0)
    local drop_delta = math.max(0, drops_now - last_drops)
    last_drops = drops_now

    local want_tier = 0
    live_gov_reason = "Optimal"
    if minimized or pip_active then
        want_tier = 3
        live_gov_reason = minimized and "Minimized" or "PiP Saver"
    elseif gov_mode == "max" then
        want_tier = 0
        live_gov_reason = "Max Lock"
    else
        if drop_delta >= 3 then
            want_tier = math.max(want_tier, 2)
            live_gov_reason = "Drop Guard"
        end
        if host_therm_tier >= 2 then
            want_tier = math.max(want_tier, 2)
            live_gov_reason = "Thermal Guard"
        elseif host_therm_tier == 1 or gov_mode == "cool" or host_on_battery then
            want_tier = math.max(want_tier, 1)
            live_gov_reason = host_on_battery and "Battery Opt" or "Cool Guard"
        end
    end

    local now_t = mp.get_time()
    if want_tier > gov_tier then
        gov_tier = want_tier
        last_step_up_time = now_t
    elseif want_tier < gov_tier then
        if want_tier == 0 and gov_tier < 3 and (now_t - last_step_up_time) < 6.0 then
            -- 6-second hysteresis prevents rapid thermal oscillation
        else
            gov_tier = want_tier
        end
    end

    local is_hdr = sig_peak > 1.05
    local is_10bit = (pixfmt:find("10") ~= nil) or (pixfmt:find("p010") ~= nil) or (pixfmt:find("16") ~= nil)

    local deb = 0.0
    local det = 0.0
    local cas = 0.22
    local grn = 0.0

    if minimized then
        deb = 0.0; det = 0.0; cas = 0.0; grn = 0.0
    elseif perf_mode == "ECO" or pip_active then
        deb = 0.0; det = 0.0; cas = pip_active and 0.12 or 0.14; grn = 0.0
    else
        -- Clean high-BPP 4K needs zero blurring; starved 1080p gets strong deblock
        if live_bpp >= 0.14 then
            deb = 0.00
        elseif live_bpp >= 0.09 then
            deb = 0.22
        elseif live_bpp >= 0.06 then
            deb = 0.36
        else
            deb = 0.56
        end
        if gov_tier >= 2 then deb = deb * 0.5 end

        if perf_mode == "MAX" then
            det = (gov_tier == 0) and 0.12 or ((gov_tier == 1) and 0.07 or 0.0)
            cas = (live_scale_ratio > 1.25) and 0.30 or ((live_scale_ratio > 1.02) and 0.24 or 0.16)
        else
            det = (gov_tier == 0) and 0.08 or 0.0
            cas = (live_scale_ratio > 1.25) and 0.24 or 0.18
        end
        if gov_tier >= 2 then cas = 0.14 end

        if gov_tier < 2 then
            grn = base_grain
            if (not is_10bit) and live_bpp < 0.08 and grn > 0 then
                grn = math.min(0.016, grn + 0.002)
            end
        end
    end

    live_deb_amt = deb
    live_grain_amt = grn

    if has_shader_opts then
        local opts_str = string.format(
            "deblock_amt=%.2f,cas_amt=%.2f,detail_lift=%.2f,grain_amt=%.3f",
            deb, cas, det, grn
        )
        prev_shader_opts = set_prop_diff(prev_shader_opts, "glsl-shader-opts", opts_str, false)
    end

    if perf_mode ~= "ECO" then
        local want_deband = (not minimized)
            and (not pip_active)
            and (gov_tier < 2)
            and (not (is_10bit and live_bpp >= 0.14))
        local deband_s = want_deband and "yes" or "no"
        prev_deband = set_prop_diff(prev_deband, "deband", deband_s, false)
        if want_deband then
            local iters = (perf_mode == "MAX" and live_bpp < 0.10 and gov_tier == 0) and 2 or 1
            if prev_deband_iters ~= iters then
                mp.set_property_number("deband-iterations", iters)
                prev_deband_iters = iters
            end
        end

        local want_peak = (is_hdr and gov_tier == 0) and "yes" or "no"
        prev_peak = set_prop_diff(prev_peak, "hdr-compute-peak", want_peak, false)

        if motion_mode == "cadence" then
            local ratio = (disp_fps > 0 and fps > 0) and (disp_fps / fps) or 1.0
            local nearest = math.max(1.0, math.floor(ratio + 0.5))
            local is_integer_cadence = (math.abs(ratio - nearest) / nearest) <= 0.018
            local want_interp = (not is_integer_cadence) and (not pip_active) and (not minimized) and (gov_tier == 0)
            live_cadence_on = want_interp
            local sync_mode = want_interp and "display-resample" or "audio"
            prev_sync = set_prop_diff(prev_sync, "video-sync", sync_mode, false)
            prev_interp = set_prop_diff(prev_interp, "interpolation", want_interp, true)
        elseif motion_mode == "smooth120" then
            local want_smooth = (not pip_active) and (not minimized) and (gov_tier < 2)
            live_cadence_on = want_smooth
            prev_sync = set_prop_diff(prev_sync, "video-sync", want_smooth and "display-resample" or "audio", false)
            prev_interp = set_prop_diff(prev_interp, "interpolation", want_smooth, true)
        end
    end
end

local function cycle_governor_mode()
    if gov_mode == "auto" then
        gov_mode = "cool"
    elseif gov_mode == "cool" then
        gov_mode = "max"
    elseif gov_mode == "max" then
        gov_mode = "off"
    else
        gov_mode = "auto"
    end
    governor_tick()
    mp.osd_message("◈ Dynamic Governor: " .. string.upper(gov_mode) .. " (" .. live_gov_reason .. ")", 2.2)
end

mp.add_forced_key_binding("g", "phaenna-cycle-governor", cycle_governor_mode)
mp.register_script_message("phaenna-set-governor", function(mode)
    if mode and mode ~= "" then
        gov_mode = mode
        governor_tick()
    end
end)
mp.register_script_message("phaenna-host-thermal", function(batt_s, pct_s, tier_s, temp_s)
    host_on_battery = (batt_s == "1" or batt_s == "true")
    host_batt_pct = tonumber(pct_s or "")
    host_therm_tier = tonumber(tier_s or "0") or 0
    host_temp_c = tonumber(temp_s or "")
    governor_tick()
end)

mp.add_periodic_timer(2.0, governor_tick)
mp.register_event("file-loaded", governor_tick)

-- =============================================================================
-- Multi-Color Curated Bento Video Info HUD ([i] 6s / [I] Pinned 1Hz)
-- =============================================================================
local hud_visible = false
local hud_pinned = false
local hud_tick_timer = nil
local hud_hide_timer = nil

local function fmt_clock(s)
    if not s or s < 0 then return "--:--" end
    local tot = math.floor(s)
    local h = math.floor(tot / 3600)
    local m = math.floor((tot % 3600) / 60)
    local sec = tot % 60
    if h > 0 then
        return string.format("%d:%02d:%02d", h, m, sec)
    end
    return string.format("%02d:%02d", m, sec)
end

local function hide_info_hud()
    hud_visible = false
    hud_pinned = false
    if hud_tick_timer then hud_tick_timer:kill(); hud_tick_timer = nil end
    if hud_hide_timer then hud_hide_timer:kill(); hud_hide_timer = nil end
    hud_ov:remove()
end

local function render_info_hud()
    if not hud_visible then
        hud_ov:remove()
        return
    end
    local mtitle = esc_ass(mp.get_property("media-title", title))
    if #mtitle > 68 then mtitle = string.sub(mtitle, 1, 65) .. "..." end

    local pos = mp.get_property_number("time-pos", 0) or 0
    local dur = mp.get_property_number("duration", 0) or 0
    local pct = mp.get_property_number("percent-pos", 0) or 0
    local spd = mp.get_property_number("speed", 1.0) or 1.0
    local vw = mp.get_property_number("width", 0) or 0
    local vh = mp.get_property_number("height", 0) or 0
    local fps = mp.get_property_number("estimated-vf-fps", 0) or 0
    local disp_fps = mp.get_property_number("display-fps", 0) or 0
    local vcodec = esc_ass(mp.get_property("video-codec", "—")):upper()
    local vbit = mp.get_property_number("video-bitrate", 0) or 0
    local hwdec = esc_ass(mp.get_property("hwdec-current", ""))
    local drops = (mp.get_property_number("decoder-frame-drop-count", 0) or 0) + (mp.get_property_number("frame-drop-count", 0) or 0)
    local cm = esc_ass(mp.get_property("video-params/colormatrix", "auto"))

    local acodec = esc_ass(mp.get_property("audio-codec-name", mp.get_property("audio-codec", "—"))):upper()
    local abit = mp.get_property_number("audio-bitrate", 0) or 0
    local sr = mp.get_property_number("audio-params/samplerate", 0) or 0
    local ch_n = mp.get_property_number("audio-params/channel-count", 2) or 2
    local vol = mp.get_property_number("volume", 100) or 100
    local muted = mp.get_property_bool("mute", false)
    local af_s = esc_ass(mp.get_property("af", ""))

    local c_dur = mp.get_property_number("demuxer-cache-duration", 0) or 0
    local c_spd = mp.get_property_number("cache-speed", 0) or 0
    local c_bytes = 0
    local cst = mp.get_property_native("demuxer-cache-state")
    if cst and cst["total-bytes"] then c_bytes = cst["total-bytes"] end

    local chap = esc_ass(mp.get_property("chapter-metadata/by-key/title", ""))
    if #chap > 28 then chap = string.sub(chap, 1, 25) .. "..." end

    local q_tag = (vh >= 2160 and "4K UHD") or (vh >= 1440 and "1440p QHD") or (vh >= 1080 and "1080p FHD") or (vh >= 720 and "720p HD") or "Stream"
    local win_badge = pip_active and "PIP FLOAT" or (mp.get_property_bool("fullscreen", false) and "FULLSCREEN" or "WINDOWED")
    local pin_badge = hud_pinned and "PINNED 1Hz" or "AUTO 6s"
    local gov_badge = governor_tier_badge()
    local hw_lbl = (hwdec ~= "" and hwdec ~= "no") and ("HW ZeroCopy · " .. hwdec) or "CPU Software"
    local hw_col = (hwdec ~= "" and hwdec ~= "no") and "&H99D334&" or "&H5E3FF4&"
    local drop_col = (drops > 0) and "&H5E3FF4&" or "&H99D334&"
    local vol_str = muted and "MUTED" or string.format("%d%%", math.floor(vol))
    local vol_col = muted and "&H5E3FF4&" or "&H99D334&"
    local vbit_s = (vbit > 0) and string.format("%.2f Mbps", vbit / 1000000.0) or "ABR"
    local bpp_lbl = string.format("%.3f bpp", live_bpp)
    local scale_lbl = string.format("%.2fx · %s", live_scale_ratio, cm)
    local shader_lbl = string.format("Deb %.2f · Grn %.3f · %s", live_deb_amt, live_grain_amt, (perf_mode == "ECO") and "CAS" or "4:4:4")
    local gov_detail = string.format("%s · %s", string.upper(gov_mode), live_cadence_on and "SmartSync ON" or "Native Cadence")
    local gov_col = (gov_tier == 0) and "&H99D334&" or ((gov_tier == 1) and "&H24BFFB&" or "&HF8BD38&")
    local host_pwr_lbl = (host_on_battery and ("BATT" .. (host_batt_pct and (" " .. host_batt_pct .. "%%") or "")) or "AC Power")
        .. (host_temp_c and string.format(" · %.0fC", host_temp_c) or (" · " .. live_gov_reason))
    local abit_s = (abit > 0) and string.format("%d kbps", math.floor(abit / 1000.0)) or "Auto"
    local sr_s = (sr > 0) and string.format("%.1f kHz", sr / 1000.0) or "Auto"
    local eq_state = (af_s ~= "") and "Active DSP/EQ" or "Bypassed / Flat"
    local c_mib = c_bytes / 1048576.0
    local spd_txt = fmt_speed(c_spd)
    local buf_health = (c_dur >= 12.0) and "Healthy" or ((c_dur >= 3.0) and "Fair" or "Low")
    local buf_col = (c_dur >= 12.0) and "&H99D334&" or ((c_dur >= 3.0) and "&H24BFFB&" or "&H5E3FF4&")

    local prog_w = math.max(8, math.floor(1140 * math.min(1.0, math.max(0.0, pct / 100.0))))
    local buf_frac = math.min(1.0, c_dur / 60.0)
    local buf_w = math.max(12, math.floor(520 * buf_frac))
    local chap_line = (chap ~= "") and chap or "Continuous Playback"

    -- Glass Bento Card at (x: 42..1238, y: 26..378)
    local ass = string.format(
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H100C09&\\1a&H18&\\p1}m 42 26 l 1238 26 l 1238 378 l 42 378{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord2\\1a&HFF&\\3c&HF8BD38&\\p1}m 42 26 l 1238 26 l 1238 378 l 42 378{\\p0}\n" ..
        "{\\an7\\pos(66,42)\\bord0\\shad0\\fs18\\b1\\1c&HF8BD38&}◈ PHAENNA INFO HUD   " ..
        "{\\1c&H24BFFB&}[%s]  {\\1c&H99D334&}[%s]  {\\1c&HFA8BA7&}[%s]  {\\1c%s}[%s]  {\\1c&HC8B8A4&}%s\n" ..
        "{\\an7\\pos(66,70)\\bord0\\shad0\\fs24\\b1\\1c&HFAF4F0&}%s\n" ..
        "{\\an7\\pos(66,102)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}%s / %s (%d%%)  ·  Speed %.2fx  ·  Chapter: {\\1c&HFAF4F0&}%s\n" ..
        "{\\an7\\pos(66,128)\\bord0\\shad0\\1c&H2B2420&\\p1}m 0 0 l 1140 0 l 1140 6 l 0 6{\\p0}\n" ..
        "{\\an7\\pos(66,128)\\bord0\\shad0\\1c&HF8BD38&\\p1}m 0 0 l %d 0 l %d 6 l 0 6{\\p0}\n" ..
        "{\\an7\\pos(66,150)\\bord0\\shad0\\fs16\\b1\\1c&HF8BD38&}▶ VIDEO & COMPRESSION\n" ..
        "{\\an7\\pos(66,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Stream  : {\\1c&HFAF4F0&\\b1}%dx%d  %s\n" ..
        "{\\an7\\pos(66,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Codec   : {\\1c&H24BFFB&}%s  ·  %s  (%s)\n" ..
        "{\\an7\\pos(66,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Rate    : {\\1c&H99D334&}%.2f fps{\\1c&HC8B8A4&} (Disp %.0fHz)\n" ..
        "{\\an7\\pos(66,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Scale   : {\\1c%s}%d drop {\\1c&HC8B8A4&}· %s\n" ..
        "{\\an7\\pos(460,150)\\bord0\\shad0\\fs16\\b1\\1c&HFA8BA7&}◈ DYNAMIC MASTERING\n" ..
        "{\\an7\\pos(460,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Decode  : {\\1c%s}%s\n" ..
        "{\\an7\\pos(460,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Pipeline: {\\1c&HF8BD38&}%s\n" ..
        "{\\an7\\pos(460,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}DSP/EQ  : {\\1c&HFA8BA7&}%s\n" ..
        "{\\an7\\pos(460,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Governor: {\\1c%s}%s\n" ..
        "{\\an7\\pos(860,150)\\bord0\\shad0\\fs16\\b1\\1c&H24BFFB&}♪ AUDIO & SYSTEM\n" ..
        "{\\an7\\pos(860,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Codec   : {\\1c&H24BFFB&}%s  ·  %s\n" ..
        "{\\an7\\pos(860,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Output  : {\\1c&HFAF4F0&}%s  ·  %d ch\n" ..
        "{\\an7\\pos(860,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Volume  : {\\1c%s}%s\n" ..
        "{\\an7\\pos(860,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Host    : {\\1c&HF8BD38&}%s\n" ..
        "{\\an7\\pos(66,288)\\bord0\\shad0\\fs16\\b1\\1c&H99D334&}⚡ BUFFER HEALTH   {\\1c&HFAF4F0&\\b0}%.1fs ahead  ·  %.1f MiB  %s  ·  {\\1c%s}%s  {\\1c&HC8B8A4&}(%s)\n" ..
        "{\\an7\\pos(66,314)\\bord0\\shad0\\1c&H2B2420&\\p1}m 0 0 l 520 0 l 520 10 l 0 10{\\p0}\n" ..
        "{\\an7\\pos(66,314)\\bord0\\shad0\\1c%s\\p1}m 0 0 l %d 0 l %d 10 l 0 10{\\p0}\n" ..
        "{\\an7\\pos(66,340)\\bord0\\shad0\\fs15\\1c&HC8B8A4&}Keys: {\\1c&HF8BD38&}[i] Hide  [I] Pin 1Hz  {\\1c&H99D334&}[p] PiP Float  [P] Corner  {\\1c&H24BFFB&}[g] Governor  [f] Fullscreen  {\\1c&HFA8BA7&}[F1] Raw Stats",
        q_tag, win_badge, pin_badge, gov_col, gov_badge, meta,
        mtitle,
        fmt_clock(pos), fmt_clock(dur), math.floor(pct), spd, chap_line,
        prog_w, prog_w,
        vw, vh, q_tag,
        vcodec, vbit_s, bpp_lbl,
        fps, disp_fps,
        drop_col, drops, scale_lbl,
        hw_col, hw_lbl,
        shader_lbl,
        eq_state,
        gov_col, gov_detail,
        acodec, abit_s,
        sr_s, ch_n,
        vol_col, vol_str,
        host_pwr_lbl,
        c_dur, c_mib, (spd_txt ~= "") and ("· " .. spd_txt) or "", buf_col, buf_health, device_lbl,
        buf_col, buf_w, buf_w
    )
    hud_ov.data = ass
    hud_ov:update()
end

local function ensure_hud_timer()
    if not hud_tick_timer then
        hud_tick_timer = mp.add_periodic_timer(0.5, render_info_hud)
    end
end

local function toggle_info_hud()
    if hud_visible and not hud_pinned then
        hide_info_hud()
        return
    end
    hud_pinned = false
    hud_visible = true
    if hud_hide_timer then hud_hide_timer:kill() end
    hud_hide_timer = mp.add_timeout(6.0, function()
        if not hud_pinned then hide_info_hud() end
    end)
    ensure_hud_timer()
    render_info_hud()
end

local function pin_info_hud()
    if hud_visible and hud_pinned then
        hide_info_hud()
        return
    end
    if hud_hide_timer then hud_hide_timer:kill(); hud_hide_timer = nil end
    hud_visible = true
    hud_pinned = true
    ensure_hud_timer()
    render_info_hud()
end

mp.add_forced_key_binding("i", "phaenna-toggle-hud", toggle_info_hud)
mp.add_forced_key_binding("I", "phaenna-pin-hud", pin_info_hud)

start_anim()
"##;

fn player_support_dir() -> PathBuf {
    Config::cache_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("phaenna"))
        .join("player")
}

/// Write YouTube-parity keybindings config (`input.conf`) and return its path.
pub fn prepare_youtube_input_conf() -> Option<PathBuf> {
    let dir = player_support_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("phaenna_yt_input.conf");
    std::fs::write(&path, YOUTUBE_INPUT_CONF).ok().map(|_| path)
}

/// Write animated `mpv` Lua ASS loading overlay script and return its path.
pub fn prepare_loading_overlay_script() -> Option<PathBuf> {
    let dir = player_support_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("phaenna_loader.lua");
    std::fs::write(&path, PHAENNA_LOADER_LUA).ok().map(|_| path)
}

/// Write `mpv` live-edge controller Lua script and return its path.
pub fn prepare_live_controller_script() -> Option<PathBuf> {
    let dir = player_support_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("phaenna_live.lua");
    std::fs::write(&path, PHAENNA_LIVE_LUA).ok().map(|_| path)
}

fn sanitize_script_opt(s: &str) -> String {
    s.replace([',', '\n', '\r', '='], " ").trim().to_string()
}

pub fn youtube_watch_url(video_id: &str) -> String {
    let clean_id = video_id.trim_start_matches("RD");
    format!("https://www.youtube.com/watch?v={}", clean_id)
}

fn shared_mpv_kv_flags(opts: &PlayOptions) -> Vec<String> {
    let use_live_pipeline = opts.is_live && opts.live_mode != LiveStreamMode::Off;
    let eff_master = if opts.performance_mode == PerformanceMode::Manual {
        opts.manual_mastering
    } else {
        ManualMasteringConfig::from_preset(opts.performance_mode)
    };
    let (max_buf, back_buf) = if use_live_pipeline {
        opts.live_mode.demuxer_limits()
    } else {
        eff_master.demuxer_cache.limits()
    };
    let hwdec_val = eff_master.hwdec.effective_mpv_value();
    let dev = crate::services::platform::detect_device_profile();
    let caps = crate::services::platform::detect_player_capabilities();
    let mut flags = vec![
        format!("hwdec={hwdec_val}"),
        format!("vo={}", caps.effective_vo()),
        format!("gpu-api={}", caps.effective_gpu_api(dev)),
        "cache=yes".to_string(),
        format!("demuxer-max-bytes={max_buf}"),
        format!("demuxer-max-back-bytes={back_buf}"),
        format!("force-media-title={}", opts.title),
    ];

    if opts.quality_format.starts_with("bestaudio") {
        flags.push("vid=no".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        flags.push(format!(
            "wayland-app-id={}",
            crate::services::pip::PHAENNA_WAYLAND_APP_ID
        ));
        flags.push(format!(
            "x11-name={}",
            crate::services::pip::PHAENNA_WAYLAND_APP_ID
        ));
    }
    if opts.auto_pip {
        flags.push("fullscreen=no".to_string());
        flags.push("ontop=yes".to_string());
        flags.push("border=no".to_string());
        flags.push("keepaspect-window=yes".to_string());
        flags.push("window-scale=0.30".to_string());
        flags.push("geometry=30%x30%-24-24".to_string());
    } else if opts.auto_fullscreen {
        flags.push("fullscreen=yes".to_string());
    }

    if let Some(conf_path) = prepare_youtube_input_conf() {
        flags.push(format!("input-conf={}", conf_path.to_string_lossy()));
    }

    let mut script_opt_parts: Vec<String> = Vec::new();
    if let Some(ytdlp_path) = find_binary("yt-dlp") {
        script_opt_parts.push(format!(
            "ytdl_hook-ytdl_path={}",
            sanitize_script_opt(&ytdlp_path.to_string_lossy())
        ));
    }
    if let Some(lua_path) = prepare_loading_overlay_script() {
        flags.push(format!("script={}", lua_path.to_string_lossy()));
        let safe_title = sanitize_script_opt(&opts.title);
        let mut safe_meta = if use_live_pipeline {
            format!("PHAENNA LIVE | {}", opts.live_mode.short_badge())
        } else {
            format!(
                "PHAENNA {} | {}",
                opts.performance_mode.short_badge(),
                opts.motion_mode.short_badge()
            )
        };
        if opts.network_mode.is_congested() {
            safe_meta.push_str(&format!(" | {}", opts.network_mode.short_badge()));
        }
        script_opt_parts.push(format!("phaenna_title={safe_title}"));
        script_opt_parts.push(format!("phaenna_meta={safe_meta}"));
        let safe_ch = sanitize_script_opt(&opts.loading_channel);
        if !safe_ch.is_empty() {
            script_opt_parts.push(format!("phaenna_channel={safe_ch}"));
        }
        let safe_q = sanitize_script_opt(&opts.loading_quality);
        if !safe_q.is_empty() {
            script_opt_parts.push(format!("phaenna_quality={safe_q}"));
        }
        if let Some(dir) = opts
            .loading_art_dir
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            script_opt_parts.push(format!("phaenna_card_dir={}", sanitize_script_opt(dir)));
            script_opt_parts.push(format!(
                "phaenna_card_count={}",
                crate::services::loading_art::Y2K_ART_COUNT
            ));
            script_opt_parts.push(format!(
                "phaenna_card_idx={}",
                opts.loading_variant_idx % crate::services::loading_art::Y2K_ART_COUNT
            ));
        }
        script_opt_parts.push(format!("phaenna_auto_pip={}", if opts.auto_pip { "1" } else { "0" }));
        script_opt_parts.push(format!("phaenna_perf={}", opts.performance_mode.short_badge()));
        script_opt_parts.push(format!("phaenna_gov={}", eff_master.governor_mode.mpv_opt_value()));
        script_opt_parts.push(format!("phaenna_grain={:.3}", eff_master.organic_grain.luma_strength()));
        script_opt_parts.push(format!("phaenna_shader_opts={}", if caps.has_shader_opts { "1" } else { "0" }));
        script_opt_parts.push(format!(
            "phaenna_motion={}",
            match opts.motion_mode {
                MotionMode::Off => "off",
                MotionMode::Cadence => "cadence",
                MotionMode::Smooth120Hz => "smooth120",
            }
        ));
        script_opt_parts.push(format!("phaenna_device={}", sanitize_script_opt(&dev.inspector_header())));
    }

    if opts.local_file.is_none() {
        flags.push(format!(
            "network-timeout={}",
            opts.network_mode.network_timeout_secs()
        ));
        flags.push(
            "stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=3,http_persistent=1,multiple_requests=1"
                .to_string(),
        );
    }

    if use_live_pipeline {
        flags.push(format!(
            "demuxer-readahead-secs={}",
            opts.live_mode.readahead_secs()
        ));
        flags.push(format!(
            "cache-pause-initial={}",
            if opts.live_mode.cache_pause_initial() { "yes" } else { "no" }
        ));
        let live_pause_wait = if opts.network_mode.is_congested() {
            opts.live_mode.cache_pause_wait_secs().max(3.5)
        } else {
            opts.live_mode.cache_pause_wait_secs()
        };
        flags.push(format!("cache-pause-wait={:.1}", live_pause_wait));
        flags.push(format!(
            "hls-bitrate={}",
            opts.network_mode.live_hls_bitrate()
        ));
        flags.push("video-sync=audio".to_string());
        flags.push("interpolation=no".to_string());

        if let Some(live_lua) = prepare_live_controller_script() {
            flags.push(format!("scripts-append={}", live_lua.to_string_lossy()));
            let (trig, tgt, spd) = opts
                .live_mode
                .catchup_threshold_and_speed()
                .unwrap_or((999.0, 15.0, 1.0));
            let active_catchup = opts.live_auto_catchup
                && opts.live_mode.catchup_threshold_and_speed().is_some();
            script_opt_parts.push(format!(
                "phaenna_live_catchup={}",
                if active_catchup { "1" } else { "0" }
            ));
            script_opt_parts.push(format!("phaenna_live_trigger={trig:.1}"));
            script_opt_parts.push(format!("phaenna_live_target={tgt:.1}"));
            script_opt_parts.push(format!("phaenna_live_speed={spd:.2}"));
        }
    } else if opts.performance_mode == PerformanceMode::Efficient {
        flags.push("video-sync=audio".to_string());
        flags.push("interpolation=no".to_string());
        if opts.local_file.is_none() {
            flags.push(format!(
                "demuxer-readahead-secs={}",
                opts.network_mode.vod_readahead_secs()
            ));
            flags.push("cache-pause-initial=yes".to_string());
            flags.push(format!(
                "cache-pause-wait={:.1}",
                opts.network_mode.vod_cache_pause_wait_secs()
            ));
            if opts.network_mode.use_disk_cache() {
                flags.push("cache-on-disk=yes".to_string());
            }
        }
    } else {
        match opts.motion_mode {
            MotionMode::Off => {
                flags.push("video-sync=audio".to_string());
                flags.push("interpolation=no".to_string());
            }
            MotionMode::Cadence => {
                // Start cool on audio sync; Dynamic Governor turns interpolation ON only for
                // genuine non-integer refresh mismatches (e.g. 24@60Hz) when on AC & cooled.
                flags.push("video-sync=audio".to_string());
                flags.push("interpolation=no".to_string());
                flags.push(
                    if opts.performance_mode == PerformanceMode::Quality {
                        "tscale=mitchell"
                    } else {
                        "tscale=oversample"
                    }
                    .to_string(),
                );
            }
            MotionMode::Smooth120Hz => {
                flags.push("video-sync=display-resample".to_string());
                flags.push("interpolation=yes".to_string());
                flags.push("tscale=catmull_rom".to_string());
                flags.push("interpolation-threshold=0.01".to_string());
                flags.push("override-display-fps=120".to_string());
            }
        }
        if opts.local_file.is_none() {
            flags.push(format!(
                "demuxer-readahead-secs={}",
                opts.network_mode.vod_readahead_secs()
            ));
            flags.push("cache-pause-initial=yes".to_string());
            flags.push(format!(
                "cache-pause-wait={:.1}",
                opts.network_mode.vod_cache_pause_wait_secs()
            ));
            if opts.network_mode.use_disk_cache() {
                flags.push("cache-on-disk=yes".to_string());
            }
        }
    }

    if !script_opt_parts.is_empty() {
        flags.push(format!("script-opts={}", script_opt_parts.join(",")));
    }

    if opts.local_file.is_none() {
        let direct_opt = if use_live_pipeline {
            None
        } else {
            opts.direct_stream.as_ref()
        };
        if let Some(direct) = direct_opt {
            flags.push("ytdl=no".to_string());
            if let Some(ref ua) = direct.user_agent {
                if !ua.trim().is_empty() {
                    flags.push(format!("user-agent={ua}"));
                }
            }
            if let Some(ref aurl) = direct.audio_url {
                flags.push(format!("audio-file={aurl}"));
            }
        } else {
            flags.push(format!("ytdl-format={}", opts.quality_format));
            let vcodec_order = eff_master.codec_priority.format_sort_vcodec();
            let sort_spec = match opts.dub_preference {
                // Keep `lang` ahead of `abr` so YouTube's `language_preference=10` (Original)
                // never loses to slightly higher-bitrate AI-dubbed audio tracks.
                DubPreference::OriginalOnly => {
                    format!("lang,res:2160,fps,hdr,vcodec:{vcodec_order},vbr,abr")
                }
                DubPreference::PreferDubbed => {
                    format!("res:2160,fps,hdr,vcodec:{vcodec_order},vbr,abr")
                }
            };
            flags.push(format!("ytdl-raw-options-append=format-sort={sort_spec}"));
            flags.push("ytdl-raw-options-append=ignore-config=".to_string());
            flags.push("ytdl-raw-options-append=no-playlist=".to_string());
            if let Some(deno_path) = find_binary("deno") {
                flags.push(format!(
                    "ytdl-raw-options-append=js-runtimes=deno:{}",
                    deno_path.to_string_lossy()
                ));
            }
            if opts.force_ipv4 {
                flags.push("ytdl-raw-options-append=force-ipv4=".to_string());
            }
            if !use_live_pipeline
                && opts
                    .network_mode
                    .effective_multistreams(opts.audio_multistreams, opts.performance_mode)
            {
                flags.push("ytdl-raw-options-append=audio-multistreams=".to_string());
            }
            if use_live_pipeline && opts.live_from_start {
                flags.push("ytdl-raw-options-append=live-from-start=".to_string());
            }
            if let Some(ref cpath) = opts.cookies_path {
                if cpath.exists() {
                    flags.push(format!(
                        "ytdl-raw-options-append=cookies={}",
                        cpath.to_string_lossy()
                    ));
                    flags.push("ytdl-raw-options-append=mark-watched=".to_string());
                }
            }
        }
    }

    let alang = match opts.dub_preference {
        DubPreference::OriginalOnly => "und,orig".to_string(),
        DubPreference::PreferDubbed => {
            if opts.dub_lang.trim().is_empty() {
                "en".to_string()
            } else {
                opts.dub_lang.trim().to_string()
            }
        }
    };
    flags.push(format!("alang={alang}"));

    if let Some(ref af) = opts.audio_filter_chain {
        if !af.trim().is_empty() {
            flags.push(format!("af={af}"));
        }
    } else if opts.audio_loudnorm {
        flags.push(format!("af={LOUDNORM_FILTER}"));
    }

    if !use_live_pipeline {
        if let Some(start) = opts.start_pos {
            if start > 3.0 {
                flags.push(format!("start={:.1}", start));
            }
        }
    }

    if opts.subtitles_enabled {
        let lang = opts.sub_lang.as_deref().unwrap_or("en,eng");
        flags.push(format!("slang={}", lang));
        flags.push("sub-auto=fuzzy".to_string());
        let effective_sub_url = opts.sub_url.as_deref().or_else(|| {
            opts.direct_stream
                .as_ref()
                .and_then(|d| d.sub_url.as_deref())
        });
        if let Some(sub_url) = effective_sub_url {
            flags.push(format!("sub-file={}", sub_url));
        }
        for flag in opts.sub_style.to_mpv_flags() {
            flags.push(flag.trim_start_matches("--").to_string());
        }
    }

    // Bypass heavy 4K GLSL shader chains in LowLatency / BalancedLive to avoid frame drops on live streams.
    let allow_shaders =
        !use_live_pipeline || opts.live_mode == LiveStreamMode::SmoothDvr;
    if allow_shaders {
        if let Some(shaders) = prepare_video_shaders_with_mastering(
            opts.shader_mode,
            opts.performance_mode,
            Some(eff_master),
        ) {
            flags.push(format!("glsl-shaders={}", shaders));
        }
        for flag in mpv_mastering_flags_with_manual(
            opts.shader_mode,
            opts.mastering_profile,
            opts.performance_mode,
            Some(eff_master),
        ) {
            flags.push(flag.trim_start_matches("--").to_string());
        }
    }

    let g = opts.picture.clamped();
    if g.brightness != 0 {
        flags.push(format!("brightness={}", g.brightness));
    }
    if g.contrast != 0 {
        flags.push(format!("contrast={}", g.contrast));
    }
    if g.saturation != 0 {
        flags.push(format!("saturation={}", g.saturation));
    }
    if g.gamma != 0 {
        flags.push(format!("gamma={}", g.gamma));
    }
    if g.hue != 0 {
        flags.push(format!("hue={}", g.hue));
    }
    if g.sharpness != 0 {
        flags.push(format!("sharpen={:.2}", g.sharpness as f64 / 50.0));
    }

    flags
}

fn resolved_mpv_target(opts: &PlayOptions) -> String {
    let use_live_pipeline = opts.is_live && opts.live_mode != LiveStreamMode::Off;
    opts.local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| {
            if !use_live_pipeline {
                opts.direct_stream.as_ref().map(|d| d.primary_url.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id))
}

/// Resolve `Player::AutoBest` to the highest-parity installed `libmpv` binary on the host.
pub fn resolve_auto_player() -> Player {
    if find_binary("mpv").is_some() {
        return Player::Mpv;
    }
    #[cfg(windows)]
    if find_binary("mpvnet").is_some() || find_binary("mpv.net").is_some() {
        return Player::MpvNet;
    }
    #[cfg(target_os = "linux")]
    if find_binary("celluloid").is_some() {
        return Player::Celluloid;
    }
    Player::Mpv
}

/// Build arguments for standalone `mpv` (pure function for unit testing).
pub fn build_mpv_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec!["--force-window=immediate".to_string()];
    if let Some(ref sock) = opts.ipc_socket {
        args.push(format!("--input-ipc-server={}", sock));
    }
    for kv in shared_mpv_kv_flags(opts) {
        args.push(format!("--{kv}"));
    }
    args.push(resolved_mpv_target(opts));
    args
}

/// Build arguments for Windows `mpv.net` (`libmpv` native shell with full CLI/IPC/GLSL/Lua parity).
pub fn build_mpvnet_args(opts: &PlayOptions) -> Vec<String> {
    build_mpv_args(opts)
}

/// Build arguments for Linux `Celluloid` (`GTK4/Wayland libmpv` translating flags via `--mpv-*`).
pub fn build_celluloid_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec![
        "--new-window".to_string(),
        "--mpv-force-window=immediate".to_string(),
    ];
    if let Some(ref sock) = opts.ipc_socket {
        args.push(format!("--mpv-input-ipc-server={}", sock));
    }
    for kv in shared_mpv_kv_flags(opts) {
        args.push(format!("--mpv-{kv}"));
    }
    args.push(resolved_mpv_target(opts));
    args
}

/// Launch playback asynchronously in the background without blocking the TUI event loop.
pub fn spawn_player(opts: &PlayOptions) -> Result<Option<Child>> {
    let effective_player = opts.player.resolve_supported_for_host();
    let resolved_player = match effective_player {
        Player::AutoBest => resolve_auto_player(),
        other => other,
    };

    match resolved_player {
        Player::AutoBest => unreachable!(),
        Player::Mpv => {
            let bin = find_binary("mpv").ok_or_else(|| {
                AppError::Player(
                    "mpv not found in PATH. Install mpv (e.g. brew/winget/apt) to launch streams."
                        .to_string(),
                )
            })?;
            let args = build_mpv_args(opts);
            let child = Command::new(bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| AppError::Player(format!("Failed to launch mpv: {}", e)))?;
            Ok(Some(child))
        }
        Player::MpvNet => {
            let bin = find_binary("mpvnet")
                .or_else(|| find_binary("mpv.net"))
                .ok_or_else(|| {
                    AppError::Player(
                        "mpv.net not found (install via `winget install mpv.net` or switch to mpv)."
                            .to_string(),
                    )
                })?;
            let args = build_mpvnet_args(opts);
            let child = Command::new(bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| AppError::Player(format!("Failed to launch mpv.net: {}", e)))?;
            Ok(Some(child))
        }
        Player::Celluloid => {
            let bin = find_binary("celluloid").ok_or_else(|| {
                AppError::Player(
                    "Celluloid not found in PATH (install via apt/dnf/flatpak or switch to mpv)."
                        .to_string(),
                )
            })?;
            let args = build_celluloid_args(opts);
            let child = Command::new(bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| AppError::Player(format!("Failed to launch Celluloid: {}", e)))?;
            Ok(Some(child))
        }
    }
}

/// Inspect `mpv`/`ytdl_hook` `stderr` and process exit code to surface actionable toast errors on crash.
pub fn classify_player_crash_stderr(stderr_text: &str, exit_code: Option<i32>) -> Option<String> {
    if stderr_text.contains("addOnDisposeCallback")
        || stderr_text.contains("Signature solving failed")
        || stderr_text.contains("n challenge solving failed")
        || stderr_text.contains("The page needs to be reloaded")
        || stderr_text.contains("Requested format is not available")
        || stderr_text.contains("Only images are available")
    {
        return Some(
            "Playback failed: yt-dlp YouTube extractor outdated — update `yt-dlp` and ensure `deno` is installed"
                .to_string(),
        );
    }
    if stderr_text.contains("HTTP Error 413") {
        return Some(
            "Playback failed: YouTube rejected oversized cookies (HTTP 413) — refresh session"
                .to_string(),
        );
    }
    if stderr_text.contains("Sign in to confirm")
        || stderr_text.contains("confirm you're not a bot")
        || stderr_text.contains("confirm you\u{2019}re not a bot")
    {
        return Some(
            "Playback failed: YouTube bot check — sign in via Google Account Hub [s]"
                .to_string(),
        );
    }
    if let Some(code) = exit_code {
        if code != 0 {
            let err_line = stderr_text
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| {
                    !l.is_empty()
                        && (l.contains("ERROR:")
                            || l.contains("[ytdl_hook]")
                            || l.contains("Failed")
                            || l.contains("Error"))
                })
                .or_else(|| stderr_text.lines().rev().map(str::trim).find(|l| !l.is_empty()));
            if let Some(line) = err_line {
                let mut clipped: String = line.chars().take(110).collect();
                if line.chars().count() > 110 {
                    clipped.push_str("...");
                }
                return Some(format!("Playback exited (code {code}): {clipped}"));
            }
            return Some(format!("Playback exited unexpectedly (code {code})"));
        }
    }
    None
}

/// Generate platform-appropriate mpv IPC socket/pipe path for a playback session.
pub fn ipc_socket_path(video_id: &str, session_id: u64) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\phaenna-{}-{}", video_id, session_id)
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir()
            .join(format!("phaenna-{}-{}.sock", video_id, session_id))
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_mpv_args_and_watch_url() {
        assert_eq!(
            youtube_watch_url("RDdQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );

        let opts = PlayOptions {
            video_id: "dQw4w9WgXcQ".to_string(),
            title: "Test Video".to_string(),
            local_file: None,
            quality_format: "bestvideo[height<=1080]+bestaudio/best".to_string(),
            player: Player::Mpv,
            cookies_path: None,
            start_pos: Some(42.5),
            ipc_socket: Some("/tmp/yt-test.sock".to_string()),
            subtitles_enabled: true,
            sub_lang: Some("en".to_string()),
            sub_url: None,
            sub_style: SubtitleStyleConfig::default(),
            shader_mode: VideoShaderMode::Balanced,
            mastering_profile: VideoMasteringProfile::StandardHd,
            performance_mode: PerformanceMode::Quality,
            manual_mastering: ManualMasteringConfig::default(),
            motion_mode: MotionMode::Smooth120Hz,
            dub_preference: DubPreference::OriginalOnly,
            dub_lang: "en".to_string(),
            audio_multistreams: true,
            audio_loudnorm: true,
            audio_filter_chain: None,
            is_live: false,
            live_mode: LiveStreamMode::BalancedLive,
            live_from_start: false,
            live_auto_catchup: true,
            network_mode: NetworkMode::Normal,
            force_ipv4: false,
            auto_fullscreen: false,
            auto_pip: false,
            loading_art_dir: None,
            loading_variant_idx: 0,
            loading_channel: String::new(),
            loading_quality: String::new(),
            picture: PictureGrade::default(),
            direct_stream: None,
        };

        let args = build_mpv_args(&opts);
        assert!(args.iter().any(|a| a == "--input-ipc-server=/tmp/yt-test.sock"));
        assert!(args.iter().any(|a| a == "--start=42.5"));
        assert!(args.iter().any(|a| a.starts_with("--glsl-shaders=")));
        assert!(args.iter().any(|a| a == "--demuxer-max-bytes=512MiB"));
        assert!(args.iter().any(|a| a.contains("format-sort=")));
        assert!(args.iter().any(|a| a == "--ytdl-raw-options-append=audio-multistreams="));
        assert!(args.iter().any(|a| a == "--alang=und,orig"));
        assert!(args.iter().any(|a| a.starts_with("--af=lavfi=")));
        assert!(args.iter().any(|a| a == "--video-sync=display-resample"));
        assert!(args.iter().any(|a| a == "--override-display-fps=120"));
        assert!(args.iter().any(|a| a.starts_with("--input-conf=")));
        assert!(args.iter().any(|a| a.starts_with("--script=")));
        assert!(args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));

        let mut eco_opts = opts.clone();
        eco_opts.performance_mode = PerformanceMode::Efficient;
        eco_opts.motion_mode = MotionMode::Cadence;
        let eco_args = build_mpv_args(&eco_opts);
        assert!(eco_args.iter().any(|a| a == "--demuxer-max-bytes=48MiB"));
        assert!(!eco_args.iter().any(|a| a.contains("audio-multistreams")));
        assert!(eco_args.iter().any(|a| a == "--scale=spline36"));
        assert!(eco_args.iter().any(|a| a == "--dscale=hermite"));
        assert!(eco_args.iter().any(|a| a == "--interpolation=no"));
        assert!(eco_args.iter().any(|a| a == "--video-sync=audio"));

        let mut bal_opts = opts.clone();
        bal_opts.performance_mode = PerformanceMode::Balanced;
        let bal_args = build_mpv_args(&bal_opts);
        assert!(bal_args.iter().any(|a| a == "--hdr-compute-peak=no"));
        assert!(bal_args.iter().any(|a| a == "--cscale=bilinear"));
        assert!(bal_args.iter().any(|a| a == "--dscale=hermite"));
        assert!(bal_args.iter().any(|a| a.contains("phaenna_gov=auto")));
        assert!(bal_args.iter().any(|a| a.contains("phaenna_grain=0.008")));

        let mut av1_opts = opts.clone();
        av1_opts.performance_mode = PerformanceMode::Manual;
        av1_opts.manual_mastering.codec_priority = crate::config::CodecPriority::Av1First;
        assert!(build_mpv_args(&av1_opts)
            .iter()
            .any(|a| a.contains("format-sort=lang,res:2160,fps,hdr,vcodec:av01")));

        let mut man_opts = opts.clone();
        man_opts.performance_mode = PerformanceMode::Manual;
        man_opts.manual_mastering.super_res = crate::config::SuperResMode::Auto1080p;
        man_opts.manual_mastering.tone_map = crate::config::ToneMapMode::Bt2446a;
        man_opts.manual_mastering.hdr_compute_peak = true;
        let man_args = build_mpv_args(&man_opts);
        assert!(man_args.iter().any(|a| a == "--tone-mapping=bt.2446a"));
        assert!(man_args.iter().any(|a| a == "--hdr-compute-peak=yes"));
    }

    #[test]
    fn test_build_mpvnet_and_celluloid_full_shader_ipc_parity() {
        let mut opts = PlayOptions {
            video_id: "dQw4w9WgXcQ".to_string(),
            title: "Reference 4K Test".to_string(),
            local_file: None,
            quality_format: "bestvideo[height<=?4320][vcodec^=av01]+bestaudio/best".to_string(),
            player: Player::Celluloid,
            cookies_path: None,
            start_pos: Some(18.0),
            ipc_socket: Some("/tmp/phaenna-celluloid.sock".to_string()),
            subtitles_enabled: true,
            sub_lang: Some("en".to_string()),
            sub_url: None,
            sub_style: SubtitleStyleConfig::default(),
            shader_mode: VideoShaderMode::Reference4K,
            mastering_profile: VideoMasteringProfile::StandardHd,
            performance_mode: PerformanceMode::Quality,
            manual_mastering: ManualMasteringConfig::default(),
            motion_mode: MotionMode::Cadence,
            dub_preference: DubPreference::PreferDubbed,
            dub_lang: "ar".to_string(),
            audio_multistreams: true,
            audio_loudnorm: false,
            audio_filter_chain: None,
            is_live: false,
            live_mode: LiveStreamMode::BalancedLive,
            live_from_start: false,
            live_auto_catchup: true,
            network_mode: NetworkMode::Normal,
            force_ipv4: false,
            auto_fullscreen: true,
            auto_pip: false,
            loading_art_dir: None,
            loading_variant_idx: 0,
            loading_channel: String::new(),
            loading_quality: String::new(),
            picture: PictureGrade::default(),
            direct_stream: None,
        };

        let c_args = build_celluloid_args(&opts);
        assert!(c_args.iter().any(|a| a == "--new-window"));
        assert!(c_args
            .iter()
            .any(|a| a == "--mpv-input-ipc-server=/tmp/phaenna-celluloid.sock"));
        assert!(c_args.iter().any(|a| a.starts_with("--mpv-glsl-shaders=")));
        assert!(c_args.iter().any(|a| a == "--mpv-deband=yes"));
        assert!(c_args.iter().any(|a| a == "--mpv-tone-mapping=spline"));
        assert!(c_args.iter().any(|a| a == "--mpv-fullscreen=yes"));
        assert!(c_args.iter().any(|a| a == "--mpv-start=18.0"));
        assert!(c_args.iter().any(|a| a == "--mpv-alang=ar"));
        assert!(c_args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));

        opts.player = Player::MpvNet;
        let m_args = build_mpvnet_args(&opts);
        assert!(m_args
            .iter()
            .any(|a| a == "--input-ipc-server=/tmp/phaenna-celluloid.sock"));
        assert!(m_args.iter().any(|a| a.starts_with("--glsl-shaders=")));
        assert!(m_args.iter().any(|a| a == "--fullscreen=yes"));
        assert!(m_args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));
    }

    #[test]
    fn test_live_stream_mode_mpv_flags() {
        let opts = PlayOptions {
            video_id: "liveStream1".to_string(),
            title: "24/7 Live Stream".to_string(),
            local_file: None,
            quality_format: "best[height<=?1080][vcodec!^=av01][protocol^=m3u8]/best".to_string(),
            player: Player::Mpv,
            cookies_path: None,
            start_pos: Some(120.0),
            ipc_socket: None,
            subtitles_enabled: false,
            sub_lang: None,
            sub_url: None,
            sub_style: SubtitleStyleConfig::default(),
            shader_mode: VideoShaderMode::Reference4K,
            mastering_profile: VideoMasteringProfile::StandardHd,
            performance_mode: PerformanceMode::Quality,
            manual_mastering: ManualMasteringConfig::default(),
            motion_mode: MotionMode::Smooth120Hz,
            dub_preference: DubPreference::OriginalOnly,
            dub_lang: "en".to_string(),
            audio_multistreams: true,
            audio_loudnorm: false,
            audio_filter_chain: None,
            is_live: true,
            live_mode: LiveStreamMode::LowLatency,
            live_from_start: false,
            live_auto_catchup: true,
            network_mode: NetworkMode::Normal,
            force_ipv4: false,
            auto_fullscreen: false,
            auto_pip: false,
            loading_art_dir: None,
            loading_variant_idx: 0,
            loading_channel: String::new(),
            loading_quality: String::new(),
            picture: PictureGrade::default(),
            direct_stream: None,
        };

        let args = build_mpv_args(&opts);
        assert!(args.iter().any(|a| a == "--video-sync=audio"));
        assert!(args.iter().any(|a| a == "--interpolation=no"));
        assert!(args.iter().any(|a| a == "--cache-pause-initial=no"));
        assert!(args.iter().any(|a| a == "--cache-pause-wait=1.2"));
        assert!(args.iter().any(|a| a == "--demuxer-readahead-secs=3"));
        assert!(args.iter().any(|a| a.contains("reconnect=1")));
        assert!(args.iter().any(|a| a.starts_with("--scripts-append=")));
        assert!(!args.iter().any(|a| a.starts_with("--start=")));
        assert!(!args.iter().any(|a| a.starts_with("--glsl-shaders=")));
        assert!(!args.iter().any(|a| a.contains("audio-multistreams")));
    }

    #[test]
    fn test_congested_wifi_and_direct_stream_mpv_flags() {
        let mut opts = PlayOptions {
            video_id: "dQw4w9WgXcQ".to_string(),
            title: "Congested WiFi Test".to_string(),
            local_file: None,
            quality_format: "bestvideo[height<=?720]+bestaudio/best".to_string(),
            player: Player::Mpv,
            cookies_path: None,
            start_pos: None,
            ipc_socket: None,
            subtitles_enabled: false,
            sub_lang: None,
            sub_url: None,
            sub_style: SubtitleStyleConfig::default(),
            shader_mode: VideoShaderMode::Balanced,
            mastering_profile: VideoMasteringProfile::StandardHd,
            performance_mode: PerformanceMode::Balanced,
            manual_mastering: ManualMasteringConfig::default(),
            motion_mode: MotionMode::Cadence,
            dub_preference: DubPreference::OriginalOnly,
            dub_lang: "en".to_string(),
            audio_multistreams: true,
            audio_loudnorm: false,
            audio_filter_chain: None,
            is_live: false,
            live_mode: LiveStreamMode::BalancedLive,
            live_from_start: false,
            live_auto_catchup: true,
            network_mode: NetworkMode::CongestedWifi,
            force_ipv4: true,
            picture: PictureGrade::default(),
            direct_stream: None,
            auto_fullscreen: false,
            auto_pip: false,
            loading_art_dir: None,
            loading_variant_idx: 0,
            loading_channel: String::new(),
            loading_quality: String::new(),
        };

        let wifi_args = build_mpv_args(&opts);
        assert!(wifi_args.iter().any(|a| a.contains("reconnect=1")));
        assert!(wifi_args.iter().any(|a| a == "--network-timeout=12"));
        assert!(wifi_args.iter().any(|a| a == "--cache-pause-wait=4.5"));
        assert!(wifi_args.iter().any(|a| a == "--demuxer-readahead-secs=120"));
        assert!(wifi_args.iter().any(|a| a == "--cache-on-disk=yes"));
        assert!(wifi_args.iter().any(|a| a == "--ytdl-raw-options-append=force-ipv4="));
        assert!(!wifi_args.iter().any(|a| a.contains("audio-multistreams")));

        opts.direct_stream = Some(SelectedStreamPair {
            primary_url: "https://cdn.example.com/video_720p.webm".to_string(),
            audio_url: Some("https://cdn.example.com/audio_opus.webm".to_string()),
            height: 720,
            fps: 30.0,
            tbr_kbps: 1050.0,
            vcodec: "vp9".to_string(),
            user_agent: Some("PhaennaTest/1.0".to_string()),
            sub_url: None,
        });
        let direct_args = build_mpv_args(&opts);
        assert!(direct_args.iter().any(|a| a == "--ytdl=no"));
        assert!(direct_args
            .iter()
            .any(|a| a == "--audio-file=https://cdn.example.com/audio_opus.webm"));
        assert!(direct_args.iter().any(|a| a == "--user-agent=PhaennaTest/1.0"));
        assert_eq!(
            direct_args.last().unwrap(),
            "https://cdn.example.com/video_720p.webm"
        );

        let mut auto_fs_opts = opts.clone();
        auto_fs_opts.auto_fullscreen = true;
        auto_fs_opts.loading_art_dir = Some("/tmp/phaenna-portraits".to_string());
        auto_fs_opts.loading_variant_idx = 4;
        auto_fs_opts.loading_channel = "ByteByteGo".to_string();
        auto_fs_opts.loading_quality = "1080P".to_string();
        auto_fs_opts.start_pos = Some(65.0);
        let fs_args = build_mpv_args(&auto_fs_opts);
        assert!(fs_args.iter().any(|a| a == "--fullscreen=yes"));
        assert!(!fs_args.iter().any(|a| a == "--image-display-duration=2"));
        assert!(fs_args.iter().any(|a| a.contains("phaenna_card_dir=/tmp/phaenna-portraits")));
        assert!(fs_args.iter().any(|a| a.contains("phaenna_card_idx=4")));
        assert!(fs_args.iter().any(|a| a == "--start=65.0"));

        let mut pip_opts = opts;
        pip_opts.auto_pip = true;
        let pip_args = build_mpv_args(&pip_opts);
        assert!(pip_args.iter().any(|a| a == "--ontop=yes"));
        assert!(pip_args.iter().any(|a| a == "--border=no"));
        assert!(pip_args.iter().any(|a| a == "--window-scale=0.30"));
    }

    #[test]
    fn test_build_mpv_audio_filter_chain_composition() {
        assert!(build_mpv_audio_filter_chain(
            [0; 10],
            0,
            false,
            AudioChannelMode::Stereo,
            false,
            false
        )
        .is_none());

        let chain = build_mpv_audio_filter_chain(
            crate::config::AudioEqPreset::VoiceClarity.band_gains_db(),
            -2,
            false,
            AudioChannelMode::Crossfeed,
            true,
            true,
        )
        .unwrap();
        assert!(chain.starts_with("lavfi=["));
        assert!(chain.contains("volume=-2dB"));
        assert!(chain.contains("highpass=f=80"));
        assert!(chain.contains("equalizer=f=2000:t=q:w=1.4:g=5"));
        assert!(chain.contains("dynaudnorm=f=150:g=15:p=0.9"));
        assert!(chain.contains("crossfeed=strength=0.45:range=0.5"));

        // Bypass skips preamp + EQ bands while keeping spatial + loudnorm
        let bypassed = build_mpv_audio_filter_chain(
            crate::config::AudioEqPreset::BassBoost.band_gains_db(),
            -3,
            true,
            AudioChannelMode::WideStage,
            false,
            false,
        )
        .unwrap();
        assert!(!bypassed.contains("equalizer="));
        assert!(!bypassed.contains("volume="));
        assert!(bypassed.contains("extrastereo=m=1.45"));
    }

    #[test]
    fn test_classify_player_crash_stderr_and_ignore_config_flag() {
        assert!(classify_player_crash_stderr("", Some(0)).is_none());
        let outdated = classify_player_crash_stderr(
            "[ytdl_hook] ERROR: TypeError: a.addOnDisposeCallback is not a function",
            Some(2),
        )
        .unwrap();
        assert!(outdated.contains("yt-dlp YouTube extractor outdated"));

        let err413 = classify_player_crash_stderr(
            "[ytdl_hook] WARNING: HTTP Error 413: Request Entity Too Large",
            Some(2),
        )
        .unwrap();
        assert!(err413.contains("HTTP 413"));

        let generic = classify_player_crash_stderr(
            "[ffmpeg] TLS fatal error\nERROR: failed to open stream",
            Some(1),
        )
        .unwrap();
        assert!(generic.contains("code 1"));
        assert!(generic.contains("ERROR: failed to open stream"));
    }
}
