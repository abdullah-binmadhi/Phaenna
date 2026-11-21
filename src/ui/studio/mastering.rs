//! Page 3 — `[3] Mastering & GPU`: Live Signal-Flow Pipeline Visualizer,
//! GPU / RAM / Thermal Impact Estimator, 4K Shaders, and 10-Point Manual Matrix.

use crate::config::{
    CodecPriority, Config, DebandTier, DemuxerCachePreset, GlslStageMode, GpuScalerMode,
    HwdecMode, MotionMode, PerformanceMode, SuperResMode, VideoShaderMode,
};
use crate::services::shaders::neural_shader_installed;
use crate::state::{AppState, ManualToggle, PlayOptionsHit};
use crate::ui::studio::widgets::{
    mode_badge_color, push_line_hit, push_pane_hit, section_block, segmented_gauge_line,
    studio_kv_row, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
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
            if cfg.shader_mode == VideoShaderMode::NeuralCinema && eff.super_res != SuperResMode::Off {
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
    let blk = section_block(" Active GPU Signal-Flow Pipeline (vo=gpu-next · libplacebo) ", false, &theme);
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
        Span::styled(format!(" [{dec_tag}]"), stage_style(true, Color::Rgb(52, 211, 153))),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled("[Deblock]", stage_style(deblock_on, theme.primary)),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled("[4:4:4 Chroma]", stage_style(chroma_on, theme.primary)),
        Span::styled(" ─▶ ", Style::default().fg(theme.text_dim)),
        Span::styled(format!("[{sr_tag}]"), stage_style(sr_on || (shader_on && eff.glsl_stage == GlslStageMode::LightCas), theme.accent)),
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
            " Scaler: {} | Cache: {} | HDR Peak: {} | Codec Prio: {}",
            eff.gpu_scaler.label(),
            eff.demuxer_cache.label(),
            if eff.hdr_compute_peak { "Dynamic" } else { "Cool Static" },
            eff.codec_priority.label()
        ),
        Style::default().fg(theme.text_dim),
    ));

    frame.render_widget(Paragraph::new(vec![line1, line2]), inner);
}

fn render_mode_shaders_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Engine Mode, Impact & 4K Shaders ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 10 {
        return;
    }

    let impact = estimate_mastering_impact(&state.config);
    let hits = &mut state.live_playback.play_hitboxes;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Mode pills
            Constraint::Length(1), // 1: Seed from pills
            Constraint::Length(1), // 2: Impact header
            Constraint::Length(1), // 3: GPU Load gauge
            Constraint::Length(1), // 4: RAM gauge
            Constraint::Length(1), // 5: Thermal gauge
            Constraint::Length(1), // 6: Shaders header
            Constraint::Min(5),    // 7: Shader list + CNN pack
        ])
        .split(inner);

    // Row 0: Mode pills
    let pill_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Percentage(19),
            Constraint::Percentage(27),
            Constraint::Percentage(27),
            Constraint::Percentage(27),
        ])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Mode[m]: ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        pill_cols[0],
    );
    for (idx, pm) in PerformanceMode::ALL.iter().enumerate() {
        let active = *pm == state.config.performance_mode;
        let col = mode_badge_color(*pm);
        let lbl = if (pill_cols[idx + 1].width as usize) >= pm.label().len() + 2 {
            format!(" {} ", pm.label())
        } else {
            format!(" {} ", pm.short_badge())
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                lbl,
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(col)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
            )),
            pill_cols[idx + 1],
        );
        hits.push((pill_cols[idx + 1], PlayOptionsHit::SetPerformanceMode(*pm)));
    }

    // Row 1: Seed from presets
    let seed_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Min(10),
        ])
        .split(rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(" Seed Man:", Style::default().fg(theme.text_dim))),
        seed_cols[0],
    );
    let seeds = [
        (PerformanceMode::Efficient, "[Seed ECO]", Color::Rgb(52, 211, 153)),
        (PerformanceMode::Balanced, "[Seed BAL]", Color::Rgb(56, 189, 248)),
        (PerformanceMode::Quality, "[Seed MAX]", Color::Rgb(251, 191, 36)),
    ];
    for (idx, (pm, lbl, col)) in seeds.into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(lbl, Style::default().fg(col))),
            seed_cols[idx + 1],
        );
        hits.push((seed_cols[idx + 1], PlayOptionsHit::SeedManualFrom(pm)));
    }

    // Impact Gauges
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Resource & Thermal Budget (Estimated):",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[2],
    );
    let ram_pct = ((impact.ram_mb * 100) / 1600).min(100) as u8;
    let w = rows[3].width as usize;
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "GPU Load",
            impact.gpu_pct,
            &format!("{:>3}%", impact.gpu_pct),
            w,
            &theme,
        )),
        rows[3],
    );
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Est RAM",
            ram_pct,
            &format!("{}M", impact.ram_mb),
            w,
            &theme,
        )),
        rows[4],
    );
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Thermal",
            impact.thermal_pct,
            &format!("{:>3}%", impact.thermal_pct),
            w,
            &theme,
        )),
        rows[5],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            " 4K Mastering Shader [s] (Hot-Swap Live):",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[6],
    );

    let all_shaders = [
        (VideoShaderMode::Reference4K, "Reference 4K", "Deblock+4:4:4+RCAS"),
        (VideoShaderMode::NeuralCinema, "Neural Cinema", "FSRCNNX CNN+4:4:4"),
        (VideoShaderMode::Anime4K, "Anime4K", "Line restore 2x"),
        (VideoShaderMode::Balanced, "Balanced", "Deblock+CAS+Deband"),
        (VideoShaderMode::Off, "Off", "Raw native stream"),
    ];
    let mut shader_lines = Vec::new();
    for (m, name, desc) in all_shaders {
        let active = m == state.config.shader_mode;
        let row = shader_lines.len() as u16;
        let name_w = 14usize;
        let desc_budget = (rows[7].width as usize).saturating_sub(name_w + 6);
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
        push_line_hit(hits, rows[7], row, PlayOptionsHit::SelectShader(m));
    }

    if (shader_lines.len() as u16) < rows[7].height {
        let row_cnn = shader_lines.len() as u16;
        let installed = neural_shader_installed();
        shader_lines.push(studio_kv_row(
            "CNN Pack",
            if installed {
                "FSRCNNX Installed (Ready)"
            } else {
                "[Click to Download CNN]"
            },
            None,
            10,
            false,
            Style::default().fg(if installed {
                Color::Rgb(52, 211, 153)
            } else {
                theme.accent
            }),
            rows[7].width,
            &theme,
        ));
        if !installed {
            push_line_hit(hits, rows[7], row_cnn, PlayOptionsHit::DownloadNeuralShaders);
        }
    }

    push_pane_hit(hits, area, 0);
    frame.render_widget(Paragraph::new(shader_lines), rows[7]);
}

fn manual_inspector_text(toggle: ManualToggle) -> &'static str {
    match toggle {
        ManualToggle::SuperRes => "2x Super-Res allocates 5K/8K 16-bit GPU FBOs. Use <=1080p for cool 4K streams.",
        ManualToggle::CodecPriority => "VP9/HEVC decodes cooler in hardware; AV1 saves bandwidth. Press [Enter] to reload codec.",
        ManualToggle::Hwdec => "VideoToolbox/Auto-Safe offloads decode to media silicon; CPU SW dramatically raises heat.",
        ManualToggle::HdrComputePeak => "Dynamic HDR peak measures per-scene brightness; Cool Static avoids compute passes.",
        ManualToggle::ToneMap => "libplacebo HDR-to-SDR curve: Spline (balanced), BT.2446a (broadcast), ST2094-40 (dynamic).",
        ManualToggle::GlslStage => "Controls active GLSL depth: Light CAS -> Guided 4:4:4 chroma -> Full 4K 2x pipeline.",
        ManualToggle::Deband => "16-bit gradient debander removes sky/shadow banding; 3-Pass adds temporal dithering.",
        ManualToggle::GpuScaler => "EWA LanczosSharp adds antiring & linear/sigmoid scaling; Spline36 stays ultra cool.",
        ManualToggle::Motion => "120Hz display-resample cadence or Catmull-Rom interpolation (hot-swaps live via [t]).",
        ManualToggle::DemuxerCache => "Forward/back RAM ring budget (48M / 160M / 512M). Higher reduces Wi-Fi micro-stalls.",
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
        ManualToggle::HdrComputePeak => if eff.hdr_compute_peak { 2 } else { 0 },
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
    if inner.width < 26 || inner.height < 8 {
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
        (ManualToggle::SuperRes, "5K 2x Upscale", eff.super_res.label(), "GPU", superres_col),
        (ManualToggle::CodecPriority, "Codec Prio", eff.codec_priority.label(), "CPU", codec_col),
        (ManualToggle::Hwdec, "Hardware Dec", eff.hwdec.label(), "Dec", theme.secondary),
        (ManualToggle::HdrComputePeak, "HDR Peak Comp", hdr_val, "HDR", hdr_col),
        (ManualToggle::ToneMap, "HDR Tone Map", eff.tone_map.label(), "Map", theme.text),
        (ManualToggle::GlslStage, "4:4:4 & Deblk", eff.glsl_stage.label(), "GLSL", theme.primary),
        (ManualToggle::Deband, "Deband Filter", eff.deband.label(), "Band", theme.text),
        (ManualToggle::GpuScaler, "GPU Scaler", eff.gpu_scaler.label(), "Scl", theme.text),
        (ManualToggle::Motion, "120Hz Motion", state.config.motion_mode.label(), "Hz", theme.secondary),
        (ManualToggle::DemuxerCache, "Demuxer Cache", eff.demuxer_cache.label(), "RAM", theme.accent),
    ];

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(3), // Inspector footer
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let label_w = if rows[0].width >= 48 { 14 } else { 11 };
    let mut lines = Vec::new();
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
            rows[0].width.saturating_sub(5),
            &theme,
        );
        line.spans
            .push(Span::styled(format!(" {dots}"), Style::default().fg(dot_col)));
        lines.push(line);
        push_line_hit(hits, rows[0], row_off, PlayOptionsHit::ToggleManual(toggle));
    }
    frame.render_widget(Paragraph::new(lines), rows[0]);

    let active_toggle = ManualToggle::ALL
        .get(state.manual_mastering_cursor)
        .copied()
        .unwrap_or(ManualToggle::SuperRes);
    let hint = manual_inspector_text(active_toggle);
    let insp_lines = vec![
        Line::from(Span::styled(
            "─".repeat(rows[1].width as usize),
            Style::default().fg(theme.divider),
        )),
        Line::from(vec![
            Span::styled(
                " Inspector: ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_words(hint, rows[1].width.saturating_sub(13) as usize),
                Style::default().fg(theme.text_dim),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(insp_lines), rows[1]);
    push_pane_hit(hits, area, 1);
}
