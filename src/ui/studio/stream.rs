//! Page 2 — `[2] Stream & Net`: Quality Ladder, Codec & FPS Controls,
//! Data-Usage Estimator, Bitrate vs Throughput Graph, Audio Codec Compare, and Network Shield.

use crate::api::ytdlp::ResolvedStreamLadder;
use crate::config::{CodecPriority, NetworkMode, Player, Quality};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_hms, format_speed_bps, push_line_hit, push_pane_hit, render_inline_pills,
    section_block, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render_stream_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 32 || area.height < 12 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(area);

    render_quality_player_pane(frame, cols[0], state);

    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(cols[1]);

    render_bitrate_ladder_chart(frame, right_rows[0], state);
    render_network_shield_pane(frame, right_rows[1], state);
}

fn active_video_id(state: &AppState) -> Option<String> {
    state
        .active_playback_video_id
        .clone()
        .or_else(|| state.active_selected_video().map(|v| v.id))
}

fn lookup_ladder(state: &AppState, vid: Option<&String>) -> Option<ResolvedStreamLadder> {
    vid.and_then(|id| {
        let clean = id.trim_start_matches("RD");
        state
            .stream_ladder_cache
            .get(id)
            .or_else(|| state.stream_ladder_cache.get(clean))
            .cloned()
    })
}

fn estimate_tier_mbps(q: Quality, ladder: Option<&ResolvedStreamLadder>) -> f64 {
    if let Some(lad) = ladder {
        if q == Quality::AudioOnly {
            if let Some(a) = lad.audio_variants.first() {
                return (a.abr_kbps / 1000.0).max(0.06);
            }
        } else {
            let max_h = q.max_height_or_default(4320);
            if let Some(v) = lad
                .video_variants
                .iter()
                .filter(|v| v.height <= max_h)
                .max_by(|a, b| {
                    a.height
                        .cmp(&b.height)
                        .then_with(|| a.tbr_kbps.total_cmp(&b.tbr_kbps))
                })
            {
                return (v.tbr_kbps / 1000.0).max(0.4);
            }
        }
    }
    match q {
        Quality::Best | Quality::P2160 => 22.0,
        Quality::P1440 => 12.0,
        Quality::P1080 => 5.2,
        Quality::P720 => 2.5,
        Quality::P480 => 1.1,
        Quality::P360 => 0.6,
        Quality::AudioOnly => 0.16,
    }
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

fn render_quality_player_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Stream Quality, Codec & Backend ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 12 {
        return;
    }

    let vid_opt = active_video_id(state);
    let ladder = lookup_ladder(state, vid_opt.as_ref());
    let dur_secs = if state.active_playback_dur > 1.0 {
        state.active_playback_dur
    } else {
        state
            .active_selected_video()
            .and_then(|v| v.duration)
            .unwrap_or(0) as f64
    };
    let selected_q = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);
    let codec_prio = state.config.effective_mastering().codec_priority;
    let max_fps_cap = state.config.max_fps_cap;
    let host_players = Player::host_supported();
    let player_now = state.config.player.resolve_supported_for_host();
    let net_mode = state.config.network_mode;
    let show_dividers = inner.height >= 20;

    let mut constraints = vec![
        Constraint::Length(1), // 0: Header
        Constraint::Length(8), // 1: Quality list
    ];
    if show_dividers {
        constraints.push(Constraint::Length(1)); // divider
    }
    constraints.extend([
        Constraint::Length(1), // Codec Priority pills
        Constraint::Length(1), // FPS Cap pills
        Constraint::Length(1), // Data Usage estimator
    ]);
    if show_dividers {
        constraints.push(Constraint::Length(1)); // divider
    }
    constraints.extend([
        Constraint::Length(1),                              // Player Engine header
        Constraint::Length((host_players.len() + 1) as u16), // Host-supported players + OS note
        Constraint::Min(1),                                 // Selected tier note + Refresh + selector footer
    ]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ci = 0usize;

    // 0: Target tier header
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Target Resolution Tier:",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        chunks[ci],
    );
    ci += 1;

    // 1: 8 Quality rows
    let q_area = chunks[ci];
    ci += 1;
    let name_budget = q_area.width.saturating_sub(22).max(12) as usize;
    let mut q_lines = Vec::with_capacity(Quality::ALL.len());
    for (i, q) in Quality::ALL.iter().enumerate() {
        let active = i == state.play_quality_idx;
        let row = q_lines.len() as u16;
        let badge = if let Some(ref lad) = ladder {
            let max_h = q.max_height_or_default(4320);
            if *q == Quality::AudioOnly {
                lad.audio_variants
                    .first()
                    .map(|a| {
                        format!(
                            "[{}] {:>3.0}K",
                            a.acodec.split('.').next().unwrap_or("opus"),
                            a.abr_kbps
                        )
                    })
                    .unwrap_or_else(|| "[Audio]".to_string())
            } else {
                lad.video_variants
                    .iter()
                    .filter(|v| v.height <= max_h)
                    .max_by(|a, b| {
                        a.height
                            .cmp(&b.height)
                            .then_with(|| a.tbr_kbps.total_cmp(&b.tbr_kbps))
                    })
                    .map(|v| {
                        format!(
                            "[{}] {:>4.1}M",
                            v.vcodec.split('.').next().unwrap_or("vid"),
                            v.tbr_kbps / 1000.0
                        )
                    })
                    .unwrap_or_else(|| format!("[~{:.1}M]", estimate_tier_mbps(*q, None)))
            }
        } else {
            format!("[~{:.1}M]", estimate_tier_mbps(*q, None))
        };

        let lbl = truncate_words(q.label(), name_budget);
        let st = if active && focused {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        q_lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {lbl:<name_budget$} ", if active { ">" } else { " " }),
                st,
            ),
            Span::styled(badge, Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, q_area, row, PlayOptionsHit::SelectQuality(i));
    }
    frame.render_widget(Paragraph::new(q_lines), q_area);

    if show_dividers {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            chunks[ci],
        );
        ci += 1;
    }

    // Codec Priority row (inline pills)
    let codec_pills = vec![
        (
            "[Auto Silicon]".to_string(),
            pill_style(
                codec_prio == CodecPriority::AutoSilicon,
                theme.primary,
                theme.text_dim,
            ),
            PlayOptionsHit::SetCodecPriority(CodecPriority::AutoSilicon),
        ),
        (
            "[VP9 HW]".to_string(),
            pill_style(
                codec_prio == CodecPriority::Vp9HwFirst,
                Color::Rgb(52, 211, 153),
                theme.text_dim,
            ),
            PlayOptionsHit::SetCodecPriority(CodecPriority::Vp9HwFirst),
        ),
        (
            "[AV1 Comp]".to_string(),
            pill_style(
                codec_prio == CodecPriority::Av1First,
                Color::Rgb(251, 191, 36),
                theme.text_dim,
            ),
            PlayOptionsHit::SetCodecPriority(CodecPriority::Av1First),
        ),
    ];
    render_inline_pills(
        frame,
        chunks[ci],
        Some((" Codec [C]:", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))),
        &codec_pills,
        hits,
    );
    ci += 1;

    // FPS Cap row (inline pills)
    let fps_pills = vec![
        (
            "[Auto]".to_string(),
            pill_style(max_fps_cap.is_none(), theme.primary, theme.text_dim),
            PlayOptionsHit::SetMaxFpsCap(None),
        ),
        (
            "[60fps]".to_string(),
            pill_style(max_fps_cap == Some(60), theme.primary, theme.text_dim),
            PlayOptionsHit::SetMaxFpsCap(Some(60)),
        ),
        (
            "[30fps]".to_string(),
            pill_style(max_fps_cap == Some(30), Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::SetMaxFpsCap(Some(30)),
        ),
    ];
    render_inline_pills(
        frame,
        chunks[ci],
        Some((" FPS Cap  :", Style::default().fg(theme.text_dim))),
        &fps_pills,
        hits,
    );
    ci += 1;

    // Data Usage Estimator row
    let mbps_est = estimate_tier_mbps(selected_q, ladder.as_ref());
    let gb_per_hr = mbps_est * 0.45; // Mbps -> GB/hour (Mbps * 3600 / 8000)
    let est_str = if dur_secs > 10.0 {
        let total_mb = (mbps_est * dur_secs) / 8.0;
        let total_lbl = if total_mb >= 1024.0 {
            format!("{:.2} GB", total_mb / 1024.0)
        } else {
            format!("{total_mb:.0} MB")
        };
        format!(
            "Data Est : ~{gb_per_hr:.2} GB/h · ~{total_lbl} ({})",
            format_hms(dur_secs)
        )
    } else {
        format!("Data Est : ~{gb_per_hr:.2} GB/h ({mbps_est:.1} Mbps avg)")
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", truncate_words(&est_str, chunks[ci].width.saturating_sub(1) as usize)),
            Style::default().fg(Color::Rgb(56, 189, 248)),
        )),
        chunks[ci],
    );
    ci += 1;

    if show_dividers {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            chunks[ci],
        );
        ci += 1;
    }

    // Player header
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Player Engine & Capabilities:",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        chunks[ci],
    );
    ci += 1;

    // Player rows (host-OS filtered with live installation badges)
    let p_area = chunks[ci];
    ci += 1;
    let wide_caps = p_area.width >= 48;
    let mut p_lines = Vec::new();
    for &pl in host_players {
        if (p_lines.len() as u16) >= p_area.height {
            break;
        }
        let active = pl == player_now;
        let installed = crate::api::player::is_player_installed(pl);
        let cap_lbl = match (pl, wide_caps) {
            (Player::AutoBest, true) => "Auto libmpv",
            (Player::AutoBest, false) => "Auto",
            (Player::Mpv, true) => "Universal 100%",
            (Player::Mpv, false) => "100%",
            (Player::MpvNet, _) => "Win 99%",
            (Player::Celluloid, _) => "GTK 95%",
        };
        let status_lbl = if installed { "READY" } else { "MISSING" };
        let status_color = if installed {
            Color::Rgb(52, 211, 153)
        } else {
            Color::Rgb(248, 113, 113)
        };
        let row = p_lines.len() as u16;
        let right_w = cap_lbl.len() as u16 + status_lbl.len() as u16 + 10;
        let p_w = p_area.width.saturating_sub(right_w).max(10) as usize;
        let lbl = truncate_words(pl.label(), p_w);
        p_lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {lbl:<p_w$} ", if active { "*" } else { "o" }),
                if active {
                    Style::default()
                        .fg(theme.secondary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            ),
            Span::styled(
                format!("[{cap_lbl}] "),
                Style::default().fg(if active { theme.accent } else { theme.text_dim }),
            ),
            Span::styled(
                format!("[{status_lbl}]"),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        push_line_hit(hits, p_area, row, PlayOptionsHit::SelectPlayer(pl));
    }
    if (p_lines.len() as u16) < p_area.height {
        let host_note = if cfg!(target_os = "macos") {
            "   macOS native libmpv · mpv.net=Win, Celluloid=Linux"
        } else if cfg!(windows) {
            "   Windows native libmpv · Celluloid=Linux"
        } else {
            "   Linux native libmpv · mpv.net=Windows"
        };
        p_lines.push(Line::from(Span::styled(
            truncate_words(host_note, p_area.width as usize),
            Style::default().fg(theme.text_dim),
        )));
    }
    frame.render_widget(Paragraph::new(p_lines), p_area);

    // Bottom tier summary + action & format preview strip (cleanly absorbs extra vertical rows)
    let bot_area = chunks[ci];
    let f_area = if bot_area.height >= 3 {
        let b_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)])
            .split(bot_area);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(bot_area.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            b_split[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        &format!("Selected : {}", selected_q.description()),
                        b_split[1].width.saturating_sub(1) as usize
                    )
                ),
                Style::default().fg(theme.text),
            )),
            b_split[1],
        );
        b_split[2]
    } else {
        bot_area
    };
    let f_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(23), Constraint::Min(10)])
        .split(f_area);
    let btn_rect = Rect::new(
        f_cols[0].x.saturating_add(1),
        f_cols[0].y,
        f_cols[0].width.saturating_sub(1),
        f_cols[0].height,
    );
    render_inline_pills(
        frame,
        btn_rect,
        None,
        &[(
            "[Refresh Ladder (r)]".to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(theme.secondary)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::RefreshStreamLadder,
        )],
        hits,
    );
    let codec_short = match codec_prio.resolve_for_host() {
        CodecPriority::Av1First => "av01",
        CodecPriority::Vp9HwFirst | CodecPriority::AutoSilicon => "vp9",
    };
    let fps_short = max_fps_cap
        .map(|f| format!("{f}p"))
        .unwrap_or_else(|| "auto".to_string());
    let fmt_note = format!(
        "fmt: {}<={}@{} [{}]",
        codec_short,
        selected_q.max_height_or_default(4320),
        fps_short,
        net_mode.short_badge()
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_words(&fmt_note, f_cols[1].width as usize),
            Style::default().fg(theme.text_dim),
        )),
        f_cols[1],
    );

    push_pane_hit(hits, area, 0);
}

fn render_bitrate_ladder_chart(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" Resolved Bitrate Ladder vs Throughput ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 6 {
        return;
    }

    let vid_opt = active_video_id(state);
    let ladder = lookup_ladder(state, vid_opt.as_ref());

    let mut tiers: Vec<(String, String, f64, bool)> = Vec::new();
    let active_h = state.live_playback.current_stream_height;

    if let Some(ref lad) = ladder {
        let mut by_tier: Vec<(u16, String, f64)> = Vec::new();
        for v in &lad.video_variants {
            if let Some(existing) = by_tier.iter_mut().find(|(h, _, _)| *h == v.height) {
                if v.tbr_kbps > existing.2 {
                    *existing = (
                        v.height,
                        v.vcodec.split('.').next().unwrap_or("vid").to_string(),
                        v.tbr_kbps,
                    );
                }
            } else {
                by_tier.push((
                    v.height,
                    v.vcodec.split('.').next().unwrap_or("vid").to_string(),
                    v.tbr_kbps,
                ));
            }
        }
        by_tier.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (h, codec, kbps) in by_tier.into_iter().take(6) {
            tiers.push((
                format!("{h:>4}p"),
                codec,
                kbps / 1000.0,
                active_h > 0 && h == active_h,
            ));
        }
    }

    if tiers.is_empty() {
        let fallbacks = [
            ("2160p", "av01", 22.0),
            ("1440p", "av01", 12.0),
            ("1080p", "vp9 ", 5.2),
            (" 720p", "vp9 ", 2.5),
            (" 480p", "avc1", 1.1),
            (" 360p", "avc1", 0.6),
        ];
        for (res, codec, mbps) in fallbacks {
            let h_num: u16 = res.trim().trim_end_matches('p').parse().unwrap_or(0);
            tiers.push((
                res.to_string(),
                codec.to_string(),
                mbps,
                active_h > 0 && h_num == active_h,
            ));
        }
    }

    let net_mbps = if state.live_playback.cache_speed_bps > 0 {
        Some((state.live_playback.cache_speed_bps as f64 * 8.0) / 1_000_000.0)
    } else {
        None
    };

    let max_mbps = tiers
        .iter()
        .map(|(_, _, m, _)| *m)
        .fold(net_mbps.unwrap_or(10.0), f64::max)
        .max(2.0);
    let show_gph = inner.width >= 52;
    let reserved = if show_gph { 35 } else { 29 };
    let bar_w = (inner.width as usize).saturating_sub(reserved).clamp(8, 32);
    let ceiling_col = net_mbps.map(|m| {
        (((m / max_mbps).clamp(0.0, 1.0) * (bar_w.saturating_sub(1)) as f64).round() as usize)
            .min(bar_w.saturating_sub(1))
    });

    let mut lines = Vec::new();
    let extra_for_header = inner.height >= 10;
    if extra_for_header {
        lines.push(Line::from(Span::styled(
            " Tier  Cod   Throughput vs Bitrate           Rate    Est",
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::BOLD),
        )));
    }
    let header_take = usize::from(extra_for_header);
    let max_bar_rows = (inner.height as usize).saturating_sub(2 + header_take).max(2);
    for (res, codec, mbps, is_active) in tiers.into_iter().take(max_bar_rows) {
        let filled = (((mbps / max_mbps).clamp(0.0, 1.0) * bar_w as f64).round() as usize)
            .min(bar_w);
        let bar_col = if let Some(ceil) = net_mbps {
            if mbps * 1.25 <= ceil {
                Color::Rgb(52, 211, 153)
            } else if mbps <= ceil {
                Color::Rgb(251, 191, 36)
            } else {
                Color::Rgb(248, 113, 113)
            }
        } else if is_active {
            theme.primary
        } else {
            theme.secondary
        };

        let mut spans = vec![Span::styled(
            format!(" {res} {codec:<4} "),
            Style::default()
                .fg(if is_active { theme.primary } else { theme.text })
                .add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() }),
        )];
        for i in 0..bar_w {
            if ceiling_col == Some(i) {
                spans.push(Span::styled(
                    "│",
                    Style::default()
                        .fg(Color::Rgb(56, 189, 248))
                        .add_modifier(Modifier::BOLD),
                ));
            } else if i < filled {
                spans.push(Span::styled("█", Style::default().fg(bar_col)));
            } else {
                spans.push(Span::styled("░", Style::default().fg(theme.divider)));
            }
        }
        let active_tag = if is_active { " [ACT]" } else { "" };
        let gph = if show_gph {
            format!(" {:>4.1}G/h", mbps * 0.45)
        } else {
            String::new()
        };
        spans.push(Span::styled(
            format!(" {:>4.1}M{gph}{active_tag}", mbps),
            Style::default().fg(if is_active { theme.primary } else { theme.text_dim }),
        ));
        lines.push(Line::from(spans));
    }

    // Audio Stream Quality Comparison Bar
    if (lines.len() as u16) + 2 <= inner.height {
        let (opus_k, aac_k) = if let Some(ref lad) = ladder {
            let opus = lad
                .audio_variants
                .iter()
                .filter(|a| a.acodec.contains("opus"))
                .map(|a| a.abr_kbps)
                .fold(0.0_f64, f64::max);
            let aac = lad
                .audio_variants
                .iter()
                .filter(|a| a.acodec.contains("mp4a") || a.acodec.contains("aac"))
                .map(|a| a.abr_kbps)
                .fold(0.0_f64, f64::max);
            (
                if opus > 1.0 { opus } else { 160.0 },
                if aac > 1.0 { aac } else { 128.0 },
            )
        } else {
            (160.0, 128.0)
        };
        let o_bars = ((opus_k / 200.0).clamp(0.1, 1.0) * 8.0).round() as usize;
        let a_bars = ((aac_k / 200.0).clamp(0.1, 1.0) * 8.0).round() as usize;
        lines.push(Line::from(vec![
            Span::styled(" Audio: ", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)),
            Span::styled(format!("Opus {opus_k:.0}k "), Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(format!("{}{}", "█".repeat(o_bars), "░".repeat(8 - o_bars)), Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled("  │  ", Style::default().fg(theme.divider)),
            Span::styled(format!("AAC {aac_k:.0}k "), Style::default().fg(theme.text_dim)),
            Span::styled(format!("{}{}", "█".repeat(a_bars), "░".repeat(8 - a_bars)), Style::default().fg(theme.secondary)),
        ]));
    }

    // Footer + Headroom Verdict
    if (lines.len() as u16) < inner.height {
        let sel_q = Quality::ALL
            .get(state.play_quality_idx)
            .copied()
            .unwrap_or(state.config.quality);
        let tier_need = estimate_tier_mbps(sel_q, ladder.as_ref());
        if let Some(mbps) = net_mbps {
            let ratio = mbps / tier_need.max(0.2);
            let (verdict, vcol) = if ratio >= 1.5 {
                (format!("[SAFE {ratio:.1}x]"), Color::Rgb(52, 211, 153))
            } else if ratio >= 1.0 {
                (format!("[MARGINAL {ratio:.1}x]"), Color::Rgb(251, 191, 36))
            } else {
                (format!("[STALL RISK {ratio:.1}x]"), Color::Rgb(248, 113, 113))
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        " │ Ceil: {:.1}M ({}) · ",
                        mbps,
                        format_speed_bps(state.live_playback.cache_speed_bps)
                    ),
                    Style::default().fg(Color::Rgb(56, 189, 248)),
                ),
                Span::styled(verdict, Style::default().fg(vcol).add_modifier(Modifier::BOLD)),
            ]));
        } else {
            let txt = format!(
                "Source: {} · Need ~{:.1}M ({} headroom)",
                if ladder.is_some() { "Resolved yt-dlp" } else { "Estimated Profile" },
                tier_need * 1.25,
                if state.has_live_ipc() { "live" } else { "probes on launch" }
            );
            lines.push(Line::from(Span::styled(
                format!(" {}", truncate_words(&txt, inner.width.saturating_sub(1) as usize)),
                Style::default().fg(Color::Rgb(56, 189, 248)),
            )));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_network_shield_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Network Resilience, Cache & Session Auth ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 8 {
        return;
    }

    let nm = state.config.network_mode;
    let auto_abr = state.config.auto_abr_downshift;
    let force_v4 = state.config.force_ipv4;
    let eff_master = state.config.effective_mastering();
    let stalls = state.live_playback.stall_count;
    let authed = state.auth_vault.active_account().is_some();
    let cookies_ok = state
        .config
        .cookies_path()
        .map(|p| p.exists())
        .unwrap_or(false);
    let vid_opt = active_video_id(state);
    let offline_cached = vid_opt
        .as_ref()
        .map(|id| state.offline_paths.contains_key(id))
        .unwrap_or(false);
    let ladder_cached = lookup_ladder(state, vid_opt.as_ref()).is_some();

    let compact = inner.height < 11;
    let mut constraints = vec![
        Constraint::Length(1), // 0: Congestion pills
        Constraint::Length(1), // 1: Auto-ABR
        Constraint::Length(1), // 2: Force IPv4
        Constraint::Length(1), // 3: Buffer Readahead / Pause Wait
        Constraint::Length(1), // 4: Cache Mode / Stalls
        Constraint::Length(1), // 5: Demux Ring / Timeout
    ];
    if !compact {
        constraints.extend([
            Constraint::Length(1), // Divider
            Constraint::Length(1), // Auth Vault & Cookie Jar
            Constraint::Length(1), // Playback Source & Ladder
            Constraint::Min(1),    // Action Pills
        ]);
    } else {
        constraints.push(Constraint::Min(1));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // 0: Inline Congestion Profile Pills
    let net_pills = vec![
        (
            "[Full Speed]".to_string(),
            pill_style(nm == NetworkMode::Normal, theme.primary, theme.text_dim),
            PlayOptionsHit::SetNetworkMode(NetworkMode::Normal),
        ),
        (
            "[Wi-Fi 720p]".to_string(),
            pill_style(
                nm == NetworkMode::CongestedWifi,
                Color::Rgb(251, 191, 36),
                theme.text_dim,
            ),
            PlayOptionsHit::SetNetworkMode(NetworkMode::CongestedWifi),
        ),
        (
            "[Eco 480p]".to_string(),
            pill_style(
                nm == NetworkMode::UltraSaver,
                Color::Rgb(52, 211, 153),
                theme.text_dim,
            ),
            PlayOptionsHit::SetNetworkMode(NetworkMode::UltraSaver),
        ),
    ];
    render_inline_pills(
        frame,
        rows[0],
        Some((" Shield [w]:", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
        &net_pills,
        hits,
    );

    // 1: Auto-ABR Downshift
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Auto-ABR Downshift [A]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if auto_abr {
                    "ON (Guards against stalls)"
                } else {
                    "OFF (Locked manual tier)"
                },
                Style::default()
                    .fg(if auto_abr { Color::Rgb(52, 211, 153) } else { theme.text_dim })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[1],
    );
    hits.push((rows[1], PlayOptionsHit::ToggleAutoAbr));

    // 2: Force IPv4
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Force IPv4 Socket  [i]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if force_v4 { "ON (IPv4 Only)" } else { "OFF (Dual-Stack IPv4/v6)" },
                Style::default()
                    .fg(if force_v4 { Color::Rgb(251, 191, 36) } else { theme.text })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[2],
    );
    hits.push((rows[2], PlayOptionsHit::ToggleForceIpv4));

    // 3: Buffer Readahead & Pause Wait
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " {}",
                truncate_words(
                    &format!(
                        "Readahead  : {}s VOD   │ Pause Wait: {:.1}s",
                        nm.vod_readahead_secs(),
                        nm.vod_cache_pause_wait_secs()
                    ),
                    rows[3].width.saturating_sub(1) as usize,
                )
            ),
            Style::default().fg(theme.text),
        )),
        rows[3],
    );

    // 4: Cache Mode & Stalls
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " {}",
                truncate_words(
                    &format!(
                        "Cache Mode : {}   │ Stalls    : {}",
                        if nm.use_disk_cache() { "Disk + RAM" } else { "RAM (Zero Disk)" },
                        stalls
                    ),
                    rows[4].width.saturating_sub(1) as usize,
                )
            ),
            Style::default().fg(theme.text),
        )),
        rows[4],
    );

    // 5: Demux Ring & Timeout
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " {}",
                truncate_words(
                    &format!(
                        "Demux Ring : {} │ Timeout: {}s",
                        eff_master.demuxer_cache.label(),
                        nm.network_timeout_secs()
                    ),
                    rows[5].width.saturating_sub(1) as usize,
                )
            ),
            Style::default().fg(theme.text_dim),
        )),
        rows[5],
    );

    if compact {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        &format!(
                            "Auth: {} | Cookies: {} | Ladder: {}",
                            if authed { "SignedIn" } else { "Guest" },
                            if cookies_ok { "Ready" } else { "None" },
                            if ladder_cached { "Cached" } else { "Est" }
                        ),
                        rows[6].width.saturating_sub(1) as usize,
                    )
                ),
                Style::default().fg(theme.text_dim),
            )),
            rows[6],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            rows[6],
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Auth Vault : ", Style::default().fg(theme.text_dim)),
                Span::styled(
                    if authed { "Signed In" } else { "Guest Mode" },
                    Style::default().fg(if authed {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.text
                    }),
                ),
                Span::styled(" │ Cookies: ", Style::default().fg(theme.text_dim)),
                Span::styled(
                    if cookies_ok { "Isolated Jar Ready" } else { "None" },
                    Style::default().fg(if cookies_ok { theme.secondary } else { theme.text_dim }),
                ),
            ])),
            rows[7],
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Media Src  : ", Style::default().fg(theme.text_dim)),
                Span::styled(
                    if offline_cached { "Offline Vault File" } else { "HTTPS Stream" },
                    Style::default().fg(if offline_cached { theme.accent } else { theme.primary }),
                ),
                Span::styled(" │ Ladder: ", Style::default().fg(theme.text_dim)),
                Span::styled(
                    if ladder_cached { "Resolved" } else { "Estimated" },
                    Style::default().fg(theme.text),
                ),
            ])),
            rows[8],
        );
        let action_pills = vec![
            (
                "[Enter Launch / Apply]".to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
                PlayOptionsHit::LaunchOrRelaunch,
            ),
            (
                "[r Re-Probe Formats]".to_string(),
                Style::default().fg(theme.secondary),
                PlayOptionsHit::RefreshStreamLadder,
            ),
        ];
        let act_rect = Rect::new(
            rows[9].x.saturating_add(1),
            rows[9].y,
            rows[9].width.saturating_sub(1),
            rows[9].height,
        );
        render_inline_pills(frame, act_rect, None, &action_pills, hits);
    }

    push_pane_hit(hits, area, 1);
}
