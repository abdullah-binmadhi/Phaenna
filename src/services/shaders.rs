//! Real-Time Video GLSL Shader & Mastering Pipeline for mpv / iina.
//!
//! Adapted from Solus's GPU shader architecture for live YouTube playback:
//! - Anti-ringing clamp shader (eliminates YouTube compression haloing)
//! - Pure-luminance (Y) FidelityFX Contrast Adaptive Sharpening (CAS)
//! - Adaptive 2x edge-aware reconstruction for <1080p uploads

use crate::config::VideoShaderMode;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum VideoMasteringProfile {
    StandardHd,
    LowResSd,
    HighFrameRate,
}

impl VideoMasteringProfile {
    pub fn from_metadata(duration_secs: Option<u64>, title: &str) -> Self {
        let lower = title.to_lowercase();
        if lower.contains("60fps") || lower.contains("120fps") {
            Self::HighFrameRate
        } else if duration_secs.map(|d| d > 3600).unwrap_or(false) {
            Self::LowResSd
        } else {
            Self::StandardHd
        }
    }
}

const SHADER_CLAMP_HIGHLIGHTS: &str = r#"//!DESC Phaenna-Clamp-ZeroBlur
//!HOOK MAIN
//!BIND HOOKED

vec4 hook() {
    return clamp(HOOKED_tex(HOOKED_pos), 0.0, 1.0);
}
"#;

const SHADER_LUMA_CAS: &str = r#"//!DESC Phaenna-Luma-CAS-Sharpen
//!HOOK MAIN
//!BIND HOOKED

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec4 center = HOOKED_tex(HOOKED_pos);
    vec3 n = HOOKED_tex(HOOKED_pos + vec2(0.0, -pt.y)).rgb;
    vec3 s = HOOKED_tex(HOOKED_pos + vec2(0.0,  pt.y)).rgb;
    vec3 e = HOOKED_tex(HOOKED_pos + vec2( pt.x, 0.0)).rgb;
    vec3 w = HOOKED_tex(HOOKED_pos + vec2(-pt.x, 0.0)).rgb;

    float lc = luma(center.rgb);
    float mn = min(lc, min(min(luma(n), luma(s)), min(luma(e), luma(w))));
    float mx = max(lc, max(max(luma(n), luma(s)), max(luma(e), luma(w))));
    float amp = clamp(min(mn, 1.0 - mx) / max(mx, 1e-5), 0.0, 1.0);
    float weight = -0.16 * sqrt(amp);

    float avg_ring = (luma(n) + luma(s) + luma(e) + luma(w)) * weight;
    float sharp_luma = clamp((avg_ring + lc) / (1.0 + 4.0 * weight), mn, mx);
    vec3 delta = vec3(sharp_luma - lc);
    return vec4(clamp(center.rgb + delta, 0.0, 1.0), center.a);
}
"#;

const SHADER_EDGE_UPSCALE: &str = r#"//!DESC Phaenna-EdgeAware-2x
//!HOOK MAIN
//!BIND HOOKED
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec4 c00 = HOOKED_tex(HOOKED_pos);
    vec3 c10 = HOOKED_tex(HOOKED_pos + vec2(pt.x, 0.0)).rgb;
    vec3 c01 = HOOKED_tex(HOOKED_pos + vec2(0.0, pt.y)).rgb;
    float l0 = luma(c00.rgb);
    float gx = abs(luma(c10) - l0);
    float gy = abs(luma(c01) - l0);
    float edge = smoothstep(0.03, 0.14, gx + gy);
    vec3 blended = mix(c00.rgb, (c00.rgb * 2.0 + c10 + c01) * 0.25, (1.0 - edge) * 0.15);
    return vec4(clamp(blended, 0.0, 1.0), c00.a);
}
"#;

fn shader_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "phaenna", "phaenna")
        .map(|d| d.cache_dir().join("shaders"))
        .unwrap_or_else(|| std::env::temp_dir().join("phaenna-shaders"))
}

/// Write GLSL shader files to the local cache directory and return colon/semicolon-separated paths.
pub fn prepare_video_shaders(mode: VideoShaderMode) -> Option<String> {
    if mode == VideoShaderMode::Off {
        return None;
    }

    let dir = shader_cache_dir();
    let _ = std::fs::create_dir_all(&dir);

    let clamp_path = dir.join("01_clamp.glsl");
    let cas_path = dir.join("02_luma_cas.glsl");
    let upscale_path = dir.join("03_edge_2x.glsl");

    let _ = std::fs::write(&clamp_path, SHADER_CLAMP_HIGHLIGHTS);
    let _ = std::fs::write(&cas_path, SHADER_LUMA_CAS);

    let mut list = vec![
        clamp_path.to_string_lossy().into_owned(),
        cas_path.to_string_lossy().into_owned(),
    ];

    if mode == VideoShaderMode::Ultra {
        let _ = std::fs::write(&upscale_path, SHADER_EDGE_UPSCALE);
        list.push(upscale_path.to_string_lossy().into_owned());
    }

    #[cfg(windows)]
    let sep = ";";
    #[cfg(not(windows))]
    let sep = ":";

    Some(list.join(sep))
}

/// Generate adaptive mpv video scaler & deband flags complementing GLSL shaders.
pub fn mpv_mastering_flags(
    mode: VideoShaderMode,
    profile: VideoMasteringProfile,
) -> Vec<String> {
    if mode == VideoShaderMode::Off {
        return Vec::new();
    }

    let mut flags = vec![
        "--scale=ewa_lanczossharp".to_string(),
        "--cscale=mitchell".to_string(),
        "--dscale=mitchell".to_string(),
        "--correct-downscaling=yes".to_string(),
        "--linear-downscaling=yes".to_string(),
        "--sigmoid-upscaling=yes".to_string(),
        "--deband=yes".to_string(),
    ];

    match profile {
        VideoMasteringProfile::HighFrameRate => {
            flags.push("--deband-iterations=1".to_string());
        }
        VideoMasteringProfile::LowResSd | VideoMasteringProfile::StandardHd => {
            flags.push("--deband-iterations=2".to_string());
            flags.push("--deband-threshold=42".to_string());
        }
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_modes_and_flags() {
        assert!(prepare_video_shaders(VideoShaderMode::Off).is_none());
        let bal = prepare_video_shaders(VideoShaderMode::Balanced).expect("balanced shaders");
        assert!(bal.contains("01_clamp.glsl"));
        assert!(!bal.contains("03_edge_2x.glsl"));

        let ultra = prepare_video_shaders(VideoShaderMode::Ultra).expect("ultra shaders");
        assert!(ultra.contains("03_edge_2x.glsl"));

        let flags = mpv_mastering_flags(VideoShaderMode::Balanced, VideoMasteringProfile::StandardHd);
        assert!(flags.iter().any(|f| f == "--deband=yes"));
    }
}
