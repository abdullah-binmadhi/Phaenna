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
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Default)]
pub struct DependencyStatus {
    pub has_ytdlp: bool,
    pub has_mpv: bool,
    pub has_iina: bool,
    pub has_vlc: bool,
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
        if name == "iina" || name == "iina-cli" {
            extras.push(PathBuf::from("/Applications/IINA.app/Contents/MacOS/iina-cli"));
            extras.push(PathBuf::from("/Applications/IINA.app/Contents/MacOS/IINA"));
        } else if name == "vlc" {
            extras.push(PathBuf::from("/Applications/VLC.app/Contents/MacOS/VLC"));
        } else if name == "mpv" {
            extras.push(PathBuf::from("/Applications/mpv.app/Contents/MacOS/mpv"));
        }
    }

    extras.into_iter().find(|p| is_executable(p))
}

pub fn detect_dependencies() -> DependencyStatus {
    let has_ytdlp = find_binary("yt-dlp").is_some();
    let has_mpv = find_binary("mpv").is_some();
    let has_iina = find_binary("iina-cli").is_some()
        || find_binary("iina").is_some()
        || Path::new("/Applications/IINA.app").exists();
    let has_vlc = find_binary("vlc").is_some()
        || Path::new("/Applications/VLC.app").exists();
    let has_ffmpeg = find_binary("ffmpeg").is_some();

    DependencyStatus {
        has_ytdlp,
        has_mpv,
        has_iina,
        has_vlc,
        has_ffmpeg,
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
    pub loading_image: Option<String>,
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
f cycle fullscreen
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
local meta = mp.get_opt("phaenna_meta") or "PHAENNA STUDIO"
local has_card = (mp.get_opt("phaenna_has_card") or "0") == "1"
local pip_active = (mp.get_opt("phaenna_auto_pip") or "0") == "1"

local function esc_ass(s)
    if not s then return "" end
    return tostring(s):gsub("[{}]", ""):gsub("[\r\n]+", " ")
end

title = esc_ass(title)
meta = esc_ass(meta)
if #title > 62 then title = string.sub(title, 1, 59) .. "..." end

-- =============================================================================
-- 1. Y2K Anime Loading Screen & Mid-Stream Buffering Overlay
-- =============================================================================
local phase = 0.0
local cache_pct = nil
local cache_speed = 0
local cache_dur = 0
local is_loading = true
local timer = nil
local boot_time = mp.get_time()
local advanced_slide = false

local function fmt_speed(bps)
    if not bps or bps <= 0 then return "" end
    if bps >= 1048576 then
        return string.format("%.1f MB/s", bps / 1048576.0)
    elseif bps >= 1024 then
        return string.format("%d KB/s", math.floor(bps / 1024.0))
    end
    return string.format("%d B/s", math.floor(bps))
end

local function is_y2k_slide_active()
    if not has_card then return false end
    local cnt = mp.get_property_number("playlist-count", 1) or 1
    local ppos = mp.get_property_number("playlist-pos", 0) or 0
    return cnt > 1 and ppos == 0
end

local function render_loader()
    if is_y2k_slide_active() then
        local elapsed = mp.get_time() - boot_time
        if elapsed >= 1.45 and not advanced_slide then
            advanced_slide = true
            mp.commandv("playlist-next")
        end
        -- Animate neon fill directly inside the generated 1280x720 Y2K card track (594..1174, 392..418)
        local pct = math.min(0.95, elapsed / 1.45)
        if cache_pct and cache_pct > 0 then
            pct = math.max(pct, math.min(1.0, cache_pct / 100.0))
        end
        local fill_w = math.max(24, math.floor(568 * pct))
        local spd = fmt_speed(cache_speed)
        local tag = (spd ~= "") and string.format("%d%%  %s", math.floor(pct * 100), spd) or string.format("%d%%", math.floor(pct * 100))
        ov.data = string.format(
            "{\\an7\\pos(600,398)\\bord0\\shad0\\1c&H99D334&\\p1}m 0 0 l %d 0 l %d 14 l 0 14{\\p0}\n" ..
            "{\\an9\\pos(1172,432)\\bord0\\shad0\\fs18\\b1\\1c&HF8BD38&}%s",
            fill_w, fill_w, tag
        )
        ov:update()
        return
    end

    if not is_loading then
        ov:remove()
        return
    end

    phase = (phase + 0.05) % 1.0
    local bar_x0 = 300
    local bar_w = 680
    local bar_y0 = 404
    local bar_y1 = 418

    local spd_txt = fmt_speed(cache_speed)
    local spd_sfx = (spd_txt ~= "") and ("  ·  " .. spd_txt) or ""
    local dur_txt = (cache_dur and cache_dur > 0.1) and string.format("  ·  %.1fs cached", cache_dur) or ""
    local status = "Connecting & Buffering HD Stream" .. spd_sfx .. "..."
    local fill_x0 = bar_x0
    local fill_w = 180
    if cache_pct and cache_pct > 0 and cache_pct < 100 then
        status = string.format("Buffering Stream (%d%%)%s%s...", math.floor(cache_pct), spd_sfx, dur_txt)
        fill_w = math.max(24, math.floor(bar_w * (cache_pct / 100.0)))
    else
        local sweep = math.floor(phase * (bar_w - 160))
        fill_x0 = bar_x0 + sweep
        fill_w = 160
    end
    local fill_x1 = math.min(bar_x0 + bar_w, fill_x0 + fill_w)

    local ass = string.format(
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H14100D&\\1a&H18&\\p1}m 230 262 l 1050 262 l 1050 458 l 230 458{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord2\\1a&HFF&\\3c&HFA8BA7&\\p1}m 230 262 l 1050 262 l 1050 458 l 230 458{\\p0}\n" ..
        "{\\an5\\pos(640,302)\\bord0\\shad0\\fs19\\b1\\1c&H99D334&}✦  %s  ✦\n" ..
        "{\\an5\\pos(640,342)\\bord0\\shad0\\fs25\\b1\\1c&HFAF4F0&}%s\n" ..
        "{\\an5\\pos(640,376)\\bord0\\shad0\\fs17\\1c&HF8BD38&}%s\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H2E2824&\\p1}m %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H99D334&\\p1}m %d %d l %d %d l %d %d l %d %d{\\p0}",
        meta, title, status,
        bar_x0, bar_y0, bar_x0 + bar_w, bar_y0, bar_x0 + bar_w, bar_y1, bar_x0, bar_y1,
        fill_x0, bar_y0, fill_x1, bar_y0, fill_x1, bar_y1, fill_x0, bar_y1
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
    if is_y2k_slide_active() then return end
    is_loading = false
    if timer then
        timer:kill()
        timer = nil
    end
    ov:remove()
end

mp.register_event("playback-restart", function()
    if is_y2k_slide_active() then return end
    local pfc = mp.get_property_bool("paused-for-cache", false)
    if not pfc then stop_anim() end
end)

mp.observe_property("paused-for-cache", "bool", function(_, val)
    if is_y2k_slide_active() then return end
    if val then
        is_loading = true
        start_anim()
    else
        local t = mp.get_property_number("time-pos", 0)
        if t and t > 0.02 then stop_anim() end
    end
end)

mp.observe_property("time-pos", "number", function(_, val)
    if is_y2k_slide_active() then return end
    if val and val > 0.04 and not mp.get_property_bool("paused-for-cache", false) then
        stop_anim()
    end
end)

mp.observe_property("cache-buffering-state", "number", function(_, val) cache_pct = val end)
mp.observe_property("cache-speed", "number", function(_, val) cache_speed = val or 0 end)
mp.observe_property("demuxer-cache-duration", "number", function(_, val) cache_dur = val or 0 end)

-- =============================================================================
-- 2. Picture-in-Picture (PiP) Floating Window Controller ([p] / [P])
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

local function toggle_pip()
    pip_active = not pip_active
    if pip_active then
        apply_pip_preset()
    else
        mp.set_property_bool("ontop", false)
        mp.set_property_bool("border", true)
        mp.set_property_number("window-scale", 1.0)
        mp.set_property("geometry", "50%:50%")
        mp.osd_message("◈ PiP Floating: OFF (Standard Window)", 2.0)
    end
end

local function cycle_pip_corner()
    if not pip_active then
        pip_active = true
    else
        pip_idx = (pip_idx % #pip_presets) + 1
    end
    apply_pip_preset()
end

mp.add_forced_key_binding("p", "phaenna-toggle-pip", toggle_pip)
mp.add_forced_key_binding("P", "phaenna-cycle-pip-corner", cycle_pip_corner)

-- =============================================================================
-- 3. Multi-Color Curated Bento Video Info HUD ([i] 6s / [I] Pinned 1Hz)
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
    local vo = esc_ass(mp.get_property("current-vo", "gpu-next"))
    local avs = mp.get_property_number("avsync", 0) or 0
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
    local hw_lbl = (hwdec ~= "" and hwdec ~= "no") and ("GPU · " .. hwdec) or "CPU Software"
    local hw_col = (hwdec ~= "" and hwdec ~= "no") and "&H99D334&" or "&H24BFFB&"
    local drop_col = (drops > 0) and "&H5E3FF4&" or "&H99D334&"
    local vol_str = muted and "MUTED" or string.format("%d%%", math.floor(vol))
    local vol_col = muted and "&H5E3FF4&" or "&H99D334&"
    local vbit_s = (vbit > 0) and string.format("%.2f Mbps", vbit / 1000000.0) or "ABR"
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

    -- Glass Bento Card at (x: 42..1238, y: 26..376)
    local ass = string.format(
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H100C09&\\1a&H18&\\p1}m 42 26 l 1238 26 l 1238 378 l 42 378{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord2\\1a&HFF&\\3c&HF8BD38&\\p1}m 42 26 l 1238 26 l 1238 378 l 42 378{\\p0}\n" ..
        "{\\an7\\pos(66,42)\\bord0\\shad0\\fs19\\b1\\1c&HF8BD38&}◈ PHAENNA INFO HUD   " ..
        "{\\1c&H24BFFB&}[%s]   {\\1c&H99D334&}[%s]   {\\1c&HFA8BA7&}[%s]   {\\1c&HC8B8A4&}%s\n" ..
        "{\\an7\\pos(66,70)\\bord0\\shad0\\fs24\\b1\\1c&HFAF4F0&}%s\n" ..
        "{\\an7\\pos(66,102)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}%s / %s (%d%%)  ·  Speed %.2fx  ·  Chapter: {\\1c&HFAF4F0&}%s\n" ..
        "{\\an7\\pos(66,128)\\bord0\\shad0\\1c&H2B2420&\\p1}m 0 0 l 1140 0 l 1140 6 l 0 6{\\p0}\n" ..
        "{\\an7\\pos(66,128)\\bord0\\shad0\\1c&HF8BD38&\\p1}m 0 0 l %d 0 l %d 6 l 0 6{\\p0}\n" ..
        "{\\an7\\pos(66,150)\\bord0\\shad0\\fs16\\b1\\1c&HF8BD38&}▶ VIDEO & QUALITY\n" ..
        "{\\an7\\pos(66,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Stream  : {\\1c&HFAF4F0&\\b1}%dx%d  %s\n" ..
        "{\\an7\\pos(66,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Codec   : {\\1c&H24BFFB&}%s  ·  %s\n" ..
        "{\\an7\\pos(66,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Rate    : {\\1c&H99D334&}%.2f fps{\\1c&HC8B8A4&} (Disp %.0fHz)\n" ..
        "{\\an7\\pos(66,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Frames  : {\\1c%s}%d dropped {\\1c&HC8B8A4&}· %s\n" ..
        "{\\an7\\pos(460,150)\\bord0\\shad0\\fs16\\b1\\1c&HFA8BA7&}◈ GPU & MASTERING\n" ..
        "{\\an7\\pos(460,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Decode  : {\\1c%s}%s\n" ..
        "{\\an7\\pos(460,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}VideoOut: {\\1c&HF8BD38&}%s\n" ..
        "{\\an7\\pos(460,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}DSP/EQ  : {\\1c&HFA8BA7&}%s\n" ..
        "{\\an7\\pos(460,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}A/V Sync: {\\1c&H99D334&}%+.1f ms\n" ..
        "{\\an7\\pos(860,150)\\bord0\\shad0\\fs16\\b1\\1c&H24BFFB&}♪ AUDIO & PLAYBACK\n" ..
        "{\\an7\\pos(860,176)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Codec   : {\\1c&H24BFFB&}%s  ·  %s\n" ..
        "{\\an7\\pos(860,200)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Output  : {\\1c&HFAF4F0&}%s  ·  %d ch\n" ..
        "{\\an7\\pos(860,224)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Volume  : {\\1c%s}%s\n" ..
        "{\\an7\\pos(860,248)\\bord0\\shad0\\fs16\\1c&HC8B8A4&}Window  : {\\1c&HF8BD38&}%s\n" ..
        "{\\an7\\pos(66,288)\\bord0\\shad0\\fs16\\b1\\1c&H99D334&}⚡ BUFFER HEALTH   {\\1c&HFAF4F0&\\b0}%.1fs ahead  ·  %.1f MiB  %s  ·  {\\1c%s}%s\n" ..
        "{\\an7\\pos(66,314)\\bord0\\shad0\\1c&H2B2420&\\p1}m 0 0 l 520 0 l 520 10 l 0 10{\\p0}\n" ..
        "{\\an7\\pos(66,314)\\bord0\\shad0\\1c%s\\p1}m 0 0 l %d 0 l %d 10 l 0 10{\\p0}\n" ..
        "{\\an7\\pos(66,340)\\bord0\\shad0\\fs15\\1c&HC8B8A4&}Keys: {\\1c&HF8BD38&}[i] Hide  [I] Pin 1Hz  {\\1c&H99D334&}[p] PiP Float  [P] PiP Corner  {\\1c&H24BFFB&}[f] Fullscreen  {\\1c&HFA8BA7&}[F1] Raw Stats",
        q_tag, win_badge, pin_badge, meta,
        mtitle,
        fmt_clock(pos), fmt_clock(dur), math.floor(pct), spd, chap_line,
        prog_w, prog_w,
        vw, vh, q_tag,
        vcodec, vbit_s,
        fps, disp_fps,
        drop_col, drops, cm,
        hw_col, hw_lbl,
        vo,
        eq_state,
        avs * 1000.0,
        acodec, abit_s,
        sr_s, ch_n,
        vol_col, vol_str,
        win_badge,
        c_dur, c_mib, (spd_txt ~= "") and ("· " .. spd_txt) or "", buf_col, buf_health,
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
    let hwdec_val = if opts.performance_mode == PerformanceMode::Manual {
        eff_master.hwdec.mpv_value()
    } else if matches!(
        eff_master.hwdec,
        crate::config::HwdecMode::VideoToolbox
    ) && cfg!(target_os = "macos")
        && opts.performance_mode == PerformanceMode::Balanced
    {
        "videotoolbox"
    } else {
        "auto-safe"
    };
    let mut flags = vec![
        format!("hwdec={hwdec_val}"),
        "vo=gpu-next".to_string(),
        "gpu-api=auto".to_string(),
        "cache=yes".to_string(),
        format!("demuxer-max-bytes={max_buf}"),
        format!("demuxer-max-back-bytes={back_buf}"),
        format!("force-media-title={}", opts.title),
    ];

    if opts.quality_format.starts_with("bestaudio") {
        flags.push("vid=no".to_string());
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
        let has_card = opts
            .loading_image
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty());
        script_opt_parts.push(format!("phaenna_has_card={}", if has_card { "1" } else { "0" }));
        script_opt_parts.push(format!("phaenna_auto_pip={}", if opts.auto_pip { "1" } else { "0" }));
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
    } else {
        match opts.motion_mode {
            MotionMode::Off => {
                flags.push("interpolation=no".to_string());
            }
            MotionMode::Cadence => {
                flags.push("video-sync=display-resample".to_string());
                flags.push("interpolation=yes".to_string());
                flags.push("tscale=oversample".to_string());
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
            flags.push("ytdl-raw-options-append=no-playlist=".to_string());
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

    let has_loading_slide = opts.player == Player::Mpv
        && opts
            .loading_image
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty());
    if !use_live_pipeline && !has_loading_slide {
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

/// Build arguments for `mpv` cleanly (pure function for unit testing).
pub fn build_mpv_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec!["--force-window=immediate".to_string()];
    if let Some(ref sock) = opts.ipc_socket {
        args.push(format!("--input-ipc-server={}", sock));
    }
    for kv in shared_mpv_kv_flags(opts) {
        args.push(format!("--{kv}"));
    }
    if let Some(img) = opts.loading_image.as_deref().filter(|p| !p.trim().is_empty()) {
        args.push("--image-display-duration=2".to_string());
        args.push("--prefetch-playlist=yes".to_string());
        args.push("--loop-playlist=no".to_string());
        args.push("--{".to_string());
        args.push(img.to_string());
        args.push("--}".to_string());
        args.push("--{".to_string());
        let use_live_pipeline = opts.is_live && opts.live_mode != LiveStreamMode::Off;
        if !use_live_pipeline {
            if let Some(start) = opts.start_pos.filter(|&s| s > 3.0) {
                args.push(format!("--start={:.1}", start));
            }
        }
        args.push(resolved_mpv_target(opts));
        args.push("--}".to_string());
    } else {
        args.push(resolved_mpv_target(opts));
    }
    args
}

/// Build arguments for macOS `iina-cli` translating all Reference 4K shaders,
/// AV1/VP9 format selection, dub preference, loudnorm, cookies, and mastering flags via `--mpv-*`.
pub fn build_iina_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec!["--no-stdin".to_string()];
    for kv in shared_mpv_kv_flags(opts) {
        args.push(format!("--mpv-{kv}"));
    }
    args.push(resolved_mpv_target(opts));
    args
}

/// Build arguments for `vlc` applying YouTube-parity keybindings in windowed & fullscreen mode.
pub fn build_vlc_args(opts: &PlayOptions) -> Vec<String> {
    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));
    let mut args = vec![
        format!("--video-title={}", opts.title),
        format!("--network-caching={}", opts.network_mode.vlc_network_caching_ms()),
        "--http-reconnect".to_string(),
        "--short-jump-size=10".to_string(),
        "--extrashort-jump-size=5".to_string(),
        "--key-play-pause=k".to_string(),
        "--key-jump-short=j".to_string(),
        "--key-jump+short=l".to_string(),
        "--key-jump-extrashort=Left".to_string(),
        "--key-jump+extrashort=Right".to_string(),
        "--key-vol-up=Up".to_string(),
        "--key-vol-down=Down".to_string(),
        "--key-vol-mute=m".to_string(),
        "--key-toggle-fullscreen=f".to_string(),
        "--key-leave-fullscreen=Esc".to_string(),
        "--key-subtitle-toggle=c".to_string(),
        "--key-audio-track=d".to_string(),
        "--key-rate-slower-fine=<".to_string(),
        "--key-rate-faster-fine=>".to_string(),
        "--key-frame-next=.".to_string(),
    ];
    if opts.auto_pip {
        args.push("--video-on-top".to_string());
    } else if opts.auto_fullscreen {
        args.push("--fullscreen".to_string());
    }
    args.push(target);
    args
}

/// Launch playback asynchronously in the background without blocking the TUI event loop!
pub fn spawn_player(opts: &PlayOptions) -> Result<Option<Child>> {
    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));

    match opts.player {
        Player::Mpv => {
            let bin = find_binary("mpv").ok_or_else(|| {
                AppError::Player(
                    "mpv was not found in PATH. Install mpv or switch player in [p] Playback Studio."
                        .to_string(),
                )
            })?;
            let args = build_mpv_args(opts);
            let child = Command::new(bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| AppError::Player(format!("Failed to launch mpv: {}", e)))?;
            Ok(Some(child))
        }
        Player::Iina => {
            #[cfg(target_os = "macos")]
            {
                if let Some(cli) = find_binary("iina-cli").or_else(|| find_binary("iina")) {
                    let args = build_iina_args(opts);
                    let child = Command::new(cli)
                        .args(&args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .map_err(|e| AppError::Player(format!("Failed to launch IINA: {}", e)))?;
                    return Ok(Some(child));
                }
                let child = Command::new("open")
                    .args(["-a", "IINA", &target])
                    .spawn()
                    .map_err(|e| AppError::Player(format!("Failed to open IINA: {}", e)))?;
                Ok(Some(child))
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err(AppError::Player("IINA is only available on macOS".to_string()))
            }
        }
        Player::Vlc => {
            let bin = find_binary("vlc").ok_or_else(|| {
                AppError::Player("VLC not found on PATH or /Applications".to_string())
            })?;
            let args = build_vlc_args(opts);
            let child = Command::new(bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| AppError::Player(format!("Failed to launch VLC: {}", e)))?;
            Ok(Some(child))
        }
        Player::Browser => {
            #[cfg(target_os = "macos")]
            let child = Command::new("open").arg(&target).spawn();
            #[cfg(target_os = "linux")]
            let child = Command::new("xdg-open").arg(&target).spawn();
            #[cfg(windows)]
            let child = Command::new("cmd").args(["/C", "start", "", &target]).spawn();

            let c = child
                .map_err(|e| AppError::Player(format!("Failed to open browser: {}", e)))?;
            Ok(Some(c))
        }
    }
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
            loading_image: None,
            picture: PictureGrade::default(),
            direct_stream: None,
        };

        let args = build_mpv_args(&opts);
        assert!(args.iter().any(|a| a == "--input-ipc-server=/tmp/yt-test.sock"));
        assert!(args.iter().any(|a| a == "--start=42.5"));
        assert!(args.iter().any(|a| a.starts_with("--glsl-shaders=")));
        assert!(args.iter().any(|a| a == "--demuxer-max-bytes=512MiB"));
        assert!(args.iter().any(|a| a.contains("format-sort=lang,res:2160")));
        assert!(args.iter().any(|a| a == "--ytdl-raw-options-append=audio-multistreams="));
        assert!(args.iter().any(|a| a == "--alang=und,orig"));
        assert!(args.iter().any(|a| a.starts_with("--af=lavfi=")));
        assert!(args.iter().any(|a| a == "--video-sync=display-resample"));
        assert!(args.iter().any(|a| a == "--override-display-fps=120"));
        assert!(args.iter().any(|a| a.starts_with("--input-conf=")));
        assert!(args.iter().any(|a| a.starts_with("--script=")));
        assert!(args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));

        let vlc_args = build_vlc_args(&opts);
        assert!(vlc_args.iter().any(|a| a == "--key-play-pause=k"));
        assert!(vlc_args.iter().any(|a| a == "--key-toggle-fullscreen=f"));

        let mut eco_opts = opts.clone();
        eco_opts.performance_mode = PerformanceMode::Efficient;
        let eco_args = build_mpv_args(&eco_opts);
        assert!(eco_args.iter().any(|a| a == "--demuxer-max-bytes=48MiB"));
        assert!(!eco_args.iter().any(|a| a.contains("audio-multistreams")));
        assert!(eco_args.iter().any(|a| a == "--scale=spline36"));

        let mut bal_opts = opts.clone();
        bal_opts.performance_mode = PerformanceMode::Balanced;
        let bal_args = build_mpv_args(&bal_opts);
        assert!(bal_args.iter().any(|a| a == "--hdr-compute-peak=no"));
        assert!(bal_args.iter().any(|a| a == "--cscale=mitchell"));
        assert!(bal_args.iter().any(|a| a.contains("vcodec:vp9.2:vp9:h264:av01")));

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
    fn test_build_iina_args_full_4k_shader_parity() {
        let opts = PlayOptions {
            video_id: "dQw4w9WgXcQ".to_string(),
            title: "Reference 4K Test".to_string(),
            local_file: None,
            quality_format: "bestvideo[height<=?4320][vcodec^=av01]+bestaudio/best".to_string(),
            player: Player::Iina,
            cookies_path: None,
            start_pos: Some(18.0),
            ipc_socket: None,
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
            auto_fullscreen: false,
            auto_pip: false,
            loading_image: None,
            picture: PictureGrade::default(),
            direct_stream: None,
        };

        let args = build_iina_args(&opts);
        assert!(args.iter().any(|a| a == "--no-stdin"));
        assert!(args.iter().any(|a| a.starts_with("--mpv-glsl-shaders=")));
        assert!(args.iter().any(|a| a == "--mpv-deband=yes"));
        assert!(args.iter().any(|a| a == "--mpv-tone-mapping=spline"));
        assert!(args.iter().any(|a| a == "--mpv-start=18.0"));
        assert!(args.iter().any(|a| a == "--mpv-alang=ar"));
        assert!(args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));
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
            loading_image: None,
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
            loading_image: None,
        };

        let wifi_args = build_mpv_args(&opts);
        assert!(wifi_args.iter().any(|a| a.contains("reconnect=1")));
        assert!(wifi_args.iter().any(|a| a == "--network-timeout=12"));
        assert!(wifi_args.iter().any(|a| a == "--cache-pause-wait=4.5"));
        assert!(wifi_args.iter().any(|a| a == "--demuxer-readahead-secs=120"));
        assert!(wifi_args.iter().any(|a| a == "--cache-on-disk=yes"));
        assert!(wifi_args.iter().any(|a| a == "--ytdl-raw-options-append=force-ipv4="));
        assert!(!wifi_args.iter().any(|a| a.contains("audio-multistreams")));

        let vlc_args = build_vlc_args(&opts);
        assert!(vlc_args.iter().any(|a| a == "--network-caching=8000"));
        assert!(vlc_args.iter().any(|a| a == "--http-reconnect"));

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
        auto_fs_opts.loading_image = Some("/tmp/y2k_card.jpg".to_string());
        auto_fs_opts.start_pos = Some(65.0);
        let fs_args = build_mpv_args(&auto_fs_opts);
        assert!(fs_args.iter().any(|a| a == "--fullscreen=yes"));
        assert!(fs_args.iter().any(|a| a == "--prefetch-playlist=yes"));
        assert!(fs_args.iter().any(|a| a == "/tmp/y2k_card.jpg"));
        assert!(fs_args.iter().any(|a| a == "--start=65.0"));
        assert!(build_vlc_args(&auto_fs_opts).iter().any(|a| a == "--fullscreen"));

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
}
