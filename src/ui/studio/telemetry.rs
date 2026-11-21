//! Page 6 — `[6] Live & Telemetry`: 60s Throughput vs Video Bitrate Braille Graph,
//! Buffer Cushion & Demuxer RAM Chart, Frame Pacing & HW Diagnostics, Live DVR & Event Log.

use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_hms, format_speed_bps, push_pane_hit, render_braille_dual_chart_lines, section_block,
    segmented_gauge_line, sparkline_string, truncate_words,
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
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
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

    let cur_net = net_mbps.last().copied().unwrap_or(0.0);
    let peak_net = net_mbps.iter().copied().fold(0.0_f64, f64::max);
    let cur_vid = if state.live_playback.video_bitrate_kbps > 0.0 {
        state.live_playback.video_bitrate_kbps / 1000.0
    } else {
        state.live_playback.current_stream_tbr_kbps / 1000.0
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

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
            Span::styled(" ━ Net: ", Style::default().fg(Color::Rgb(56, 189, 248))),
            Span::styled(format!("{cur_net:.1}M ({})", format_speed_bps(state.live_playback.cache_speed_bps)), Style::default().fg(theme.text)),
            Span::styled("  ━ Vid: ", Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(format!("{cur_vid:.1}M"), Style::default().fg(theme.text)),
            Span::styled(format!("  Peak:{peak_net:.1}M"), Style::default().fg(theme.text_dim)),
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

    let fps_samples: Vec<f64> = state
        .live_playback
        .telemetry_history
        .vf_fps
        .iter()
        .copied()
        .collect();
    let spark = sparkline_string(&fps_samples, (inner.width as usize).saturating_sub(26).clamp(8, 28));
    let cur_fps = if state.live_playback.estimated_vf_fps > 0.1 {
        format!("{:.1} fps", state.live_playback.estimated_vf_fps)
    } else if state.live_playback.current_stream_fps > 0.1 {
        format!("{:.0} fps (target)", state.live_playback.current_stream_fps)
    } else {
        "--".to_string()
    };

    let codec = if !state.live_playback.video_codec_live.is_empty() {
        state.live_playback.video_codec_live.as_str()
    } else {
        "--"
    };
    let res = if state.live_playback.decoded_width > 0 {
        format!(
            "{}x{}",
            state.live_playback.decoded_width, state.live_playback.decoded_height
        )
    } else if state.live_playback.current_stream_height > 0 {
        format!("{}p", state.live_playback.current_stream_height)
    } else {
        "--".to_string()
    };
    let hwdec = if !state.live_playback.hwdec_current.is_empty() {
        state.live_playback.hwdec_current.as_str()
    } else if state.has_live_ipc() {
        "none / probing"
    } else {
        state.config.effective_mastering().hwdec.label()
    };
    let color = if !state.live_playback.colormatrix.is_empty() {
        format!(
            "{} (Sig-Peak {:.1}x)",
            state.live_playback.colormatrix,
            state.live_playback.hdr_sig_peak.max(1.0)
        )
    } else {
        "--".to_string()
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
        "--".to_string()
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(" VF FPS (30s): ", Style::default().fg(theme.text_dim)),
            Span::styled(spark, Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(format!(" {cur_fps}"), Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" Video Codec : ", Style::default().fg(theme.text_dim)),
            Span::styled(format!("{codec} ({res})"), Style::default().fg(theme.primary)),
        ]),
        Line::from(vec![
            Span::styled(" HW Decoder  : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                hwdec.to_string(),
                Style::default().fg(if hwdec == "no" || hwdec == "none / probing" {
                    Color::Rgb(251, 191, 36)
                } else {
                    Color::Rgb(52, 211, 153)
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Color / HDR : ", Style::default().fg(theme.text_dim)),
            Span::styled(color, Style::default().fg(theme.text)),
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
            Span::styled(audio_pipe, Style::default().fg(theme.secondary)),
        ]),
    ];

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
            Constraint::Length(1), // 0: Live mode + catchup toggle
            Constraint::Length(1), // 1: Edge lag meter
            Constraint::Length(1), // 2: Action buttons
            Constraint::Length(1), // 3: Event log header
            Constraint::Min(2),    // 4: Scrollable events
        ])
        .split(inner);

    let lag = if state.live_playback.is_live_stream {
        state
            .live_playback
            .live_edge_lag_secs(state.active_playback_pos)
    } else {
        0.0
    };
    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Live profile + Auto Catchup
    let r0_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Live [v]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                state.config.live_mode.label(),
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        r0_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Catchup: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.live_auto_catchup {
                    "ON (Auto 1.04x+)"
                } else {
                    "OFF"
                },
                Style::default().fg(theme.secondary),
            ),
        ])),
        r0_cols[1],
    );
    hits.push((r0_cols[0], PlayOptionsHit::CycleLiveMode));
    hits.push((r0_cols[1], PlayOptionsHit::ToggleLiveCatchup));

    // Row 1: Visual Live Edge Lag Meter
    let max_lag = state
        .config
        .live_mode
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

    // Row 2: DVR Action Buttons
    let btn_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(rows[2]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [E Live Edge] ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        )),
        btn_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [S From Start] ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        btn_cols[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [R Audio Radio] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        btn_cols[2],
    );
    hits.push((btn_cols[0], PlayOptionsHit::JumpToLiveEdge));
    hits.push((btn_cols[1], PlayOptionsHit::PlayLiveFromStart));
    hits.push((btn_cols[2], PlayOptionsHit::LaunchAudioRadio));

    // Row 3 & 4: Session Event Log
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Session Event Log (Latest First):",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[3],
    );

    let events = &state.live_playback.telemetry_history.events;
    let mut ev_lines = Vec::new();
    if events.is_empty() {
        ev_lines.push(Line::from(Span::styled(
            "  No live session events recorded yet.",
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = rows[4].height as usize;
        let max_scroll = events.len().saturating_sub(cap);
        let scroll = state.studio.event_log_scroll.min(max_scroll);
        for ev in events.iter().rev().skip(scroll).take(cap) {
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
