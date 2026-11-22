//! Page 3 — `[3] Mastering & GPU`: Live Signal-Flow Pipeline Visualizer,
//! Engine Mode, Content Profiles, GPU / RAM / Thermal Impact Estimator,
//! Active GLSL Chain Manifest, and 10-Point Manual Matrix with Wrapped Inspector.

use crate::config::{
    CodecPriority, Config, DebandTier, DemuxerCachePreset, GlslStageMode, GpuScalerMode,
    HwdecMode, ManualMasteringConfig, MotionMode, PerformanceMode, SuperResMode, VideoShaderMode,
};
use crate::services::shaders::{
    mpv_mastering_flags_with_manual, neural_shader_installed, VideoMasteringProfile,
};
use crate::state::{AppState, ContentProfile, ManualToggle, PlayOptionsHit};
use crate::ui::studio::widgets::{
    mode_badge_color, push_line_hit, push_pane_hit, render_inline_pills, section_block,
    segmented_gauge_line, studio_kv_row, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioImpactEstimate {
    pub gpu_pct: u8,
    pub ram_mb: u32,
    pub thermal_pct: u8,
}

/// Deterministic GPU / Unified RAM / Thermal load estimator from active mastering config.
pub fn estimate_mastering_impact(cfg: &Config) -> StudioImpactEstimate {
    let eff = cfg.effective_mastering();
    let shader_active = cfg.shader_mode != VideoShaderMode::Off;

    let mut gpu: u16 = 12;
    if shader_active {
        gpu += match eff.glsl_stage {
            GlslStageMode::Off => 0,
            GlslStageMode::LightCas => 6,
            GlslStageMode::Guided444 => 16,
            GlslStageMode::FullMastering => 26,
        };
        if eff.glsl_stage == GlslStageMode::FullMastering
            && !matches!(cfg.shader_mode, VideoShaderMode::Off | VideoShaderMode::Balanced)
        {
            gpu += match eff.super_res {
                SuperResMode::Off => 0,
                SuperResMode::Auto1080p => 18,
                SuperResMode::Always2x => 30,
            };
            if cfg.shader_mode == VideoShaderMode::NeuralCinema
                && eff.super_res != SuperResMode::Off
            {
                gpu += 10;
            }
        }
    }
    gpu += match eff.gpu_scaler {
        GpuScalerMode::Spline36 => 2,
        GpuScalerMode::EwaLanczos => 10,
        GpuScalerMode::EwaLanczosSharp => 16,
    };
    gpu += match eff.deband {
        DebandTier::Off => 0,
        DebandTier::OnePassCool => 6,
        DebandTier::ThreePassStudio => 14,
    };
    gpu += match cfg.motion_mode {
        MotionMode::Off => 0,
        MotionMode::Cadence => 6,
        MotionMode::Smooth120Hz => 16,
    };
    let gpu_pct = gpu.min(100) as u8;

    let mut ram_mb: u32 = 220;
    ram_mb += match eff.demuxer_cache {
        DemuxerCachePreset::Small48M => 64,
        DemuxerCachePreset::Medium160M => 208,
        DemuxerCachePreset::Large512M => 640,
    };
    if shader_active
        && eff.glsl_stage == GlslStageMode::FullMastering
        && eff.super_res == SuperResMode::Always2x
    {
        ram_mb += 280;
    }
    if cfg.audio_multistreams && cfg.performance_mode != PerformanceMode::Efficient {
        ram_mb += 90;
    }

    let mut therm: u16 = ((gpu_pct as u16) * 65) / 100;
    therm += match eff.codec_priority {
        CodecPriority::Vp9HwFirst => 4,
        CodecPriority::Av1First => 12,
    };
    if eff.hwdec == HwdecMode::Off {
        therm += 24;
    }
    if eff.hdr_compute_peak {
        therm += 8;
    }
    let thermal_pct = therm.min(100) as u8;

    StudioImpactEstimate {
        gpu_pct,
        ram_mb,
        thermal_pct,
    }
}

pub fn render_mastering_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 34 || area.height < 12 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(10)])
        .split(area);

    render_signal_flow_pipeline(frame, rows[0], state);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(rows[1]);

    render_mode_shaders_pane(frame, cols[0], state);
    render_manual_matrix_pane(frame, cols[1], state);
}

fn render_signal_flow_pipeline(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(
        " Active GPU Signal-Flow Pipeline (vo=gpu-next · libplacebo) ",
        false,
        &theme,
    );
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 30 || inner.height < 2 {
        return;
    }

    let eff = state.config.effective_mastering();
    let shader = state.config.shader_mode;
    let shader_on = shader != VideoShaderMode::Off && eff.glsl_stage != GlslStageMode::Off;
    let deblock_on = shader_on && eff.glsl_stage != GlslStageMode::LightCas;
    let chroma_on = deblock_on;
    let sr_on = shader_on
        && eff.glsl_stage == GlslStageMode::FullMastering
        && eff.super_res != SuperResMode::Off
        && !matches!(shader, VideoShaderMode::Balanced);
    let deband_on = eff.deband != DebandTier::Off;
    let motion_on = state.config.motion_mode != MotionMode::Off;

    let stage_style = |on: bool, col: Color| {
        if on {
            Style::default().fg(col).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.divider)
        }
    };

    let dec_tag = match eff.hwdec {
        HwdecMode::VideoToolbox => "HW VT",
        HwdecMode::AutoSafe => "HW Auto",
        HwdecMode::Off => "CPU SW",
    };
    let sr_tag = if sr_on {
        match shader {
            VideoShaderMode::NeuralCinema => "2x FSRCNNX",
            VideoShaderMode::Anime4K => "2x Anime4K",
            _ => "2x RCAS",
        }
    } else if shader_on && eff.glsl_stage == GlslStageMode::LightCas {
        "1x Light CAS"
    } else {
        "Bypass 1x"
    };

    let line1 = Line::from(vec![
        Span::styled(
            format!(" [{dec_tag}]"),
            stage_style(true, Color::Rgb(52, 211, 153)),
        ),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled("[Deblock]", stage_style(deblock_on, theme.primary)),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled("[4:4:4 Chroma]", stage_style(chroma_on, theme.primary)),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("[{sr_tag}]"),
            stage_style(
                sr_on || (shader_on && eff.glsl_stage == GlslStageMode::LightCas),
                theme.accent,
            ),
        ),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled("[Deband]", stage_style(deband_on, theme.secondary)),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("[{}]", eff.tone_map.mpv_value()),
            stage_style(true, Color::Rgb(56, 189, 248)),
        ),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("[{}]", state.config.motion_mode.short_badge()),
            stage_style(motion_on, Color::Rgb(192, 132, 252)),
        ),
    ]);

    let line2 = Line::from(Span::styled(
        format!(
            " {}",
            truncate_words(
                &format!(
                    "Scaler: {} | Cache: {} | HDR Peak: {} | Codec Prio: {}",
                    eff.gpu_scaler.label(),
                    eff.demuxer_cache.label(),
                    if eff.hdr_compute_peak {
                        "Dynamic"
                    } else {
                        "Cool Static"
                    },
                    eff.codec_priority.label()
                ),
                inner.width.saturating_sub(1) as usize,
            )
        ),
        Style::default().fg(theme.text_dim),
    ));

    frame.render_widget(Paragraph::new(vec![line1, line2]), inner);
}

fn active_glsl_manifest(shader: VideoShaderMode, eff: ManualMasteringConfig) -> (String, usize) {
    if shader == VideoShaderMode::Off || eff.glsl_stage == GlslStageMode::Off {
        return ("Bypass (raw native decoder output)".to_string(), 0);
    }
    if eff.glsl_stage == GlslStageMode::LightCas {
        return ("00_light_cas.glsl".to_string(), 1);
    }
    let mut chain = vec!["01_deblock_dering.glsl", "02_chroma_guided.glsl"];
    if eff.glsl_stage == GlslStageMode::FullMastering && eff.super_res != SuperResMode::Off {
        match shader {
            VideoShaderMode::Reference4K => {
                chain.push(if eff.super_res == SuperResMode::Auto1080p {
                    "03_superres_1080p_rcas.glsl"
                } else {
                    "03_superres_4k_rcas.glsl"
                });
            }
            VideoShaderMode::NeuralCinema => {
                if neural_shader_installed() {
                    chain.push("fsrcnnx_x2_16_0_4_1.glsl");
                } else {
                    chain.push("03_superres_4k_rcas.glsl");
                }
            }
            VideoShaderMode::Anime4K => {
                chain.push(if eff.super_res == SuperResMode::Auto1080p {
                    "04_anime4k_1080p_restore.glsl"
                } else {
                    "04_anime4k_restore.glsl"
                });
            }
            VideoShaderMode::Balanced | VideoShaderMode::Off => {}
        }
    }
    let count = chain.len();
    (chain.join(" -> "), count)
}

fn active_content_profile(cfg: &Config) -> Option<ContentProfile> {
    let eff = cfg.effective_mastering();
    if cfg.shader_mode == VideoShaderMode::Off && cfg.performance_mode == PerformanceMode::Efficient
    {
        return Some(ContentProfile::CoolBattery);
    }
    if cfg.shader_mode == VideoShaderMode::Reference4K
        && cfg.performance_mode == PerformanceMode::Quality
        && cfg.motion_mode == MotionMode::Cadence
    {
        return Some(ContentProfile::Cinema24p);
    }
    if cfg.shader_mode == VideoShaderMode::Balanced
        && cfg.motion_mode == MotionMode::Smooth120Hz
        && eff.codec_priority == CodecPriority::Vp9HwFirst
    {
        return Some(ContentProfile::Sports60p);
    }
    if cfg.shader_mode == VideoShaderMode::Anime4K {
        return Some(ContentProfile::Anime2x);
    }
    if cfg.performance_mode == PerformanceMode::Manual
        && eff.glsl_stage == GlslStageMode::LightCas
        && eff.deband == DebandTier::Off
    {
        return Some(ContentProfile::CrispCode);
    }
    None
}

fn render_mode_shaders_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Engine Mode, Profiles & 4K Shaders ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 11 {
        return;
    }

    let impact = estimate_mastering_impact(&state.config);
    let eff = state.config.effective_mastering();
    let active_prof = active_content_profile(&state.config);
    let perf_now = state.config.performance_mode;
    let shader_now = state.config.shader_mode;
    let cnn_installed = neural_shader_installed();
    let (chain_str, pass_count) = active_glsl_manifest(shader_now, eff);

    let show_profiles = inner.height >= 16;
    let show_manifest = inner.height >= 19;

    let mut constraints = vec![
        Constraint::Length(1), // 0: Mode pills
        Constraint::Length(1), // 1: Seed pills
    ];
    if show_profiles {
        constraints.push(Constraint::Length(1)); // Profile row 1
        constraints.push(Constraint::Length(1)); // Profile row 2
    }
    constraints.extend([
        Constraint::Length(1), // Impact header
        Constraint::Length(1), // GPU Load
        Constraint::Length(1), // Est RAM
        Constraint::Length(1), // Thermal
        Constraint::Length(1), // Shaders header
        Constraint::Length(5), // 5 Shaders
        Constraint::Length(1), // CNN pack
    ]);
    if show_manifest {
        constraints.push(Constraint::Min(2)); // Active GLSL chain + Thermal verdict
    } else {
        constraints.push(Constraint::Min(0));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ri = 0usize;

    // Mode pills via render_inline_pills
    let mode_pills: Vec<(String, Style, PlayOptionsHit)> = PerformanceMode::ALL
        .iter()
        .map(|pm| {
            let active = *pm == perf_now;
            let col = mode_badge_color(*pm);
            let lbl = if inner.width >= 50 {
                format!("[{}]", pm.label())
            } else {
                format!("[{}]", pm.short_badge())
            };
            (
                lbl,
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(col)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
                PlayOptionsHit::SetPerformanceMode(*pm),
            )
        })
        .collect();
    render_inline_pills(
        frame,
        rows[ri],
        Some((
            " Mode[m]:",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        &mode_pills,
        hits,
    );
    ri += 1;

    // Seed Man pills via render_inline_pills
    let seed_pills = vec![
        (
            "[Seed ECO]".to_string(),
            Style::default().fg(Color::Rgb(52, 211, 153)),
            PlayOptionsHit::SeedManualFrom(PerformanceMode::Efficient),
        ),
        (
            "[Seed BAL]".to_string(),
            Style::default().fg(Color::Rgb(56, 189, 248)),
            PlayOptionsHit::SeedManualFrom(PerformanceMode::Balanced),
        ),
        (
            "[Seed MAX]".to_string(),
            Style::default().fg(Color::Rgb(251, 191, 36)),
            PlayOptionsHit::SeedManualFrom(PerformanceMode::Quality),
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Seed Man:", Style::default().fg(theme.text_dim))),
        &seed_pills,
        hits,
    );
    ri += 1;

    if show_profiles {
        let prof_r1 = [
            ContentProfile::Cinema24p,
            ContentProfile::Sports60p,
            ContentProfile::Anime2x,
        ]
        .into_iter()
        .map(|cp| {
            let active = active_prof == Some(cp);
            (
                format!("[{}]", cp.short_label()),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.secondary)
                },
                PlayOptionsHit::ApplyContentProfile(cp),
            )
        })
        .collect::<Vec<_>>();
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Profiles:", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
            &prof_r1,
            hits,
        );
        ri += 1;

        let prof_r2 = [ContentProfile::CrispCode, ContentProfile::CoolBattery]
            .into_iter()
            .map(|cp| {
                let active = active_prof == Some(cp);
                (
                    format!("[{}]", cp.short_label()),
                    if active {
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme.accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                    PlayOptionsHit::ApplyContentProfile(cp),
                )
            })
            .collect::<Vec<_>>();
        render_inline_pills(
            frame,
            rows[ri],
            Some(("          ", Style::default().fg(theme.text_dim))),
            &prof_r2,
            hits,
        );
        ri += 1;
    }

    // Impact Gauges
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Resource & Thermal Budget (Estimated):",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    let ram_pct = ((impact.ram_mb * 100) / 1600).min(100) as u8;
    let w = rows[ri].width as usize;
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "GPU Load",
            impact.gpu_pct,
            &format!("{:>3}%", impact.gpu_pct),
            w,
            &theme,
        )),
        rows[ri],
    );
    ri += 1;

    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Est RAM",
            ram_pct,
            &format!("{}M", impact.ram_mb),
            w,
            &theme,
        )),
        rows[ri],
    );
    ri += 1;

    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Thermal",
            impact.thermal_pct,
            &format!("{:>3}%", impact.thermal_pct),
            w,
            &theme,
        )),
        rows[ri],
    );
    ri += 1;

    frame.render_widget(
        Paragraph::new(Span::styled(
            " 4K Mastering Shader [s] (Hot-Swap Live):",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    let sh_area = rows[ri];
    ri += 1;
    let all_shaders = [
        (VideoShaderMode::Reference4K, "Reference 4K", "Deblock+4:4:4+RCAS"),
        (VideoShaderMode::NeuralCinema, "Neural Cinema", "FSRCNNX CNN+4:4:4"),
        (VideoShaderMode::Anime4K, "Anime4K", "Line restore 2x"),
        (VideoShaderMode::Balanced, "Balanced", "Deblock+CAS+Deband"),
        (VideoShaderMode::Off, "Off", "Raw native stream"),
    ];
    let mut shader_lines = Vec::new();
    for (m, name, desc) in all_shaders {
        let active = m == shader_now;
        let row = shader_lines.len() as u16;
        let name_w = 14usize;
        let desc_budget = (sh_area.width as usize).saturating_sub(name_w + 6);
        let d_fit = truncate_words(desc, desc_budget);
        shader_lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {name:<name_w$}", if active { ">" } else { " " }),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            ),
            Span::styled(" │ ", Style::default().fg(theme.divider)),
            Span::styled(d_fit, Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, sh_area, row, PlayOptionsHit::SelectShader(m));
    }
    frame.render_widget(Paragraph::new(shader_lines), sh_area);

    // CNN Pack status row
    let cnn_area = rows[ri];
    ri += 1;
    frame.render_widget(
        Paragraph::new(studio_kv_row(
            "CNN Pack",
            if cnn_installed {
                "FSRCNNX Installed (Ready)"
            } else {
                "[Click to Download CNN]"
            },
            None,
            10,
            false,
            Style::default().fg(if cnn_installed {
                Color::Rgb(52, 211, 153)
            } else {
                theme.accent
            }),
            cnn_area.width,
            &theme,
        )),
        cnn_area,
    );
    if !cnn_installed {
        push_line_hit(hits, cnn_area, 0, PlayOptionsHit::DownloadNeuralShaders);
    }

fn pad_h(r: Rect, px: u16) -> Rect {
    Rect::new(
        r.x.saturating_add(px),
        r.y,
        r.width.saturating_sub(px * 2),
        r.height,
    )
}

    if show_manifest && ri < rows.len() && rows[ri].height >= 2 {
        let (verdict_txt, v_col) = if impact.thermal_pct <= 35 {
            ("Ultra-Cool Fanless / Peak Battery", Color::Rgb(52, 211, 153))
        } else if impact.thermal_pct <= 60 {
            ("Balanced Warm / Quiet Chassis", Color::Rgb(56, 189, 248))
        } else {
            ("High GPU Load / AC Adapter Rec", Color::Rgb(251, 191, 36))
        };
        let mut manifest_lines = vec![
            Line::from(Span::styled(
                "─".repeat(pad_h(rows[ri], 1).width as usize),
                Style::default().fg(theme.divider),
            )),
            Line::from(vec![
                Span::styled(
                    format!("Chain ({pass_count} pass): "),
                    Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                ),
                Span::styled(chain_str, Style::default().fg(theme.text)),
            ]),
            Line::from(vec![
                Span::styled("Thermal Verdict: ", Style::default().fg(theme.text_dim)),
                Span::styled(verdict_txt, Style::default().fg(v_col).add_modifier(Modifier::BOLD)),
            ]),
        ];
        if rows[ri].height >= 5 {
            manifest_lines.push(Line::from(Span::styled(
                "Keys: [m] Mode [s] Shader [t] 120Hz [Enter] Apply",
                Style::default().fg(theme.text_dim),
            )));
        }
        frame.render_widget(
            Paragraph::new(manifest_lines).wrap(Wrap { trim: true }),
            pad_h(rows[ri], 1),
        );
    }

    push_pane_hit(hits, area, 0);
}

fn manual_inspector_meta(toggle: ManualToggle) -> (&'static str, &'static str, &'static str, bool) {
    match toggle {
        ManualToggle::SuperRes => (
            "5K 2x Upscale",
            "glsl-shaders",
            "2x Super-Res allocates 5K/8K 16-bit GPU FBOs. Use <=1080p for cool 4K streams.",
            true,
        ),
        ManualToggle::CodecPriority => (
            "Codec Prio",
            "ytdl-raw-options",
            "VP9/HEVC decodes cooler in hardware; AV1 saves bandwidth. Press [Enter] to reload codec.",
            false,
        ),
        ManualToggle::Hwdec => (
            "Hardware Dec",
            "hwdec",
            "VideoToolbox/Auto-Safe offloads decode to media silicon; CPU SW dramatically raises heat.",
            true,
        ),
        ManualToggle::HdrComputePeak => (
            "HDR Peak Comp",
            "hdr-compute-peak",
            "Dynamic HDR peak measures per-scene brightness; Cool Static avoids compute passes.",
            true,
        ),
        ManualToggle::ToneMap => (
            "HDR Tone Map",
            "tone-mapping",
            "libplacebo HDR-to-SDR curve: Spline (balanced), BT.2446a (broadcast), ST2094-40 (dynamic).",
            true,
        ),
        ManualToggle::GlslStage => (
            "4:4:4 & Deblk",
            "glsl-shaders",
            "Controls active GLSL depth: Light CAS -> Guided 4:4:4 chroma -> Full 4K 2x pipeline.",
            true,
        ),
        ManualToggle::Deband => (
            "Deband Filter",
            "deband",
            "16-bit gradient debander removes sky/shadow banding; 3-Pass adds temporal dithering.",
            true,
        ),
        ManualToggle::GpuScaler => (
            "GPU Scaler",
            "scale / cscale",
            "EWA LanczosSharp adds antiring & linear/sigmoid scaling; Spline36 stays ultra cool.",
            true,
        ),
        ManualToggle::Motion => (
            "120Hz Motion",
            "tscale / video-sync",
            "120Hz display-resample cadence or Catmull-Rom interpolation (hot-swaps live via [t]).",
            true,
        ),
        ManualToggle::DemuxerCache => (
            "Demuxer Cache",
            "demuxer-max-bytes",
            "Forward/back RAM ring budget (48M / 160M / 512M). Higher reduces Wi-Fi micro-stalls.",
            true,
        ),
    }
}

fn heat_dots(toggle: ManualToggle, cfg: &Config) -> (&'static str, Color) {
    let eff = cfg.effective_mastering();
    let level = match toggle {
        ManualToggle::SuperRes => match eff.super_res {
            SuperResMode::Off => 0,
            SuperResMode::Auto1080p => 2,
            SuperResMode::Always2x => 3,
        },
        ManualToggle::CodecPriority => match eff.codec_priority {
            CodecPriority::Vp9HwFirst => 1,
            CodecPriority::Av1First => 2,
        },
        ManualToggle::Hwdec => match eff.hwdec {
            HwdecMode::VideoToolbox | HwdecMode::AutoSafe => 1,
            HwdecMode::Off => 3,
        },
        ManualToggle::HdrComputePeak => {
            if eff.hdr_compute_peak {
                2
            } else {
                0
            }
        }
        ManualToggle::ToneMap => 1,
        ManualToggle::GlslStage => match eff.glsl_stage {
            GlslStageMode::Off => 0,
            GlslStageMode::LightCas => 1,
            GlslStageMode::Guided444 => 2,
            GlslStageMode::FullMastering => 3,
        },
        ManualToggle::Deband => match eff.deband {
            DebandTier::Off => 0,
            DebandTier::OnePassCool => 1,
            DebandTier::ThreePassStudio => 2,
        },
        ManualToggle::GpuScaler => match eff.gpu_scaler {
            GpuScalerMode::Spline36 => 0,
            GpuScalerMode::EwaLanczos => 1,
            GpuScalerMode::EwaLanczosSharp => 2,
        },
        ManualToggle::Motion => match cfg.motion_mode {
            MotionMode::Off => 0,
            MotionMode::Cadence => 1,
            MotionMode::Smooth120Hz => 3,
        },
        ManualToggle::DemuxerCache => match eff.demuxer_cache {
            DemuxerCachePreset::Small48M => 1,
            DemuxerCachePreset::Medium160M => 2,
            DemuxerCachePreset::Large512M => 3,
        },
    };
    match level {
        0 => ("○○○", Color::Rgb(52, 211, 153)),
        1 => ("●○○", Color::Rgb(52, 211, 153)),
        2 => ("●●○", Color::Rgb(251, 191, 36)),
        _ => ("●●●", Color::Rgb(248, 113, 113)),
    }
}

fn delta_vs_balanced_badges(cfg: &Config) -> Vec<&'static str> {
    let eff = cfg.effective_mastering();
    let bal = ManualMasteringConfig::from_preset(PerformanceMode::Balanced);
    let mut diffs = Vec::new();
    if eff.super_res != bal.super_res {
        diffs.push(match eff.super_res {
            SuperResMode::Off => "Up:Off",
            SuperResMode::Auto1080p => "+1080p2x",
            SuperResMode::Always2x => "+Always2x",
        });
    }
    if eff.codec_priority != bal.codec_priority {
        diffs.push("+AV1");
    }
    if eff.hwdec != bal.hwdec {
        diffs.push("Dec:Mod");
    }
    if eff.hdr_compute_peak != bal.hdr_compute_peak {
        diffs.push("+HDRDyn");
    }
    if eff.tone_map != bal.tone_map {
        diffs.push("Tone:Mod");
    }
    if eff.glsl_stage != bal.glsl_stage {
        diffs.push(match eff.glsl_stage {
            GlslStageMode::Off => "GLSL:Off",
            GlslStageMode::LightCas => "GLSL:CAS",
            GlslStageMode::Guided444 => "GLSL:444",
            GlslStageMode::FullMastering => "+FullGLSL",
        });
    }
    if eff.deband != bal.deband {
        diffs.push("+Deband3");
    }
    if eff.gpu_scaler != bal.gpu_scaler {
        diffs.push("+Lanczos");
    }
    if cfg.motion_mode != MotionMode::Off {
        diffs.push("+Motion");
    }
    if eff.demuxer_cache != bal.demuxer_cache {
        diffs.push("RAM:Mod");
    }
    diffs
}

fn render_manual_matrix_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let is_manual = state.config.performance_mode == PerformanceMode::Manual;
    let title = if is_manual {
        " 10-Point Manual & Thermal Matrix [a] (ACTIVE) "
    } else {
        " 10-Point Manual & Thermal Matrix [a] (Click/Space) "
    };
    let blk = section_block(title, focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 26 || inner.height < 14 {
        return;
    }

    let eff = state.config.effective_mastering();
    let hdr_val = if eff.hdr_compute_peak {
        "ON (Dynamic Peak)"
    } else {
        "OFF (Static Cool)"
    };
    let hdr_col = if eff.hdr_compute_peak {
        Color::Rgb(251, 191, 36)
    } else {
        Color::Rgb(52, 211, 153)
    };
    let superres_col = match eff.super_res {
        SuperResMode::Off => Color::Rgb(52, 211, 153),
        SuperResMode::Auto1080p => Color::Rgb(56, 189, 248),
        SuperResMode::Always2x => Color::Rgb(251, 191, 36),
    };
    let codec_col = match eff.codec_priority {
        CodecPriority::Vp9HwFirst => Color::Rgb(52, 211, 153),
        CodecPriority::Av1First => Color::Rgb(251, 191, 36),
    };

    let rows_spec: [(ManualToggle, &str, &str, &str, Color); 10] = [
        (
            ManualToggle::SuperRes,
            "5K 2x Upscale",
            eff.super_res.label(),
            "GPU",
            superres_col,
        ),
        (
            ManualToggle::CodecPriority,
            "Codec Prio",
            eff.codec_priority.label(),
            "CPU",
            codec_col,
        ),
        (
            ManualToggle::Hwdec,
            "Hardware Dec",
            eff.hwdec.label(),
            "Dec",
            theme.secondary,
        ),
        (
            ManualToggle::HdrComputePeak,
            "HDR Peak Comp",
            hdr_val,
            "HDR",
            hdr_col,
        ),
        (
            ManualToggle::ToneMap,
            "HDR Tone Map",
            eff.tone_map.label(),
            "Map",
            theme.text,
        ),
        (
            ManualToggle::GlslStage,
            "4:4:4 & Deblk",
            eff.glsl_stage.label(),
            "GLSL",
            theme.primary,
        ),
        (
            ManualToggle::Deband,
            "Deband Filter",
            eff.deband.label(),
            "Band",
            theme.text,
        ),
        (
            ManualToggle::GpuScaler,
            "GPU Scaler",
            eff.gpu_scaler.label(),
            "Scl",
            theme.text,
        ),
        (
            ManualToggle::Motion,
            "120Hz Motion",
            state.config.motion_mode.label(),
            "Hz",
            theme.secondary,
        ),
        (
            ManualToggle::DemuxerCache,
            "Demuxer Cache",
            eff.demuxer_cache.label(),
            "RAM",
            theme.accent,
        ),
    ];

    let outer_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // 10-point matrix (exact 10 lines)
            Constraint::Min(4),     // Inspector + Flags + Delta stack
        ])
        .split(inner);

    let active_toggle = ManualToggle::ALL
        .get(state.manual_mastering_cursor)
        .copied()
        .unwrap_or(ManualToggle::SuperRes);
    let (insp_name, mpv_prop, insp_desc, is_live_swap) = manual_inspector_meta(active_toggle);
    let diffs = delta_vs_balanced_badges(&state.config);
    let flags_preview = mpv_mastering_flags_with_manual(
        state.config.shader_mode,
        VideoMasteringProfile::StandardHd,
        state.config.performance_mode,
        Some(eff),
    )
    .into_iter()
    .take(6)
    .collect::<Vec<_>>()
    .join(" ");

    let hits = &mut state.live_playback.play_hitboxes;
    let label_w = if outer_rows[0].width >= 48 { 14 } else { 11 };
    let mut lines = Vec::with_capacity(10);
    for (idx, (toggle, lbl, val, tag, col)) in rows_spec.into_iter().enumerate() {
        let row_off = lines.len() as u16;
        let highlighted = focused && state.manual_mastering_cursor == idx;
        let (dots, dot_col) = heat_dots(toggle, &state.config);
        let mut line = studio_kv_row(
            lbl,
            val,
            Some(tag),
            label_w,
            highlighted,
            Style::default().fg(col),
            outer_rows[0].width.saturating_sub(5),
            &theme,
        );
        line.spans
            .push(Span::styled(format!(" {dots}"), Style::default().fg(dot_col)));
        lines.push(line);
        push_line_hit(hits, outer_rows[0], row_off, PlayOptionsHit::ToggleManual(toggle));
    }
    frame.render_widget(Paragraph::new(lines), outer_rows[0]);

    // Bottom Inspector + Flags + Delta stack (zero truncation, zero blank void)
    let bot = outer_rows[1];
    let show_flags = bot.height >= 7;
    let show_delta = bot.height >= 8;
    let mut b_constraints = vec![
        Constraint::Length(1), // Divider
        Constraint::Length(1), // Inspector Header + LIVE/RELAUNCH badge
        Constraint::Min(2),    // Wrapped untruncated Inspector body
    ];
    if show_flags {
        b_constraints.push(Constraint::Length(1)); // Divider before Flags
        b_constraints.push(Constraint::Length(2)); // Effective mpv flags preview
    }
    if show_delta {
        b_constraints.push(Constraint::Length(1)); // Delta vs BAL strip + [Seed BAL]
    }
    let bot_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(b_constraints)
        .split(bot);

    // Divider
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(bot.width as usize),
            Style::default().fg(theme.divider),
        )),
        bot_chunks[0],
    );

    // Header
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" Inspector [{insp_name}] "),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("· Prop: {mpv_prop} "), Style::default().fg(theme.text_dim)),
            Span::styled(
                if is_live_swap { "[LIVE]" } else { "[RELAUNCH]" },
                Style::default()
                    .fg(if is_live_swap {
                        Color::Rgb(52, 211, 153)
                    } else {
                        Color::Rgb(251, 191, 36)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        bot_chunks[1],
    );

    // Wrapped description with clean 1-cell horizontal padding (never truncates mid-sentence!)
    frame.render_widget(
        Paragraph::new(Span::styled(insp_desc, Style::default().fg(theme.text)))
            .wrap(Wrap { trim: true }),
        Rect::new(
            bot_chunks[2].x.saturating_add(1),
            bot_chunks[2].y,
            bot_chunks[2].width.saturating_sub(2),
            bot_chunks[2].height,
        ),
    );

    let mut b_idx = 3usize;
    if show_flags {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "─".repeat(bot.width as usize),
                Style::default().fg(theme.divider),
            )),
            bot_chunks[b_idx],
        );
        b_idx += 1;
        let flags_txt = if flags_preview.is_empty() {
            "Flags: Raw native pass-through (zero scaler/deband overhead)".to_string()
        } else {
            format!("Flags: {flags_preview}")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(flags_txt, Style::default().fg(theme.secondary)))
                .wrap(Wrap { trim: true }),
            Rect::new(
                bot_chunks[b_idx].x.saturating_add(1),
                bot_chunks[b_idx].y,
                bot_chunks[b_idx].width.saturating_sub(2),
                bot_chunks[b_idx].height,
            ),
        );
        b_idx += 1;
    }

    if show_delta && b_idx < bot_chunks.len() {
        let delta_summary = if diffs.is_empty() {
            "Delta vs BAL: Exact Balanced cool baseline".to_string()
        } else {
            format!("Delta vs BAL: {}", diffs.join(", "))
        };
        let d_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(16), Constraint::Length(12)])
            .split(bot_chunks[b_idx]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", truncate_words(&delta_summary, d_cols[0].width.saturating_sub(1) as usize)),
                Style::default().fg(theme.text_dim),
            )),
            d_cols[0],
        );
        render_inline_pills(
            frame,
            d_cols[1],
            None,
            &[(
                "[Seed BAL]".to_string(),
                Style::default().fg(Color::Rgb(56, 189, 248)),
                PlayOptionsHit::SeedManualFrom(PerformanceMode::Balanced),
            )],
            hits,
        );
    }

    push_pane_hit(hits, area, 1);
}
