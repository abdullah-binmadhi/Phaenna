//! Phaenna Reference 4K Studio — Real-Time Video De-Compression & Super-Resolution GLSL Pipeline.
//!
//! Solves YouTube AV1/VP9/AVC bitrate starvation & 4:2:0 subsampling for `mpv` and `iina`:
//! 1. `01_deblock_dering.glsl` (`LUMA`): Edge-preserving bilateral DCT compression block & mosquito-noise cleaner.
//! 2. `02_chroma_guided.glsl` (`CHROMA` + `LUMA`): Luma-guided 4:2:0 -> 4:4:4 chroma reconstructor stopping color bleed.
//! 3. `03_superres_4k_rcas.glsl` (`LUMA` 2x): Directional structure-tensor upscaler + FidelityFX RCAS with strict anti-ringing.
//! 4. `04_anime4k_restore.glsl` (`LUMA` 2x): Gradient clamp & line-art thinness reconstruction for stylized animation.
//! 5. Optional neural CNN upscaler (`FSRCNNX_x2_16-0-4-1.glsl`) downloadable into local shader cache.

use crate::config::VideoShaderMode;
use std::path::PathBuf;

const FSRCNNX_URL: &str =
    "https://raw.githubusercontent.com/igv/FSRCNN-TensorFlow/master/GLSL/FSRCNNX_x2_16-0-4-1.glsl";
const FSRCNNX_FILENAME: &str = "fsrcnnx_x2_16_0_4_1.glsl";

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

/// 1. Edge-preserving bilateral deblocker & deringer for YouTube DCT compression macroblocks.
const SHADER_DEBLOCK_DERING: &str = r#"//!DESC Phaenna-Ref4K-Deblock-Dering
//!HOOK LUMA
//!BIND HOOKED

vec4 hook() {
    vec2 pt = HOOKED_pt;
    float c = HOOKED_tex(HOOKED_pos).x;
    float n = HOOKED_tex(HOOKED_pos + vec2(0.0, -pt.y)).x;
    float s = HOOKED_tex(HOOKED_pos + vec2(0.0,  pt.y)).x;
    float e = HOOKED_tex(HOOKED_pos + vec2( pt.x, 0.0)).x;
    float w = HOOKED_tex(HOOKED_pos + vec2(-pt.x, 0.0)).x;

    // Range-sigma bilateral weight preserves true detail edges while dissolving flat DCT macroblocks
    float sigma = 0.038;
    float wn = exp(-((n - c) * (n - c)) / (2.0 * sigma * sigma));
    float ws = exp(-((s - c) * (s - c)) / (2.0 * sigma * sigma));
    float we = exp(-((e - c) * (e - c)) / (2.0 * sigma * sigma));
    float ww = exp(-((w - c) * (w - c)) / (2.0 * sigma * sigma));

    float wsum = 1.0 + wn + ws + we + ww;
    float filtered = (c + n * wn + s * ws + e * we + w * ww) / wsum;

    // Protect micro-contrast texture (film grain, hair, foliage)
    float local_contrast = max(max(abs(n - c), abs(s - c)), max(abs(e - c), abs(w - c)));
    float protect = smoothstep(0.025, 0.11, local_contrast);
    float out_y = mix(filtered, c, protect * 0.72);
    return vec4(clamp(out_y, 0.0, 1.0), 0.0, 0.0, 1.0);
}
"#;

/// 2. Luma-guided joint-bilateral 4:2:0 -> 4:4:4 chroma reconstructor (eliminates red/cyan text fringing).
const SHADER_CHROMA_GUIDED: &str = r#"//!DESC Phaenna-Ref4K-Chroma-444-Guided
//!HOOK CHROMA
//!BIND HOOKED
//!BIND LUMA

vec4 hook() {
    vec2 uv = HOOKED_pos;
    vec2 c_pt = HOOKED_pt;
    vec2 l_pt = LUMA_pt;

    float yc = LUMA_tex(uv).x;
    vec2 c0 = HOOKED_tex(uv).xy;
    vec2 cn = HOOKED_tex(uv + vec2(0.0, -c_pt.y)).xy;
    vec2 cs = HOOKED_tex(uv + vec2(0.0,  c_pt.y)).xy;
    vec2 ce = HOOKED_tex(uv + vec2( c_pt.x, 0.0)).xy;
    vec2 cw = HOOKED_tex(uv + vec2(-c_pt.x, 0.0)).xy;

    float yn = LUMA_tex(uv + vec2(0.0, -l_pt.y * 2.0)).x;
    float ys = LUMA_tex(uv + vec2(0.0,  l_pt.y * 2.0)).x;
    float ye = LUMA_tex(uv + vec2( l_pt.x * 2.0, 0.0)).x;
    float yw = LUMA_tex(uv + vec2(-l_pt.x * 2.0, 0.0)).x;

    float sigma = 0.045;
    float wn = exp(-((yn - yc) * (yn - yc)) / (2.0 * sigma * sigma));
    float ws = exp(-((ys - yc) * (ys - yc)) / (2.0 * sigma * sigma));
    float we = exp(-((ye - yc) * (ye - yc)) / (2.0 * sigma * sigma));
    float ww = exp(-((yw - yc) * (yw - yc)) / (2.0 * sigma * sigma));

    float norm = 1.0 + wn + ws + we + ww;
    vec2 guided = (c0 + cn * wn + cs * ws + ce * we + cw * ww) / norm;
    return vec4(clamp(guided, 0.0, 1.0), 0.0, 1.0);
}
"#;

/// 3. Directional 2x Super-Resolution + FidelityFX RCAS with strict 3x3 neighborhood anti-ringing.
const SHADER_SUPERRES_4K_RCAS: &str = r#"//!DESC Phaenna-Ref4K-SuperRes-RCAS-2x
//!HOOK LUMA
//!BIND HOOKED
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *
//!WHEN OUTPUT.w HOOKED.w / 1.200 >

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;

    float c = HOOKED_tex(pos).x;
    float n = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float s = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float e = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float w = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;
    float nw = HOOKED_tex(pos + vec2(-pt.x, -pt.y)).x;
    float ne = HOOKED_tex(pos + vec2( pt.x, -pt.y)).x;
    float sw = HOOKED_tex(pos + vec2(-pt.x,  pt.y)).x;
    float se = HOOKED_tex(pos + vec2( pt.x,  pt.y)).x;

    // 3x3 min/max bounds eliminate overshoot ringing around high-contrast edges
    float mn4 = min(min(n, s), min(e, w));
    float mx4 = max(max(n, s), max(e, w));
    float mn = min(min(c, mn4), min(min(nw, ne), min(sw, se)));
    float mx = max(max(c, mx4), max(max(nw, ne), max(sw, se)));

    // Directional structure-tensor edge orientation weight
    float gx = abs(e - w) + 0.5 * (abs(ne - nw) + abs(se - sw));
    float gy = abs(s - n) + 0.5 * (abs(sw - nw) + abs(se - ne));
    float aniso = clamp((gx - gy) / (gx + gy + 1e-5), -1.0, 1.0);

    // Sub-pixel edge-aligned reconstruction prior to RCAS pass
    float recon_v = (n + c * 2.0 + s) * 0.25;
    float recon_h = (w + c * 2.0 + e) * 0.25;
    float edge_mag = smoothstep(0.04, 0.22, gx + gy);
    float base = mix(c, mix(recon_h, recon_v, 0.5 + 0.5 * aniso), edge_mag * 0.35);

    // AMD FidelityFX RCAS contrast-adaptive sharpening (0.28 studio sweet-spot)
    float hit_min = min(mn4, base) / max(4.0 * mx4, 1e-5);
    float hit_max = (1.0 - max(mx4, base)) / max(4.0 * mn4 - 4.0, -1e-5);
    float lobe = max(-hit_min, hit_max);
    float sharpness = 0.28;
    float weight = clamp(lobe, -0.1875 * sharpness, 0.0);

    float rcas = (base + (n + s + e + w) * weight) / (1.0 + 4.0 * weight);
    return vec4(clamp(rcas, mn, mx), 0.0, 0.0, 1.0);
}
"#;

/// 4. Anime4K-inspired gradient clamp & line-art thinness restoration for animation content.
const SHADER_ANIME4K_RESTORE: &str = r#"//!DESC Phaenna-Anime4K-Line-Restore-2x
//!HOOK LUMA
//!BIND HOOKED
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *
//!WHEN OUTPUT.w HOOKED.w / 1.200 >

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;

    float mc = HOOKED_tex(pos).x;
    float mn = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float ms = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float me = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float mw = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;

    // Push dim borders closer to darkest dark-line core for crisp hand-drawn contours
    float min_n = min(mc, min(min(mn, ms), min(me, mw)));
    float max_n = max(mc, max(max(mn, ms), max(me, mw)));
    float gx = abs(me - mw);
    float gy = abs(ms - mn);
    float grad = clamp(sqrt(gx * gx + gy * gy) * 1.4, 0.0, 1.0);

    float refined = mix(mc, min_n, grad * 0.42);
    return vec4(clamp(refined, min_n, max_n), 0.0, 0.0, 1.0);
}
"#;

pub fn shader_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "phaenna", "phaenna")
        .map(|d| d.cache_dir().join("shaders"))
        .unwrap_or_else(|| std::env::temp_dir().join("phaenna-shaders"))
}

pub fn neural_shader_path() -> PathBuf {
    shader_cache_dir().join(FSRCNNX_FILENAME)
}

pub fn neural_shader_installed() -> bool {
    neural_shader_path().exists()
}

/// Download optional `FSRCNNX_x2_16-0-4-1.glsl` neural CNN shader pack for Neural Cinema 4K mode.
pub async fn download_neural_shader_pack(
    client: &reqwest::Client,
) -> std::result::Result<PathBuf, String> {
    let dir = shader_cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed creating shader dir: {e}"))?;
    let resp = client
        .get(FSRCNNX_URL)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Network error downloading FSRCNNX: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} downloading FSRCNNX pack", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed reading FSRCNNX response: {e}"))?;
    if !body.contains("//!HOOK") {
        return Err("Downloaded file did not contain valid mpv //!HOOK GLSL headers".to_string());
    }
    let dest = neural_shader_path();
    std::fs::write(&dest, body).map_err(|e| format!("Failed saving FSRCNNX shader: {e}"))?;
    Ok(dest)
}

/// Write GLSL shader files to the local cache directory and return colon/semicolon-separated paths.
pub fn prepare_video_shaders(mode: VideoShaderMode) -> Option<String> {
    if mode == VideoShaderMode::Off {
        return None;
    }

    let dir = shader_cache_dir();
    let _ = std::fs::create_dir_all(&dir);

    let deblock_path = dir.join("01_deblock_dering.glsl");
    let chroma_path = dir.join("02_chroma_guided.glsl");
    let superres_path = dir.join("03_superres_4k_rcas.glsl");
    let anime_path = dir.join("04_anime4k_restore.glsl");

    let _ = std::fs::write(&deblock_path, SHADER_DEBLOCK_DERING);
    let _ = std::fs::write(&chroma_path, SHADER_CHROMA_GUIDED);

    let mut list = vec![
        deblock_path.to_string_lossy().into_owned(),
        chroma_path.to_string_lossy().into_owned(),
    ];

    match mode {
        VideoShaderMode::Off => unreachable!(),
        VideoShaderMode::Balanced => {}
        VideoShaderMode::Reference4K => {
            let _ = std::fs::write(&superres_path, SHADER_SUPERRES_4K_RCAS);
            list.push(superres_path.to_string_lossy().into_owned());
        }
        VideoShaderMode::NeuralCinema => {
            let cnn = neural_shader_path();
            if cnn.exists() {
                list.push(cnn.to_string_lossy().into_owned());
            } else {
                let _ = std::fs::write(&superres_path, SHADER_SUPERRES_4K_RCAS);
                list.push(superres_path.to_string_lossy().into_owned());
            }
        }
        VideoShaderMode::Anime4K => {
            let _ = std::fs::write(&anime_path, SHADER_ANIME4K_RESTORE);
            list.push(anime_path.to_string_lossy().into_owned());
        }
    }

    #[cfg(windows)]
    let sep = ";";
    #[cfg(not(windows))]
    let sep = ":";

    Some(list.join(sep))
}

/// Generate adaptive mpv/libplacebo mastering flags complementing Reference 4K GLSL shaders.
pub fn mpv_mastering_flags(
    mode: VideoShaderMode,
    profile: VideoMasteringProfile,
) -> Vec<String> {
    if mode == VideoShaderMode::Off {
        return Vec::new();
    }

    let deband_iters = if mode == VideoShaderMode::Balanced
        && profile == VideoMasteringProfile::HighFrameRate
    {
        1
    } else {
        3
    };

    vec![
        "--scale=ewa_lanczossharp".to_string(),
        "--scale-antiring=0.7".to_string(),
        "--dscale=mitchell".to_string(),
        "--dscale-antiring=0.7".to_string(),
        "--cscale=ewa_lanczossharp".to_string(),
        "--cscale-antiring=0.7".to_string(),
        "--correct-downscaling=yes".to_string(),
        "--linear-downscaling=yes".to_string(),
        "--sigmoid-upscaling=yes".to_string(),
        "--deband=yes".to_string(),
        format!("--deband-iterations={deband_iters}"),
        "--deband-threshold=48".to_string(),
        "--deband-range=24".to_string(),
        "--deband-grain=16".to_string(),
        "--dither-depth=auto".to_string(),
        "--temporal-dither=yes".to_string(),
        "--tone-mapping=spline".to_string(),
        "--hdr-compute-peak=yes".to_string(),
        "--gamut-mapping-mode=clip".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_modes_and_flags() {
        assert!(prepare_video_shaders(VideoShaderMode::Off).is_none());

        let bal = prepare_video_shaders(VideoShaderMode::Balanced).expect("balanced shaders");
        assert!(bal.contains("01_deblock_dering.glsl"));
        assert!(bal.contains("02_chroma_guided.glsl"));
        assert!(!bal.contains("03_superres_4k_rcas.glsl"));

        let ref4k = prepare_video_shaders(VideoShaderMode::Reference4K).expect("ref4k shaders");
        assert!(ref4k.contains("01_deblock_dering.glsl"));
        assert!(ref4k.contains("02_chroma_guided.glsl"));
        assert!(ref4k.contains("03_superres_4k_rcas.glsl"));

        let neural = prepare_video_shaders(VideoShaderMode::NeuralCinema).expect("neural shaders");
        assert!(neural.contains("01_deblock_dering.glsl"));
        assert!(neural.contains("02_chroma_guided.glsl"));

        let anime = prepare_video_shaders(VideoShaderMode::Anime4K).expect("anime shaders");
        assert!(anime.contains("04_anime4k_restore.glsl"));

        assert!(SHADER_DEBLOCK_DERING.contains("//!HOOK LUMA"));
        assert!(SHADER_CHROMA_GUIDED.contains("//!HOOK CHROMA"));
        assert!(SHADER_SUPERRES_4K_RCAS.contains("//!WIDTH HOOKED.w 2 *"));
        assert!(SHADER_ANIME4K_RESTORE.contains("//!HOOK LUMA"));

        let flags =
            mpv_mastering_flags(VideoShaderMode::Reference4K, VideoMasteringProfile::StandardHd);
        assert!(flags.iter().any(|f| f == "--deband=yes"));
        assert!(flags.iter().any(|f| f == "--deband-iterations=3"));
        assert!(flags.iter().any(|f| f == "--tone-mapping=spline"));
        assert!(flags.iter().any(|f| f == "--scale-antiring=0.7"));
    }
}
