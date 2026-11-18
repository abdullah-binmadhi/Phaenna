//! Cross-platform binary detector & non-blocking external video player launcher.

use crate::config::{
    Config, DubPreference, LiveStreamMode, MotionMode, PerformanceMode, Player, VideoShaderMode,
};
use crate::error::{AppError, Result};
use crate::services::shaders::{
    mpv_mastering_flags_for, prepare_video_shaders_for, VideoMasteringProfile,
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
    pub motion_mode: MotionMode,
    pub dub_preference: DubPreference,
    pub dub_lang: String,
    pub audio_multistreams: bool,
    pub audio_loudnorm: bool,
    pub is_live: bool,
    pub live_mode: LiveStreamMode,
    pub live_from_start: bool,
    pub live_auto_catchup: bool,
}

pub const LOUDNORM_FILTER: &str = "lavfi=[dynaudnorm=f=150:g=15:p=0.9]";

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

const PHAENNA_LOADER_LUA: &str = r#"-- Phaenna Animated ASS Loading Bar Overlay (replaces startup black screen until first frame)
local ov = mp.create_osd_overlay("ass-events")
ov.res_x = 1280
ov.res_y = 720

local title = mp.get_opt("phaenna_title") or "Preparing Stream"
local meta = mp.get_opt("phaenna_meta") or "PHAENNA STUDIO"
if #title > 62 then title = string.sub(title, 1, 59) .. "..." end
title = title:gsub("[{}]", "")
meta = meta:gsub("[{}]", "")

local phase = 0.0
local cache_pct = nil
local is_loading = true
local timer = nil

local function render_loader()
    if not is_loading then
        ov:remove()
        return
    end
    phase = (phase + 0.045) % 1.0
    local bar_x0 = 280
    local bar_w = 720
    local bar_y0 = 404
    local bar_y1 = 418

    local status = "Connecting & Buffering HD Stream..."
    local fill_x0 = bar_x0
    local fill_w = 180
    if cache_pct and cache_pct > 0 and cache_pct < 100 then
        status = string.format("Buffering Stream (%d%%)...", math.floor(cache_pct))
        fill_w = math.max(24, math.floor(bar_w * (cache_pct / 100.0)))
    else
        local sweep = math.floor(phase * (bar_w - 160))
        fill_x0 = bar_x0 + sweep
        fill_w = 160
    end
    local fill_x1 = math.min(bar_x0 + bar_w, fill_x0 + fill_w)

    local ass = string.format(
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H181412&\\1a&H14&\\p1}m 220 265 l 1060 265 l 1060 455 l 220 455{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord2\\1a&HFF&\\3c&H8A5CF6&\\p1}m 220 265 l 1060 265 l 1060 455 l 220 455{\\p0}\n" ..
        "{\\an5\\pos(640,305)\\bord0\\shad0\\fs20\\b1\\1c&H99D334&}%s\n" ..
        "{\\an5\\pos(640,344)\\bord0\\shad0\\fs26\\b1\\1c&HFFFFFF&}%s\n" ..
        "{\\an5\\pos(640,378)\\bord0\\shad0\\fs17\\1c&HB0B8C8&}%s\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H2E2824&\\p1}m %d %d l %d %d l %d %d l %d %d{\\p0}\n" ..
        "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&HF65C8B&\\p1}m %d %d l %d %d l %d %d l %d %d{\\p0}",
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
    is_loading = false
    if timer then
        timer:kill()
        timer = nil
    end
    ov:remove()
end

mp.register_event("playback-restart", function()
    local pfc = mp.get_property_bool("paused-for-cache", false)
    if not pfc then stop_anim() end
end)

mp.observe_property("paused-for-cache", "bool", function(_, val)
    if val then
        is_loading = true
        start_anim()
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

mp.observe_property("cache-buffering-state", "number", function(_, val)
    cache_pct = val
end)

start_anim()
"#;

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
    let (max_buf, back_buf) = if use_live_pipeline {
        opts.live_mode.demuxer_limits()
    } else {
        opts.performance_mode.demuxer_limits()
    };
    let mut flags = vec![
        "hwdec=auto-safe".to_string(),
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

    if let Some(conf_path) = prepare_youtube_input_conf() {
        flags.push(format!("input-conf={}", conf_path.to_string_lossy()));
    }

    let mut script_opt_parts: Vec<String> = Vec::new();
    if let Some(lua_path) = prepare_loading_overlay_script() {
        flags.push(format!("script={}", lua_path.to_string_lossy()));
        let safe_title = sanitize_script_opt(&opts.title);
        let safe_meta = if use_live_pipeline {
            format!("PHAENNA LIVE | {}", opts.live_mode.short_badge())
        } else {
            format!(
                "PHAENNA {} | {}",
                opts.performance_mode.short_badge(),
                opts.motion_mode.short_badge()
            )
        };
        script_opt_parts.push(format!("phaenna_title={safe_title}"));
        script_opt_parts.push(format!("phaenna_meta={safe_meta}"));
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
        flags.push(format!(
            "cache-pause-wait={:.1}",
            opts.live_mode.cache_pause_wait_secs()
        ));
        flags.push("network-timeout=15".to_string());
        flags.push("hls-bitrate=max".to_string());
        flags.push("stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=4".to_string());
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
    }

    if !script_opt_parts.is_empty() {
        flags.push(format!("script-opts={}", script_opt_parts.join(",")));
    }

    if opts.local_file.is_none() {
        flags.push(format!("ytdl-format={}", opts.quality_format));
        let sort_spec = match opts.dub_preference {
            // Keep `lang` ahead of `abr` so YouTube's `language_preference=10` (Original)
            // never loses to slightly higher-bitrate AI-dubbed audio tracks.
            DubPreference::OriginalOnly => "lang,res:2160,fps,hdr,vcodec:av01:vp9.2:vp9,vbr,abr",
            DubPreference::PreferDubbed => "res:2160,fps,hdr,vcodec:av01:vp9.2:vp9,vbr,abr",
        };
        flags.push(format!("ytdl-raw-options-append=format-sort={sort_spec}"));
        if !use_live_pipeline && opts.performance_mode.loads_multistreams(opts.audio_multistreams) {
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

    if opts.audio_loudnorm {
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
        if let Some(ref sub_url) = opts.sub_url {
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
        if let Some(shaders) = prepare_video_shaders_for(opts.shader_mode, opts.performance_mode) {
            flags.push(format!("glsl-shaders={}", shaders));
        }
        for flag in mpv_mastering_flags_for(opts.shader_mode, opts.mastering_profile, opts.performance_mode) {
            flags.push(flag.trim_start_matches("--").to_string());
        }
    }

    flags
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
    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));
    args.push(target);
    args
}

/// Build arguments for macOS `iina-cli` translating all Reference 4K shaders,
/// AV1/VP9 format selection, dub preference, loudnorm, cookies, and mastering flags via `--mpv-*`.
pub fn build_iina_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec!["--no-stdin".to_string()];
    for kv in shared_mpv_kv_flags(opts) {
        args.push(format!("--mpv-{kv}"));
    }
    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));
    args.push(target);
    args
}

/// Build arguments for `vlc` applying YouTube-parity keybindings in windowed & fullscreen mode.
pub fn build_vlc_args(opts: &PlayOptions) -> Vec<String> {
    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));
    vec![
        format!("--video-title={}", opts.title),
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
        target,
    ]
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
            motion_mode: MotionMode::Smooth120Hz,
            dub_preference: DubPreference::OriginalOnly,
            dub_lang: "en".to_string(),
            audio_multistreams: true,
            audio_loudnorm: true,
            is_live: false,
            live_mode: LiveStreamMode::BalancedLive,
            live_from_start: false,
            live_auto_catchup: true,
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
            motion_mode: MotionMode::Cadence,
            dub_preference: DubPreference::PreferDubbed,
            dub_lang: "ar".to_string(),
            audio_multistreams: true,
            audio_loudnorm: false,
            is_live: false,
            live_mode: LiveStreamMode::BalancedLive,
            live_from_start: false,
            live_auto_catchup: true,
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
            motion_mode: MotionMode::Smooth120Hz,
            dub_preference: DubPreference::OriginalOnly,
            dub_lang: "en".to_string(),
            audio_multistreams: true,
            audio_loudnorm: false,
            is_live: true,
            live_mode: LiveStreamMode::LowLatency,
            live_from_start: false,
            live_auto_catchup: true,
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
}
