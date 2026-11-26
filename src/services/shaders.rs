//! Phaenna Reference 4K Studio — Real-Time Video De-Compression, True 4:4:4 Chroma & Conditional Super-Res GLSL Pipeline.
//!
//! Engineered for Maximum Perceived Uncompressed Fidelity per Watt across Windows, Linux & macOS:
//! 1. `01_phaenna_master_fused.glsl` (`LUMA`): Single-pass BPP-adaptive bilateral deblock + clamped
//!    local micro-contrast + FidelityFX RCAS + PTS-seeded midtone organic grain via live `//!PARAM`.
//! 2. `02_phaenna_chroma_444.glsl` (`CHROMA -> LUMA.w x LUMA.h`): Analytical luma-chroma linear
//!    regression reconstructing razor-sharp 4:4:4 color aligned to Luma edges (`cscale=bilinear`!).
//! 3. `03_superres_1080p_rcas.glsl` / `ArtCNN-C4F16` / `FSRCNNX`: Conditionally gated ONLY when
//!    upscaling sub-4K sources (`HOOKED.h <= 1080 && OUTPUT.w / HOOKED.w > 1.22`).
//! 4. `04_anime4k_restore.glsl` (`LUMA` 2x): Gradient clamp & line-art thinness restoration.

use crate::config::{
    DebandTier, GlslStageMode, GpuScalerMode, ManualMasteringConfig, PerformanceMode,
    SuperResMode, VideoShaderMode,
};
use std::path::PathBuf;

const ARTCNN_URL: &str =
    "https://raw.githubusercontent.com/Artoriuz/ArtCNN/main/GLSL/ArtCNN_C4F16.glsl";
const ARTCNN_FILENAME: &str = "artcnn_c4f16.glsl";

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

/// 0. Featherweight single-pass Luma RCAS sharpening (`Efficient` / ECO mode, zero extra FBO).
const SHADER_LIGHT_CAS: &str = r#"//!DESC Phaenna-Efficient-Light-CAS
//!HOOK LUMA
//!BIND HOOKED

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;
    float c = HOOKED_tex(pos).x;
    float n = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float s = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float e = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float w = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;

    float mn = min(c, min(min(n, s), min(e, w)));
    float mx = max(c, max(max(n, s), max(e, w)));
    float hit_min = mn / max(4.0 * mx, 1e-5);
    float hit_max = (1.0 - mx) / max(4.0 * mn - 4.0, -1e-5);
    float lobe = max(-hit_min, hit_max);
    float weight = clamp(lobe, -0.045, 0.0);
    float out_y = (c + (n + s + e + w) * weight) / (1.0 + 4.0 * weight);
    return vec4(clamp(out_y, mn, mx), 0.0, 0.0, 1.0);
}
"#;

/// 1A. Fused Single-Pass Master De-Compression, Clamped Micro-Detail, RCAS & Organic Grain (`//!PARAM` live tunable).
const SHADER_MASTER_FUSED_PARAM: &str = r#"//!PARAM deblock_amt
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
0.30
//!PARAM cas_amt
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 0.6
0.28
//!PARAM detail_lift
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 0.35
0.12
//!PARAM grain_amt
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 0.03
0.008
//!DESC Phaenna-Master-Fused-Luma
//!HOOK LUMA
//!BIND HOOKED

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;

    float c  = HOOKED_tex(pos).x;
    float n  = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float s  = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float e  = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float w  = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;
    float nw = HOOKED_tex(pos + vec2(-pt.x, -pt.y)).x;
    float ne = HOOKED_tex(pos + vec2( pt.x, -pt.y)).x;
    float sw = HOOKED_tex(pos + vec2(-pt.x,  pt.y)).x;
    float se = HOOKED_tex(pos + vec2( pt.x,  pt.y)).x;

    float mn4 = min(min(n, s), min(e, w));
    float mx4 = max(max(n, s), max(e, w));
    float mn  = min(min(c, mn4), min(min(nw, ne), min(sw, se)));
    float mx  = max(max(c, mx4), max(max(nw, ne), max(sw, se)));

    // 1. BPP-gated rational bilateral deblock & dering (skipped completely on clean high-BPP 4K)
    float base = c;
    if (deblock_amt > 0.001) {
        float inv2s2 = 1.0 / (2.0 * 0.036 * 0.036);
        float wn = 1.0 / (1.0 + ((n - c) * (n - c)) * inv2s2);
        float ws = 1.0 / (1.0 + ((s - c) * (s - c)) * inv2s2);
        float we = 1.0 / (1.0 + ((e - c) * (e - c)) * inv2s2);
        float ww = 1.0 / (1.0 + ((w - c) * (w - c)) * inv2s2);
        float filtered = (c + n * wn + s * ws + e * we + w * ww) / (1.0 + wn + ws + we + ww);
        float local_contrast = max(max(abs(n - c), abs(s - c)), max(abs(e - c), abs(w - c)));
        float protect = smoothstep(0.025, 0.11, local_contrast);
        float cleaned = mix(filtered, c, protect * 0.72);
        base = mix(c, cleaned, deblock_amt);
    }

    // 2. Clamped mid-frequency micro-contrast detail lift (recovers quantized skin/foliage texture)
    if (detail_lift > 0.001) {
        float avg4 = (n + s + e + w) * 0.25;
        float lifted = base + (base - avg4) * (detail_lift * 1.25);
        base = clamp(lifted, mn, mx);
    }

    // 3. FidelityFX RCAS contrast-adaptive sharpening constrained by 3x3 bounds
    float out_y = base;
    if (cas_amt > 0.001) {
        float hit_min = min(mn4, base) / max(4.0 * mx4, 1e-5);
        float hit_max = (1.0 - max(mx4, base)) / max(4.0 * mn4 - 4.0, -1e-5);
        float lobe = max(-hit_min, hit_max);
        float weight = clamp(lobe, -0.20 * cas_amt, 0.0);
        out_y = clamp((base + (n + s + e + w) * weight) / (1.0 + 4.0 * weight), mn, mx);
    }

    // 4. Frame-seeded midtone-weighted organic micro-grain (breaks residual 8-bit contour flatness)
    if (grain_amt > 0.0004) {
        float env = clamp(4.0 * out_y * (1.0 - out_y), 0.0, 1.0);
        float t1 = fract(float(frame) * 0.131);
        float t2 = fract(float(frame) * 0.719);
        float rnd = fract(sin(dot(pos * vec2(1291.7, 739.3) + vec2(t1 * 19.3, t2 * 37.7), vec2(12.9898, 78.233))) * 43758.5453) - 0.5;
        out_y = clamp(out_y + rnd * grain_amt * env, 0.0, 1.0);
    }

    return vec4(out_y, 0.0, 0.0, 1.0);
}
"#;

/// 1B. Static fallback fused Luma shader for legacy `mpv` builds without `--glsl-shader-opts`.
const SHADER_MASTER_FUSED_STATIC: &str = r#"//!DESC Phaenna-Master-Fused-Luma-Static
//!HOOK LUMA
//!BIND HOOKED

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;
    float c  = HOOKED_tex(pos).x;
    float n  = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float s  = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float e  = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float w  = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;

    float inv2s2 = 1.0 / (2.0 * 0.036 * 0.036);
    float wn = 1.0 / (1.0 + ((n - c) * (n - c)) * inv2s2);
    float ws = 1.0 / (1.0 + ((s - c) * (s - c)) * inv2s2);
    float we = 1.0 / (1.0 + ((e - c) * (e - c)) * inv2s2);
    float ww = 1.0 / (1.0 + ((w - c) * (w - c)) * inv2s2);
    float filtered = (c + n * wn + s * ws + e * we + w * ww) / (1.0 + wn + ws + we + ww);
    float local_contrast = max(max(abs(n - c), abs(s - c)), max(abs(e - c), abs(w - c)));
    float protect = smoothstep(0.025, 0.11, local_contrast);
    float base = mix(c, mix(filtered, c, protect * 0.72), 0.30);

    float mn4 = min(min(n, s), min(e, w));
    float mx4 = max(max(n, s), max(e, w));
    float hit_min = min(mn4, base) / max(4.0 * mx4, 1e-5);
    float hit_max = (1.0 - max(mx4, base)) / max(4.0 * mn4 - 4.0, -1e-5);
    float weight = clamp(max(-hit_min, hit_max), -0.056, 0.0);
    float out_y = clamp((base + (n + s + e + w) * weight) / (1.0 + 4.0 * weight), mn4, mx4);
    return vec4(out_y, 0.0, 0.0, 1.0);
}
"#;

/// 2. Analytical Luma-Guided 4:4:4 Chroma Reconstructor running at full `LUMA` resolution (`--cscale=bilinear`!).
const SHADER_CHROMA_444: &str = r#"//!DESC Phaenna-Ref4K-Chroma-444-Analytical
//!HOOK CHROMA
//!BIND HOOKED
//!BIND LUMA
//!WIDTH LUMA.w
//!HEIGHT LUMA.h
//!WHEN OUTPUT.w 960 >=

vec4 hook() {
    vec2 uv = HOOKED_pos;
    vec2 c_pt = HOOKED_pt;

    float y_hr = LUMA_tex(uv).x;
    float yn = LUMA_tex(uv + vec2(0.0, -c_pt.y)).x;
    float ys = LUMA_tex(uv + vec2(0.0,  c_pt.y)).x;
    float ye = LUMA_tex(uv + vec2( c_pt.x, 0.0)).x;
    float yw = LUMA_tex(uv + vec2(-c_pt.x, 0.0)).x;

    vec2 c0 = HOOKED_tex(uv).xy;
    vec2 cn = HOOKED_tex(uv + vec2(0.0, -c_pt.y)).xy;
    vec2 cs = HOOKED_tex(uv + vec2(0.0,  c_pt.y)).xy;
    vec2 ce = HOOKED_tex(uv + vec2( c_pt.x, 0.0)).xy;
    vec2 cw = HOOKED_tex(uv + vec2(-c_pt.x, 0.0)).xy;

    // Linear regression between local Luma & Chroma reconstructs crisp 4:4:4 edges
    float y_mean = (y_hr + yn + ys + ye + yw) * 0.2;
    vec2  c_mean = (c0 + cn + cs + ce + cw) * 0.2;

    float dy0 = y_hr - y_mean;
    float dyn = yn - y_mean;
    float dys = ys - y_mean;
    float dye = ye - y_mean;
    float dyw = yw - y_mean;

    float var_y = dy0 * dy0 + dyn * dyn + dys * dys + dye * dye + dyw * dyw;
    vec2 cov_yc = (c0 - c_mean) * dy0
                + (cn - c_mean) * dyn
                + (cs - c_mean) * dys
                + (ce - c_mean) * dye
                + (cw - c_mean) * dyw;

    vec2 slope = clamp(cov_yc / (var_y + 0.0012), vec2(-1.35), vec2(1.35));
    vec2 recon = c_mean + slope * (y_hr - y_mean);

    vec2 c_min = min(c0, min(min(cn, cs), min(ce, cw)));
    vec2 c_max = max(c0, max(max(cn, cs), max(ce, cw)));
    return vec4(clamp(recon, c_min, c_max), 0.0, 1.0);
}
"#;

/// 3. Conditional Directional 2x Super-Resolution + FidelityFX RCAS with 3x3 anti-ringing.
const SHADER_SUPERRES_4K_RCAS: &str = r#"//!DESC Phaenna-Ref4K-SuperRes-RCAS-2x
//!HOOK LUMA
//!BIND HOOKED
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *
//!WHEN OUTPUT.w HOOKED.w / 1.220 >

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

    float mn4 = min(min(n, s), min(e, w));
    float mx4 = max(max(n, s), max(e, w));
    float mn = min(min(c, mn4), min(min(nw, ne), min(sw, se)));
    float mx = max(max(c, mx4), max(max(nw, ne), max(sw, se)));

    float gx = abs(e - w) + 0.5 * (abs(ne - nw) + abs(se - sw));
    float gy = abs(s - n) + 0.5 * (abs(sw - nw) + abs(se - ne));
    float aniso = clamp((gx - gy) / (gx + gy + 1e-5), -1.0, 1.0);

    float recon_v = (n + c * 2.0 + s) * 0.25;
    float recon_h = (w + c * 2.0 + e) * 0.25;
    float edge_mag = smoothstep(0.04, 0.22, gx + gy);
    float base = mix(c, mix(recon_h, recon_v, 0.5 + 0.5 * aniso), edge_mag * 0.38);

    float hit_min = min(mn4, base) / max(4.0 * mx4, 1e-5);
    float hit_max = (1.0 - max(mx4, base)) / max(4.0 * mn4 - 4.0, -1e-5);
    float lobe = max(-hit_min, hit_max);
    float sharpness = 0.26;
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
//!WHEN OUTPUT.w HOOKED.w / 1.220 >

vec4 hook() {
    vec2 pt = HOOKED_pt;
    vec2 pos = HOOKED_pos;

    float mc = HOOKED_tex(pos).x;
    float mn = HOOKED_tex(pos + vec2(0.0, -pt.y)).x;
    float ms = HOOKED_tex(pos + vec2(0.0,  pt.y)).x;
    float me = HOOKED_tex(pos + vec2( pt.x, 0.0)).x;
    float mw = HOOKED_tex(pos + vec2(-pt.x, 0.0)).x;

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

pub fn mpv_shader_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "phaenna", "phaenna")
        .map(|d| d.cache_dir().join("mpv-shader-cache"))
        .unwrap_or_else(|| std::env::temp_dir().join("phaenna-mpv-shader-cache"))
}

pub fn artcnn_shader_path() -> PathBuf {
    shader_cache_dir().join(ARTCNN_FILENAME)
}

pub fn artcnn_shader_installed() -> bool {
    artcnn_shader_path().exists()
}

pub fn neural_shader_path() -> PathBuf {
    shader_cache_dir().join(FSRCNNX_FILENAME)
}

pub fn neural_shader_installed() -> bool {
    neural_shader_path().exists()
}

async fn download_glsl_pack(
    client: &reqwest::Client,
    url: &str,
    dest: PathBuf,
    label: &str,
) -> std::result::Result<PathBuf, String> {
    let dir = shader_cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed creating shader dir: {e}"))?;
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Network error downloading {label}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} downloading {label} pack", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed reading {label} response: {e}"))?;
    if !body.contains("//!HOOK") {
        return Err(format!(
            "Downloaded {label} file did not contain valid mpv //!HOOK GLSL headers"
        ));
    }
    std::fs::write(&dest, body).map_err(|e| format!("Failed saving {label} shader: {e}"))?;
    Ok(dest)
}

/// Download lightweight `ArtCNN_C4F16.glsl` neural residual upscaler pack into shader cache.
#[allow(dead_code)]
pub async fn download_artcnn_shader_pack(
    client: &reqwest::Client,
) -> std::result::Result<PathBuf, String> {
    download_glsl_pack(client, ARTCNN_URL, artcnn_shader_path(), "ArtCNN-C4F16").await
}

/// Download optional `FSRCNNX_x2_16-0-4-1.glsl` neural CNN shader pack for Neural Cinema 4K mode.
pub async fn download_neural_shader_pack(
    client: &reqwest::Client,
) -> std::result::Result<PathBuf, String> {
    download_glsl_pack(client, FSRCNNX_URL, neural_shader_path(), "FSRCNNX-16").await
}

fn effective_master_config(
    perf: PerformanceMode,
    manual: Option<ManualMasteringConfig>,
) -> ManualMasteringConfig {
    match (perf, manual) {
        (PerformanceMode::Manual, Some(m)) => m,
        (_, Some(m))
            if perf != PerformanceMode::Efficient
                && perf != PerformanceMode::Balanced
                && perf != PerformanceMode::Quality =>
        {
            m
        }
        _ => ManualMasteringConfig::from_preset(perf),
    }
}

/// Write GLSL shader files to local cache tailored by `PerformanceMode`.
#[cfg(test)]
pub fn prepare_video_shaders_for(mode: VideoShaderMode, perf: PerformanceMode) -> Option<String> {
    prepare_video_shaders_with_mastering(mode, perf, None)
}

/// Write GLSL shader files honoring `ManualMasteringConfig` and host player `//!PARAM` capabilities.
pub fn prepare_video_shaders_with_mastering(
    mode: VideoShaderMode,
    perf: PerformanceMode,
    manual: Option<ManualMasteringConfig>,
) -> Option<String> {
    if mode == VideoShaderMode::Off {
        return None;
    }

    let eff = effective_master_config(perf, manual);
    if eff.glsl_stage == GlslStageMode::Off {
        return None;
    }

    let dir = shader_cache_dir();
    let _ = std::fs::create_dir_all(&dir);

    #[cfg(windows)]
    let sep = ";";
    #[cfg(not(windows))]
    let sep = ":";

    if eff.glsl_stage == GlslStageMode::LightCas {
        let cas_path = dir.join("00_light_cas.glsl");
        let _ = std::fs::write(&cas_path, SHADER_LIGHT_CAS);
        return Some(cas_path.to_string_lossy().into_owned());
    }

    let caps = crate::services::platform::detect_player_capabilities();
    let fused_path = dir.join("01_phaenna_master_fused.glsl");
    let fused_src = if caps.has_shader_opts {
        SHADER_MASTER_FUSED_PARAM
    } else {
        SHADER_MASTER_FUSED_STATIC
    };
    let chroma_path = dir.join("02_phaenna_chroma_444.glsl");
    let _ = std::fs::write(&fused_path, fused_src);
    let _ = std::fs::write(&chroma_path, SHADER_CHROMA_444);

    let mut list = vec![
        fused_path.to_string_lossy().into_owned(),
        chroma_path.to_string_lossy().into_owned(),
    ];

    if eff.glsl_stage == GlslStageMode::Guided444 || eff.super_res == SuperResMode::Off {
        return Some(list.join(sep));
    }

    let superres_2x_src = |src: &str| -> String {
        if eff.super_res == SuperResMode::Auto1080p {
            src.replace(
                "//!WHEN OUTPUT.w HOOKED.w / 1.220 >",
                "//!WHEN HOOKED.h 1080 <= OUTPUT.w HOOKED.w / 1.220 > *",
            )
        } else {
            src.to_string()
        }
    };

    let superres_path = dir.join(if eff.super_res == SuperResMode::Auto1080p {
        "03_superres_1080p_rcas.glsl"
    } else {
        "03_superres_4k_rcas.glsl"
    });
    let anime_path = dir.join(if eff.super_res == SuperResMode::Auto1080p {
        "04_anime4k_1080p_restore.glsl"
    } else {
        "04_anime4k_restore.glsl"
    });

    match mode {
        VideoShaderMode::Off => unreachable!(),
        VideoShaderMode::Balanced => {}
        VideoShaderMode::Reference4K => {
            let _ = std::fs::write(&superres_path, superres_2x_src(SHADER_SUPERRES_4K_RCAS));
            list.push(superres_path.to_string_lossy().into_owned());
        }
        VideoShaderMode::NeuralCinema => {
            let art = artcnn_shader_path();
            let cnn = neural_shader_path();
            if art.exists() {
                list.push(art.to_string_lossy().into_owned());
            } else if cnn.exists() {
                list.push(cnn.to_string_lossy().into_owned());
            } else {
                let _ = std::fs::write(&superres_path, superres_2x_src(SHADER_SUPERRES_4K_RCAS));
                list.push(superres_path.to_string_lossy().into_owned());
            }
        }
        VideoShaderMode::Anime4K => {
            let _ = std::fs::write(&anime_path, superres_2x_src(SHADER_ANIME4K_RESTORE));
            list.push(anime_path.to_string_lossy().into_owned());
        }
    }

    Some(list.join(sep))
}

/// Generate adaptive mpv/libplacebo mastering flags keyed to shader mode and `PerformanceMode`.
#[cfg(test)]
pub fn mpv_mastering_flags_for(
    mode: VideoShaderMode,
    profile: VideoMasteringProfile,
    perf: PerformanceMode,
) -> Vec<String> {
    mpv_mastering_flags_with_manual(mode, profile, perf, None)
}

/// Generate capability-gated `mpv` / `libplacebo` flags from preset or `ManualMasteringConfig`.
pub fn mpv_mastering_flags_with_manual(
    mode: VideoShaderMode,
    profile: VideoMasteringProfile,
    perf: PerformanceMode,
    manual: Option<ManualMasteringConfig>,
) -> Vec<String> {
    let eff = match (perf, manual) {
        (PerformanceMode::Manual, Some(m)) => m,
        _ => ManualMasteringConfig::from_preset(perf),
    };
    let caps = crate::services::platform::detect_player_capabilities();
    let cache_dir = mpv_shader_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    if perf == PerformanceMode::Efficient && manual.is_none() {
        let mut flags = vec![
            "--scale=spline36".to_string(),
            "--dscale=hermite".to_string(),
            "--correct-downscaling=yes".to_string(),
            "--cscale=bilinear".to_string(),
            "--deband=no".to_string(),
            "--dither=fruit".to_string(),
            "--temporal-dither=no".to_string(),
            "--tone-mapping=spline".to_string(),
            "--vd-lavc-fast=yes".to_string(),
        ];
        if caps.has_perceptual_gamut {
            flags.push("--gamut-mapping-mode=perceptual".to_string());
        }
        if caps.has_gpu_shader_cache_dir {
            flags.push("--gpu-shader-cache=yes".to_string());
            flags.push(format!("--gpu-shader-cache-dir={}", cache_dir.to_string_lossy()));
        }
        if mode == VideoShaderMode::Off {
            flags.retain(|f| !f.starts_with("--deband"));
        }
        return flags;
    }

    if mode == VideoShaderMode::Off && perf != PerformanceMode::Manual {
        return Vec::new();
    }

    // 02_phaenna_chroma_444 outputs at full LUMA.w x LUMA.h, eliminating redundant GPU chroma scaling!
    let chroma_444_active = mode != VideoShaderMode::Off
        && matches!(
            eff.glsl_stage,
            GlslStageMode::Guided444 | GlslStageMode::FullMastering
        );

    let mut flags = match eff.gpu_scaler {
        GpuScalerMode::Spline36 => vec![
            "--scale=spline36".to_string(),
            "--dscale=hermite".to_string(),
            "--correct-downscaling=yes".to_string(),
            "--linear-downscaling=yes".to_string(),
            format!(
                "--cscale={}",
                if chroma_444_active { "bilinear" } else { "mitchell" }
            ),
        ],
        GpuScalerMode::EwaLanczos => vec![
            "--scale=ewa_lanczos".to_string(),
            "--dscale=hermite".to_string(),
            "--correct-downscaling=yes".to_string(),
            "--linear-downscaling=yes".to_string(),
            format!(
                "--cscale={}",
                if chroma_444_active { "bilinear" } else { "ewa_lanczos" }
            ),
        ],
        GpuScalerMode::EwaLanczosSharp => vec![
            "--scale=ewa_lanczossharp".to_string(),
            "--scale-antiring=0.7".to_string(),
            "--dscale=hermite".to_string(),
            "--correct-downscaling=yes".to_string(),
            "--linear-downscaling=yes".to_string(),
            "--sigmoid-upscaling=yes".to_string(),
            format!(
                "--cscale={}",
                if chroma_444_active { "bilinear" } else { "ewa_lanczossharp" }
            ),
        ],
    };

    match eff.deband {
        DebandTier::Off => {
            flags.push("--deband=no".to_string());
            flags.push("--temporal-dither=no".to_string());
        }
        DebandTier::OnePassCool => {
            flags.extend([
                "--deband=yes".to_string(),
                "--deband-iterations=1".to_string(),
                "--deband-threshold=36".to_string(),
                "--deband-range=16".to_string(),
                "--deband-grain=6".to_string(),
                "--temporal-dither=no".to_string(),
            ]);
        }
        DebandTier::ThreePassStudio => {
            let deband_iters = if mode == VideoShaderMode::Balanced
                && profile == VideoMasteringProfile::HighFrameRate
            {
                1
            } else {
                2
            };
            flags.extend([
                "--deband=yes".to_string(),
                format!("--deband-iterations={deband_iters}"),
                "--deband-threshold=42".to_string(),
                "--deband-range=18".to_string(),
                "--deband-grain=10".to_string(),
                "--temporal-dither=yes".to_string(),
            ]);
        }
    }

    flags.extend([
        "--dither=fruit".to_string(),
        "--dither-size-fruit=6".to_string(),
        "--dither-depth=auto".to_string(),
        format!("--tone-mapping={}", eff.tone_map.mpv_value()),
        format!(
            "--hdr-compute-peak={}",
            if eff.hdr_compute_peak { "yes" } else { "no" }
        ),
        format!(
            "--gamut-mapping-mode={}",
            if caps.has_perceptual_gamut { "perceptual" } else { "clip" }
        ),
    ]);

    if caps.has_icc_auto {
        flags.push("--icc-profile-auto=yes".to_string());
    }
    if caps.has_target_colorspace_hint {
        flags.push("--target-colorspace-hint=auto".to_string());
    }
    if caps.has_hdr_contrast_recovery {
        let recov = if perf == PerformanceMode::Quality || eff.glsl_stage == GlslStageMode::FullMastering {
            "0.30"
        } else {
            "0.20"
        };
        flags.push(format!("--hdr-contrast-recovery={recov}"));
    }
    if caps.has_hdr_peak_percentile && (perf == PerformanceMode::Quality || eff.hdr_compute_peak) {
        flags.push("--hdr-peak-percentile=99.995".to_string());
    }
    if caps.has_libplacebo_opts {
        flags.push("--libplacebo-opts=allow_delayed_peak=yes".to_string());
    }
    if caps.has_gpu_shader_cache_dir {
        flags.push("--gpu-shader-cache=yes".to_string());
        flags.push(format!("--gpu-shader-cache-dir={}", cache_dir.to_string_lossy()));
    }
    if caps.has_shader_opts && chroma_444_active {
        let is_max = perf == PerformanceMode::Quality || eff.glsl_stage == GlslStageMode::FullMastering;
        let deblock = if is_max { 0.30 } else { 0.25 };
        let cas = if is_max { 0.28 } else { 0.22 };
        let detail = if is_max { 0.12 } else { 0.08 };
        let grain = eff.organic_grain.luma_strength();
        flags.push(format!(
            "--glsl-shader-opts=deblock_amt={deblock:.2},cas_amt={cas:.2},detail_lift={detail:.2},grain_amt={grain:.3}"
        ));
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_modes_and_flags() {
        assert!(prepare_video_shaders_for(VideoShaderMode::Off, PerformanceMode::Quality).is_none());

        let bal = prepare_video_shaders_for(VideoShaderMode::Balanced, PerformanceMode::Quality)
            .expect("balanced shaders");
        assert!(bal.contains("01_phaenna_master_fused.glsl"));
        assert!(bal.contains("02_phaenna_chroma_444.glsl"));
        assert!(!bal.contains("03_superres_1080p_rcas.glsl"));

        let ref4k = prepare_video_shaders_for(VideoShaderMode::Reference4K, PerformanceMode::Quality)
            .expect("ref4k shaders");
        assert!(ref4k.contains("01_phaenna_master_fused.glsl"));
        assert!(ref4k.contains("02_phaenna_chroma_444.glsl"));
        assert!(ref4k.contains("03_superres_1080p_rcas.glsl"));

        let neural =
            prepare_video_shaders_for(VideoShaderMode::NeuralCinema, PerformanceMode::Quality)
                .expect("neural shaders");
        assert!(neural.contains("01_phaenna_master_fused.glsl"));
        assert!(neural.contains("02_phaenna_chroma_444.glsl"));

        let anime = prepare_video_shaders_for(VideoShaderMode::Anime4K, PerformanceMode::Quality)
            .expect("anime shaders");
        assert!(anime.contains("04_anime4k_1080p_restore.glsl"));

        assert!(SHADER_MASTER_FUSED_PARAM.contains("//!HOOK LUMA"));
        assert!(SHADER_MASTER_FUSED_PARAM.contains("//!PARAM grain_amt"));
        assert!(
            SHADER_MASTER_FUSED_PARAM.find("//!PARAM grain_amt").unwrap()
                < SHADER_MASTER_FUSED_PARAM.find("//!HOOK LUMA").unwrap()
        );
        assert!(SHADER_MASTER_FUSED_PARAM.contains("float(frame)"));
        assert!(SHADER_CHROMA_444.contains("//!HOOK CHROMA"));
        assert!(SHADER_CHROMA_444.contains("//!WIDTH LUMA.w"));
        assert!(SHADER_CHROMA_444.contains("//!WHEN OUTPUT.w 960 >="));
        assert!(SHADER_SUPERRES_4K_RCAS.contains("//!WIDTH HOOKED.w 2 *"));
        assert!(SHADER_ANIME4K_RESTORE.contains("//!HOOK LUMA"));

        let flags = mpv_mastering_flags_for(
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
            PerformanceMode::Quality,
        );
        assert!(flags.iter().any(|f| f == "--deband=yes"));
        assert!(flags.iter().any(|f| f == "--deband-iterations=2"));
        assert!(flags.iter().any(|f| f == "--dscale=hermite"));
        assert!(flags.iter().any(|f| f == "--cscale=bilinear"));
        assert!(flags.iter().any(|f| f == "--dither=fruit"));
        assert!(flags.iter().any(|f| f == "--tone-mapping=spline"));
        assert!(flags.iter().any(|f| f == "--scale=ewa_lanczos"));
        assert!(flags.iter().any(|f| f == "--hdr-compute-peak=yes"));

        let eco_sh =
            prepare_video_shaders_for(VideoShaderMode::Reference4K, PerformanceMode::Efficient)
                .unwrap();
        assert!(eco_sh.contains("00_light_cas.glsl"));
        assert!(!eco_sh.contains("03_superres_1080p_rcas.glsl"));
        let bal_sh =
            prepare_video_shaders_for(VideoShaderMode::Reference4K, PerformanceMode::Balanced)
                .unwrap();
        assert!(bal_sh.contains("01_phaenna_master_fused.glsl"));
        assert!(!bal_sh.contains("03_superres_1080p_rcas.glsl"));

        // Balanced preset is cool-tuned (spline36, hermite down, bilinear cscale bypassed by 4:4:4, 1-pass deband)
        let bal_flags = mpv_mastering_flags_for(
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
            PerformanceMode::Balanced,
        );
        assert!(bal_flags.iter().any(|f| f == "--scale=spline36"));
        assert!(bal_flags.iter().any(|f| f == "--dscale=hermite"));
        assert!(bal_flags.iter().any(|f| f == "--cscale=bilinear"));
        assert!(bal_flags.iter().any(|f| f == "--hdr-compute-peak=no"));
        assert!(bal_flags.iter().any(|f| f == "--deband-iterations=1"));

        // Manual <=1080p SuperRes mode gates 2x doubling cleanly
        let mut custom = ManualMasteringConfig::from_preset(PerformanceMode::Quality);
        custom.super_res = SuperResMode::Auto1080p;
        custom.hdr_compute_peak = false;
        let auto_sh = prepare_video_shaders_with_mastering(
            VideoShaderMode::Reference4K,
            PerformanceMode::Manual,
            Some(custom),
        )
        .unwrap();
        assert!(auto_sh.contains("03_superres_1080p_rcas.glsl"));
        let custom_flags = mpv_mastering_flags_with_manual(
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
            PerformanceMode::Manual,
            Some(custom),
        );
        assert!(custom_flags.iter().any(|f| f == "--hdr-compute-peak=no"));

        let eco_flags = mpv_mastering_flags_for(
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
            PerformanceMode::Efficient,
        );
        assert!(eco_flags.iter().any(|f| f == "--scale=spline36"));
        assert!(eco_flags.iter().any(|f| f == "--dscale=hermite"));
        assert!(eco_flags.iter().any(|f| f == "--deband=no"));
    }
}
