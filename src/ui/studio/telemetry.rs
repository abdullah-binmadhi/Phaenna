//! Page 6 — `[6] Live & Telemetry`: 60s Throughput vs Video Bitrate Braille Graph,
//! Buffer Cushion & Demuxer RAM Chart, 4-Pill Health Scorecard, Frame Pacing & HW Diagnostics,
//! Live Broadcast DVR Pills & Filterable Session Event Log.

use crate::config::{HwdecMode, LiveStreamMode, Quality, VideoShaderMode};
use crate::state::{AppState, EventLogFilter, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_hms, format_speed_bps, push_pane_hit, render_braille_dual_chart_lines,
    render_braille_standby_chart_lines, render_inline_pills, section_block, segmented_gauge_line,
    sparkline_string, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render_telemetry_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 34 || area.height < 12 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[1]);

    render_throughput_bitrate_chart(frame, top_cols[0], state);
    render_buffer_ram_chart(frame, top_cols[1], state);
    render_frame_hw_diagnostics(frame, bot_cols[0], state);
    render_live_dvr_and_events(frame, bot_cols[1], state);
}

fn pill_style(active: bool, bg: Color, idle_fg: Color) -> Style {
    if active {
        Style::default()
            .fg(Color::Black)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(idle_fg)
    }
}

fn estimated_target_mbps(state: &AppState) -> f64 {
    let q = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);
    match q {
        Quality::Best | Quality::P2160 => 22.0,
        Quality::P1440 => 12.0,
        Quality::P1080 => 5.2,
        Quality::P720 => 2.5,
        Quality::P480 => 1.1,
        Quality::P360 => 0.6,
        Quality::AudioOnly => 0.2,
    }
}

fn render_throughput_bitrate_chart(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" 60s Throughput vs Video Bitrate (Mbps) ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 4 {
        return;
    }

    let net_mbps: Vec<f64> = state
        .live_playback
        .telemetry_history
        .cache_speed_kbps
        .iter()
        .map(|k| k / 1000.0)
        .collect();
    let vid_mbps: Vec<f64> = state
        .live_playback
        .telemetry_history
        .video_bitrate_kbps
        .iter()
        .map(|k| k / 1000.0)
        .collect();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let target_mbps = estimated_target_mbps(state);
    if net_mbps.is_empty() {
        let cap = format!("Standby · Target ~{target_mbps:.1}M ceiling (probes live)");
        let lines = render_braille_standby_chart_lines(
            rows[0].width as usize,
            rows[0].height as usize,
            (target_mbps * 1.4).max(10.0),
            target_mbps,
            "M",
            &cap,
            Color::Rgb(56, 189, 248),
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[0]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        &format!("Target: {target_mbps:.1}M · Codec: {} (Standby pre-launch)", state.config.effective_mastering().codec_priority.label()),
                        rows[1].width.saturating_sub(1) as usize,
                    )
                ),
                Style::default().fg(theme.text_dim),
            )),
            rows[1],
        );
        return;
    }

    let cur_net = net_mbps.last().copied().unwrap_or(0.0);
    let avg_net = net_mbps.iter().sum::<f64>() / net_mbps.len().max(1) as f64;
    let mut sorted = net_mbps.clone();
    sorted.sort_by(f64::total_cmp);
    let p95_idx = ((sorted.len() as f64) * 0.95).floor() as usize;
    let p95_net = sorted.get(p95_idx.min(sorted.len() - 1)).copied().unwrap_or(cur_net);
    let cur_vid = if state.live_playback.video_bitrate_kbps > 0.0 {
        state.live_playback.video_bitrate_kbps / 1000.0
    } else {
        state.live_playback.current_stream_tbr_kbps / 1000.0
    };

    let lines = render_braille_dual_chart_lines(
        &net_mbps,
        Some(&vid_mbps),
        None,
        rows[0].width as usize,
        rows[0].height as usize,
        10.0,
        "M",
        Color::Rgb(56, 189, 248),
        Color::Rgb(52, 211, 153),
        &theme,
    );
    frame.render_widget(Paragraph::new(lines), rows[0]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ━ Net:", Style::default().fg(Color::Rgb(56, 189, 248))),
            Span::styled(
                format!("{cur_net:.1}M ({})", format_speed_bps(state.live_playback.cache_speed_bps)),
                Style::default().fg(theme.text),
            ),
            Span::styled(format!(" Avg:{avg_net:.1}M"), Style::default().fg(theme.text_dim)),
            Span::styled(" ━ Vid:", Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(format!("{cur_vid:.1}M"), Style::default().fg(theme.text)),
            Span::styled(format!(" P95:{p95_net:.1}M"), Style::default().fg(theme.text_dim)),
        ])),
        rows[1],
    );
}

fn render_buffer_ram_chart(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" 60s Buffer Cushion & Demuxer RAM ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 4 {
        return;
    }

    let ahead: Vec<f64> = state
        .live_playback
        .telemetry_history
        .cache_ahead_secs
        .iter()
        .copied()
        .collect();
    let stalls: Vec<bool> = state
        .live_playback
        .telemetry_history
        .stall_samples
        .iter()
        .copied()
        .collect();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    if ahead.is_empty() {
        let target_ahead = state.config.network_mode.vod_readahead_secs() as f64;
        let max_mb = (state.config.effective_demuxer_max_bytes_num() / (1024 * 1024)).max(1);
        let cap = format!("Target Readahead: {target_ahead:.0}s · {max_mb}M Ring (Standby)");
        let lines = render_braille_standby_chart_lines(
            rows[0].width as usize,
            rows[0].height as usize,
            (target_ahead * 1.2).max(60.0),
            target_ahead,
            "s",
            &cap,
            theme.primary,
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[0]);
    } else {
        let lines = render_braille_dual_chart_lines(
            &ahead,
            None,
            Some(&stalls),
            rows[0].width as usize,
            rows[0].height as usize,
            30.0,
            "s",
            theme.primary,
            theme.secondary,
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[0]);
    }

    let cache_mb = state.live_playback.cache_bytes / (1024 * 1024);
    let max_mb = (state.config.effective_demuxer_max_bytes_num() / (1024 * 1024)).max(1);
    let pct = ((cache_mb * 100) / max_mb).min(100) as u8;
    let right = format!(
        "{cache_mb}/{max_mb}M · Ahead:{:.1}s · Stl:{}",
        state.live_playback.cache_ahead_secs, state.live_playback.stall_count
    );
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Demux RAM",
            pct,
            &right,
            rows[1].width as usize,
            &theme,
        )),
        rows[1],
    );
}

fn render_frame_hw_diagnostics(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" Frame Pacing & Hardware Diagnostics ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 6 {
        return;
    }

    let live = state.has_live_ipc();
    let eff = state.config.effective_mastering();
    let hwdec = if !state.live_playback.hwdec_current.is_empty() {
        state.live_playback.hwdec_current.as_str()
    } else if live {
        "none / probing"
    } else {
        eff.hwdec.label()
    };
    let hw_ok = if live {
        hwdec != "no" && hwdec != "none / probing"
    } else {
        eff.hwdec != HwdecMode::Off
    };
    let pace_ok = state.live_playback.dropped_frames <= 5;
    let sync_ok = state.live_playback.avsync_ms.abs() <= 25.0;
    let cache_ok = !state.live_playback.paused_for_cache
        && (!live || state.live_playback.cache_ahead_secs >= 3.0);

    let (health_lbl, health_col) = if !live {
        ("STANDBY", theme.secondary)
    } else if hw_ok && pace_ok && sync_ok && cache_ok {
        ("OPTIMAL", Color::Rgb(52, 211, 153))
    } else {
        ("WATCH", Color::Rgb(251, 191, 36))
    };

    let fps_samples: Vec<f64> = state
        .live_playback
        .telemetry_history
        .vf_fps
        .iter()
        .copied()
        .collect();
    let spark = sparkline_string(
        &fps_samples,
        (inner.width as usize).saturating_sub(28).clamp(8, 26),
    );
    let cur_fps = if state.live_playback.estimated_vf_fps > 0.1 {
        format!("{:.1} fps", state.live_playback.estimated_vf_fps)
    } else if state.live_playback.current_stream_fps > 0.1 {
        format!("{:.0} fps (target)", state.live_playback.current_stream_fps)
    } else {
        "Standby".to_string()
    };

    let codec = if !state.live_playback.video_codec_live.is_empty() {
        state.live_playback.video_codec_live.as_str()
    } else {
        eff.codec_priority.label()
    };
    let res = if state.live_playback.decoded_width > 0 {
        format!(
            "{}x{}",
            state.live_playback.decoded_width, state.live_playback.decoded_height
        )
    } else if state.live_playback.current_stream_height > 0 {
        format!("{}p", state.live_playback.current_stream_height)
    } else {
        "Target Auto".to_string()
    };
    let color = if !state.live_playback.colormatrix.is_empty() {
        format!(
            "{} (Sig-Peak {:.1}x)",
            state.live_playback.colormatrix,
            state.live_playback.hdr_sig_peak.max(1.0)
        )
    } else {
        format!("bt.709 / {} (Configured)", eff.tone_map.mpv_value())
    };
    let audio_pipe = if state.live_playback.audio_samplerate > 0 {
        format!(
            "{} Hz · {} · {:.0} kbps",
            state.live_playback.audio_samplerate,
            if state.live_playback.audio_channels.is_empty() {
                "stereo"
            } else {
                state.live_playback.audio_channels.as_str()
            },
            state.live_playback.audio_bitrate_kbps
        )
    } else {
        format!(
            "{} · EQ: {}",
            state.config.audio_channel_mode.short_label(),
            state.config.audio_eq_preset.short_label()
        )
    };

    let pass_cnt = if state.config.shader_mode == VideoShaderMode::Off {
        0
    } else {
        match eff.glsl_stage {
            crate::config::GlslStageMode::Off => 0,
            crate::config::GlslStageMode::LightCas => 1,
            crate::config::GlslStageMode::Guided444 => 2,
            crate::config::GlslStageMode::FullMastering => {
                if state.config.shader_mode == VideoShaderMode::Balanced {
                    2
                } else {
                    3
                }
            }
        }
    };

    let rem_secs = (state.active_playback_dur - state.active_playback_pos).max(0.0);
    let badge = |ok: bool, good: &str, warn: &str| {
        Span::styled(
            if ok { good.to_string() } else { warn.to_string() },
            Style::default()
                .fg(if !live {
                    theme.text_dim
                } else if ok {
                    Color::Rgb(52, 211, 153)
                } else {
                    Color::Rgb(251, 191, 36)
                })
                .add_modifier(Modifier::BOLD),
        )
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(" Pipeline : ", Style::default().fg(theme.text_dim)),
            badge(hw_ok, "[HW ✓]", "[SW !]"),
            Span::raw(" "),
            badge(pace_ok, "[Pace ✓]", "[Drop !]"),
            Span::raw(" "),
            badge(sync_ok, "[Sync ✓]", "[Drift !]"),
            Span::raw(" "),
            badge(cache_ok, "[Buf ✓]", "[Low !]"),
            Span::raw(" "),
            Span::styled(health_lbl, Style::default().fg(health_col).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" VF FPS (30s): ", Style::default().fg(theme.text_dim)),
            Span::styled(spark, Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(
                format!(" {cur_fps}"),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Video Codec : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_words(&format!("{codec} ({res})"), inner.width.saturating_sub(16) as usize),
                Style::default().fg(theme.primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(" HW Decoder  : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                hwdec.to_string(),
                Style::default().fg(if hw_ok {
                    Color::Rgb(52, 211, 153)
                } else {
                    Color::Rgb(251, 191, 36)
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Color / HDR : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_words(&color, inner.width.saturating_sub(16) as usize),
                Style::default().fg(theme.text),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Frame Drops : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!(
                    "{} dropped / {} delayed",
                    state.live_playback.dropped_frames, state.live_playback.delayed_frames
                ),
                Style::default().fg(if state.live_playback.dropped_frames > 15 {
                    Color::Rgb(248, 113, 113)
                } else {
                    theme.text
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Audio Pipe  : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_words(&audio_pipe, inner.width.saturating_sub(16) as usize),
                Style::default().fg(theme.secondary),
            ),
        ]),
    ];

    if (lines.len() as u16) + 2 <= inner.height {
        lines.push(Line::from(vec![
            Span::styled(" Render Spec : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_words(
                    &format!(
                        "{} + {} · {}shd",
                        eff.gpu_scaler.label(),
                        state.config.motion_mode.short_badge(),
                        pass_cnt
                    ),
                    inner.width.saturating_sub(16) as usize,
                ),
                Style::default().fg(theme.text),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" Session IPC : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                truncate_words(
                    &format!(
                        "{} · Rem {} ({:.2}x) · SB {}",
                        if live { "Live Socket" } else { "Standby" },
                        format_hms(rem_secs),
                        state.live_playback.speed,
                        format_hms(state.analytics.sponsorblock_saved_secs)
                    ),
                    inner.width.saturating_sub(16) as usize,
                ),
                Style::default().fg(theme.accent),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_live_dvr_and_events(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Live Broadcast DVR & Session Event Log ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Live mode pills + catchup toggle
            Constraint::Length(1), // 1: Edge lag meter
            Constraint::Length(1), // 2: Action buttons
            Constraint::Length(1), // 3: Event log filter bar
            Constraint::Min(2),    // 4: Scrollable filtered events
        ])
        .split(inner);

    let lag = if state.live_playback.is_live_stream {
        state
            .live_playback
            .live_edge_lag_secs(state.active_playback_pos)
    } else {
        0.0
    };
    let live_mode = state.config.live_mode;
    let catchup = state.config.live_auto_catchup;
    let ev_filter = state.studio.event_log_filter;
    let ev_scroll = state.studio.event_log_scroll;
    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Inline Live Mode pills + Catchup
    let mode_pills = vec![
        (
            "[Low-Lat]".to_string(),
            pill_style(
                live_mode == LiveStreamMode::LowLatency,
                Color::Rgb(52, 211, 153),
                theme.text_dim,
            ),
            PlayOptionsHit::SetLiveMode(LiveStreamMode::LowLatency),
        ),
        (
            "[Bal 6s]".to_string(),
            pill_style(
                live_mode == LiveStreamMode::BalancedLive,
                Color::Rgb(52, 211, 153),
                theme.text_dim,
            ),
            PlayOptionsHit::SetLiveMode(LiveStreamMode::BalancedLive),
        ),
        (
            "[DVR 20s]".to_string(),
            pill_style(
                live_mode == LiveStreamMode::SmoothDvr,
                theme.primary,
                theme.text_dim,
            ),
            PlayOptionsHit::SetLiveMode(LiveStreamMode::SmoothDvr),
        ),
        (
            "[VOD]".to_string(),
            pill_style(live_mode == LiveStreamMode::Off, theme.secondary, theme.text_dim),
            PlayOptionsHit::SetLiveMode(LiveStreamMode::Off),
        ),
        (
            if catchup { "[Catchup:ON]" } else { "[Catchup:OFF]" }.to_string(),
            pill_style(catchup, theme.secondary, theme.text_dim),
            PlayOptionsHit::ToggleLiveCatchup,
        ),
    ];
    render_inline_pills(
        frame,
        rows[0],
        Some((" Live [v]:", Style::default().fg(theme.text_dim))),
        &mode_pills,
        hits,
    );

    // Row 1: Visual Live Edge Lag Meter
    let max_lag = live_mode
        .catchup_threshold_and_speed()
        .map(|(t, _, _)| t)
        .unwrap_or(12.0);
    let lag_pct = ((lag / max_lag.max(1.0)).clamp(0.0, 1.0) * 100.0).round() as u8;
    frame.render_widget(
        Paragraph::new(segmented_gauge_line(
            "Edge Lag",
            lag_pct,
            &format!("-{lag:.1}s / {max_lag:.0}s"),
            rows[1].width as usize,
            &theme,
        )),
        rows[1],
    );

    // Row 2: DVR Action Buttons via render_inline_pills
    let dvr_pills = vec![
        (
            "[E Live Edge]".to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::JumpToLiveEdge,
        ),
        (
            "[S From Start]".to_string(),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::PlayLiveFromStart,
        ),
        (
            "[R Audio Radio]".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::LaunchAudioRadio,
        ),
    ];
    render_inline_pills(
        frame,
        rows[2],
        Some((" DVR Cmd :", Style::default().fg(theme.text_dim))),
        &dvr_pills,
        hits,
    );

    // Row 3: Event Log Filter Pills + Clear
    let mut filter_pills: Vec<(String, Style, PlayOptionsHit)> = EventLogFilter::ALL
        .into_iter()
        .map(|f| {
            (
                format!("[{}]", f.label()),
                pill_style(ev_filter == f, theme.primary, theme.text_dim),
                PlayOptionsHit::SetEventLogFilter(f),
            )
        })
        .collect();
    filter_pills.push((
        "[Clr]".to_string(),
        Style::default().fg(Color::Rgb(248, 113, 113)),
        PlayOptionsHit::ClearEventLog,
    ));
    render_inline_pills(
        frame,
        rows[3],
        Some((
            " Events [F]:",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        &filter_pills,
        hits,
    );

    // Row 4: Filtered Session Event Log
    let events: Vec<_> = state
        .live_playback
        .telemetry_history
        .events
        .iter()
        .rev()
        .filter(|e| ev_filter.matches(e.kind))
        .collect();
    let mut ev_lines = Vec::new();
    if events.is_empty() {
        ev_lines.push(Line::from(Span::styled(
            if state.live_playback.telemetry_history.events.is_empty() {
                "  Standby: mpv IPC logs stalls, ABR swaps, skips & shader changes here."
            } else {
                "  No events match active filter. Press [F] or click [All]."
            },
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = rows[4].height as usize;
        let max_scroll = events.len().saturating_sub(cap);
        let scroll = ev_scroll.min(max_scroll);
        for ev in events.into_iter().skip(scroll).take(cap) {
            let kind_col = match ev.kind {
                "STALL" => Color::Rgb(248, 113, 113),
                "SKIP" => Color::Rgb(251, 191, 36),
                "ABR" => Color::Rgb(56, 189, 248),
                "SHADER" | "MODE" => theme.secondary,
                _ => Color::Rgb(52, 211, 153),
            };
            let msg_budget = rows[4].width.saturating_sub(17).max(10) as usize;
            ev_lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:>6} ", format_hms(ev.media_pos_secs)),
                    Style::default().fg(theme.text_dim),
                ),
                Span::styled(
                    format!("[{:<6}] ", ev.kind),
                    Style::default().fg(kind_col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_words(&ev.summary, msg_budget),
                    Style::default().fg(theme.text),
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(ev_lines), rows[4]);
    push_pane_hit(hits, area, 1);
}
