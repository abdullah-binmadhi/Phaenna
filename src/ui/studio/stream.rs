//! Page 2 — `[2] Stream & Net`: Quality Ladder, Bitrate-vs-Throughput Bar Graph,
//! Player Engine Capabilities, and Network Resilience Shield.

use crate::config::{NetworkMode, Player, Quality};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_speed_bps, push_line_hit, push_pane_hit, section_block, truncate_words,
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
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
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

fn render_quality_player_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Stream Quality & Player Backend ", focused, &theme);
    let inner = blk.inner(area);
    if inner.width < 20 || inner.height < 10 {
        frame.render_widget(blk, area);
        return;
    }

    let vid_opt = active_video_id(state);
    let ladder = vid_opt.as_ref().and_then(|id| {
        let clean = id.trim_start_matches("RD");
        state
            .stream_ladder_cache
            .get(id)
            .or_else(|| state.stream_ladder_cache.get(clean))
            .cloned()
    });

    let mut lines = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;
    let name_budget = inner.width.saturating_sub(22).max(12) as usize;

    lines.push(Line::from(Span::styled(
        " Target Resolution Tier:",
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    )));

    for (i, q) in Quality::ALL.iter().enumerate() {
        let active = i == state.play_quality_idx;
        let row = lines.len() as u16;
        let badge = if let Some(ref lad) = ladder {
            let max_h = q.max_height_or_default(4320);
            if *q == Quality::AudioOnly {
                lad.audio_variants
                    .first()
                    .map(|a| format!("[{}] {:>3.0}K", a.acodec.split('.').next().unwrap_or("opus"), a.abr_kbps))
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
                    .unwrap_or_default()
            }
        } else {
            String::new()
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
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {lbl:<name_budget$} ", if active { ">" } else { " " }),
                st,
            ),
            Span::styled(badge, Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectQuality(i));
    }

    if (lines.len() as u16) + 1 < inner.height {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        " Player Engine & Capabilities:",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));

    let players = [
        (Player::Mpv, "[IPC] [HW] [GLSL] [SB]"),
        (Player::Iina, "[GLSL] [macOS Native]"),
        (Player::Vlc, "[Wide Codec] [Net Buf]"),
        (Player::Browser, "[Zero Config]"),
    ];
    for (pl, caps) in players {
        if (lines.len() as u16) >= inner.height {
            break;
        }
        let active = pl == state.config.player;
        let row = lines.len() as u16;
        let p_w = inner.width.saturating_sub(caps.len() as u16 + 6).max(10) as usize;
        let lbl = truncate_words(pl.label(), p_w);
        lines.push(Line::from(vec![
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
                caps,
                Style::default().fg(if active { theme.accent } else { theme.text_dim }),
            ),
        ]));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectPlayer(pl));
    }

    push_pane_hit(hits, area, 0);
    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_bitrate_ladder_chart(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" Resolved Bitrate Ladder vs Throughput ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 4 {
        return;
    }

    let vid_opt = active_video_id(state);
    let ladder = vid_opt.as_ref().and_then(|id| {
        let clean = id.trim_start_matches("RD");
        state
            .stream_ladder_cache
            .get(id)
            .or_else(|| state.stream_ladder_cache.get(clean))
    });

    let mut tiers: Vec<(String, String, f64, bool)> = Vec::new();
    let active_h = state.live_playback.current_stream_height;

    if let Some(lad) = ladder {
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
    let bar_w = (inner.width as usize).saturating_sub(29).clamp(8, 34);
    let ceiling_col = net_mbps.map(|m| {
        (((m / max_mbps).clamp(0.0, 1.0) * (bar_w.saturating_sub(1)) as f64).round() as usize)
            .min(bar_w.saturating_sub(1))
    });

    let mut lines = Vec::new();
    for (res, codec, mbps, is_active) in tiers {
        if (lines.len() as u16) + 1 >= inner.height {
            break;
        }
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
        spans.push(Span::styled(
            format!(" {:>4.1}M{active_tag}", mbps),
            Style::default().fg(if is_active { theme.primary } else { theme.text_dim }),
        ));
        lines.push(Line::from(spans));
    }

    let footer = if let Some(mbps) = net_mbps {
        format!(
            " │ Throughput Ceiling: {:.1} Mbps ({})  (Ladder: {})",
            mbps,
            format_speed_bps(state.live_playback.cache_speed_bps),
            if ladder.is_some() { "Resolved" } else { "Estimated" }
        )
    } else {
        format!(
            " Ladder Source: {}  (Throughput probes once mpv starts)",
            if ladder.is_some() { "Resolved yt-dlp" } else { "Estimated Profile" }
        )
    };
    lines.push(Line::from(Span::styled(
        truncate_words(&footer, inner.width as usize),
        Style::default().fg(Color::Rgb(56, 189, 248)),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_network_shield_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Network Resilience & Congestion Shield ", focused, &theme);
    let inner = blk.inner(area);
    if inner.width < 22 || inner.height < 6 {
        frame.render_widget(blk, area);
        return;
    }

    let mut lines = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;

    lines.push(Line::from(Span::styled(
        " Congestion Profile [w]:",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));

    for nm in NetworkMode::ALL {
        let active = nm == state.config.network_mode;
        let row = lines.len() as u16;
        lines.push(Line::from(Span::styled(
            format!(" {} {}", if active { "(*)" } else { "( )" }, nm.label()),
            if active {
                Style::default()
                    .fg(Color::Rgb(251, 191, 36))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )));
        push_line_hit(hits, inner, row, PlayOptionsHit::CycleNetworkMode);
    }

    if (lines.len() as u16) + 1 < inner.height {
        lines.push(Line::from(""));
    }

    if (lines.len() as u16) < inner.height {
        let row_abr = lines.len() as u16;
        lines.push(Line::from(vec![
            Span::styled(" Auto-ABR Downshift [A]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.auto_abr_downshift { "ON" } else { "OFF" },
                Style::default()
                    .fg(if state.config.auto_abr_downshift {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.text_dim
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        push_line_hit(hits, inner, row_abr, PlayOptionsHit::ToggleAutoAbr);
    }

    if (lines.len() as u16) < inner.height {
        let row_v4 = lines.len() as u16;
        lines.push(Line::from(vec![
            Span::styled(" Force IPv4 Socket  [i]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.force_ipv4 { "ON (IPv4 Only)" } else { "OFF (Dual-Stack)" },
                Style::default()
                    .fg(if state.config.force_ipv4 {
                        Color::Rgb(251, 191, 36)
                    } else {
                        theme.text
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        push_line_hit(hits, inner, row_v4, PlayOptionsHit::ToggleForceIpv4);
    }

    if (lines.len() as u16) < inner.height {
        let nm = state.config.network_mode;
        lines.push(Line::from(Span::styled(
            format!(
                " Readahead: {}s | Pause Wait: {:.1}s | Disk Cache: {} | Stalls: {}",
                nm.vod_readahead_secs(),
                nm.vod_cache_pause_wait_secs(),
                if nm.use_disk_cache() { "ON" } else { "RAM" },
                state.live_playback.stall_count
            ),
            Style::default().fg(theme.text_dim),
        )));
    }

    push_pane_hit(hits, area, 1);
    frame.render_widget(Paragraph::new(lines).block(blk), area);
}
