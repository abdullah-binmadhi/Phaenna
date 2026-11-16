//! Cross-platform binary detector & non-blocking external video player launcher.

use crate::config::Player;
use crate::error::{AppError, Result};
use crate::services::shaders::{mpv_mastering_flags, prepare_video_shaders, VideoMasteringProfile};
use crate::services::substyler::SubtitleStyleConfig;
use crate::config::VideoShaderMode;
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
    pub ytdlp_path: Option<PathBuf>,
    pub mpv_path: Option<PathBuf>,
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
    let ytdlp_path = find_binary("yt-dlp");
    let mpv_path = find_binary("mpv");
    let has_iina = find_binary("iina-cli").is_some()
        || find_binary("iina").is_some()
        || Path::new("/Applications/IINA.app").exists();
    let has_vlc = find_binary("vlc").is_some()
        || Path::new("/Applications/VLC.app").exists();
    let has_ffmpeg = find_binary("ffmpeg").is_some();

    DependencyStatus {
        has_ytdlp: ytdlp_path.is_some(),
        has_mpv: mpv_path.is_some(),
        has_iina,
        has_vlc,
        has_ffmpeg,
        ytdlp_path,
        mpv_path,
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
}

pub fn youtube_watch_url(video_id: &str) -> String {
    let clean_id = video_id.trim_start_matches("RD");
    format!("https://www.youtube.com/watch?v={}", clean_id)
}

/// Build arguments for `mpv` cleanly (pure function for unit testing).
pub fn build_mpv_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec![
        "--hwdec=auto-safe".to_string(),
        "--vo=gpu-next".to_string(),
        "--gpu-api=auto".to_string(),
        "--cache=yes".to_string(),
        "--demuxer-max-bytes=512MiB".to_string(),
        "--demuxer-max-back-bytes=128MiB".to_string(),
        "--force-window=immediate".to_string(),
        format!("--force-media-title={}", opts.title),
    ];

    if opts.local_file.is_none() {
        args.push(format!("--ytdl-format={}", opts.quality_format));
        args.push(
            "--ytdl-raw-options-append=format-sort=res:2160,fps,hdr,vcodec:av01:vp9.2:vp9,vbr,abr"
                .to_string(),
        );
        if let Some(ref cpath) = opts.cookies_path {
            if cpath.exists() {
                args.push(format!(
                    "--ytdl-raw-options-append=cookies={}",
                    cpath.to_string_lossy()
                ));
                args.push("--ytdl-raw-options-append=mark-watched=".to_string());
            }
        }
    }

    if let Some(ref sock) = opts.ipc_socket {
        args.push(format!("--input-ipc-server={}", sock));
    }

    if let Some(start) = opts.start_pos {
        if start > 3.0 {
            args.push(format!("--start={:.1}", start));
        }
    }

    if opts.subtitles_enabled {
        let lang = opts.sub_lang.as_deref().unwrap_or("en,eng");
        args.push(format!("--slang={}", lang));
        args.push("--sub-auto=fuzzy".to_string());
        if let Some(ref sub_url) = opts.sub_url {
            args.push(format!("--sub-file={}", sub_url));
        }
        args.extend(opts.sub_style.to_mpv_flags());
    }

    if let Some(shaders) = prepare_video_shaders(opts.shader_mode) {
        args.push(format!("--glsl-shaders={}", shaders));
        args.extend(mpv_mastering_flags(opts.shader_mode, opts.mastering_profile));
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
/// AV1/VP9 format selection, cookies, and mastering flags via `--mpv-*` parity.
pub fn build_iina_args(opts: &PlayOptions) -> Vec<String> {
    let mut args = vec![
        "--no-stdin".to_string(),
        "--mpv-hwdec=auto-safe".to_string(),
        "--mpv-vo=gpu-next".to_string(),
        "--mpv-gpu-api=auto".to_string(),
        "--mpv-cache=yes".to_string(),
        "--mpv-demuxer-max-bytes=512MiB".to_string(),
        "--mpv-demuxer-max-back-bytes=128MiB".to_string(),
        format!("--mpv-force-media-title={}", opts.title),
    ];

    if opts.local_file.is_none() {
        args.push(format!("--mpv-ytdl-format={}", opts.quality_format));
        args.push(
            "--mpv-ytdl-raw-options-append=format-sort=res:2160,fps,hdr,vcodec:av01:vp9.2:vp9,vbr,abr"
                .to_string(),
        );
        if let Some(ref cpath) = opts.cookies_path {
            if cpath.exists() {
                args.push(format!(
                    "--mpv-ytdl-raw-options-append=cookies={}",
                    cpath.to_string_lossy()
                ));
                args.push("--mpv-ytdl-raw-options-append=mark-watched=".to_string());
            }
        }
    }

    if let Some(start) = opts.start_pos {
        if start > 3.0 {
            args.push(format!("--mpv-start={:.1}", start));
        }
    }

    if opts.subtitles_enabled {
        let lang = opts.sub_lang.as_deref().unwrap_or("en,eng");
        args.push(format!("--mpv-slang={}", lang));
        args.push("--mpv-sub-auto=fuzzy".to_string());
        if let Some(ref sub_url) = opts.sub_url {
            args.push(format!("--mpv-sub-file={}", sub_url));
        }
        for flag in opts.sub_style.to_mpv_flags() {
            if let Some(trimmed) = flag.strip_prefix("--") {
                args.push(format!("--mpv-{}", trimmed));
            }
        }
    }

    if let Some(shaders) = prepare_video_shaders(opts.shader_mode) {
        args.push(format!("--mpv-glsl-shaders={}", shaders));
        for flag in mpv_mastering_flags(opts.shader_mode, opts.mastering_profile) {
            if let Some(trimmed) = flag.strip_prefix("--") {
                args.push(format!("--mpv-{}", trimmed));
            }
        }
    }

    let target = opts
        .local_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| youtube_watch_url(&opts.video_id));
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
                    "mpv was not found in PATH. Install mpv or switch player with [!] Setup."
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
            let child = Command::new(bin)
                .arg(&target)
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
        };

        let args = build_mpv_args(&opts);
        assert!(args.iter().any(|a| a == "--input-ipc-server=/tmp/yt-test.sock"));
        assert!(args.iter().any(|a| a == "--start=42.5"));
        assert!(args.iter().any(|a| a.starts_with("--glsl-shaders=")));
        assert!(args.iter().any(|a| a == "--demuxer-max-bytes=512MiB"));
        assert!(args.iter().any(|a| a.contains("format-sort=res:2160")));
        assert!(args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));
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
        };

        let args = build_iina_args(&opts);
        assert!(args.iter().any(|a| a == "--no-stdin"));
        assert!(args.iter().any(|a| a.starts_with("--mpv-glsl-shaders=")));
        assert!(args.iter().any(|a| a == "--mpv-deband=yes"));
        assert!(args.iter().any(|a| a == "--mpv-tone-mapping=spline"));
        assert!(args.iter().any(|a| a == "--mpv-start=18.0"));
        assert!(args.last().unwrap().ends_with("v=dQw4w9WgXcQ"));
    }
}
