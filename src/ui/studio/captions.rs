//! Page 5 — `[5] Captions & Skip`: Subtitles, Live Typography Styler Preview,
//! SponsorBlock Guard, Visual Timeline Map & Clickable Segment Jumps.

use crate::api::sponsorblock::category_label;
use crate::config::SponsorBlockMode;
use crate::services::substyler::{SubtitlePreset, SubtitleStyleConfig};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_hms, push_line_hit, push_pane_hit, section_block, sponsor_color,
    sponsor_timeline_spans, truncate_words,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub fn render_captions_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 32 || area.height < 12 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);

    render_subtitles_preview_pane(frame, cols[0], state);
    render_sponsorblock_timeline_pane(frame, cols[1], state);
}

fn parse_hex_color(hex: &str, fallback: Color) -> Color {
    let s = hex.trim().trim_start_matches('#');
    if s.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return Color::Rgb(r, g, b);
        }
    }
    fallback
}

fn render_subtitles_preview_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Subtitles & Live Typography Preview ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 10 {
        return;
    }

    let live = state.has_live_ipc();
    let sub_on = if live {
        state.live_playback.sub_visible
    } else {
        state.config.subtitles_enabled
    };
    let hits = &mut state.live_playback.play_hitboxes;

    let track_h = inner.height.saturating_sub(11).clamp(3, 6);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),       // 0: Subtitles ON/OFF + Collision Avoidance
            Constraint::Length(1),       // 1: Sub Delay controls
            Constraint::Length(1),       // 2: Sub Tracks header
            Constraint::Length(track_h), // 3: Scrollable Sub Tracks
            Constraint::Length(1),       // 4: Preset header
            Constraint::Length(1),       // 5: Preset pills
            Constraint::Length(1),       // 6: Font/Size/Border spec
            Constraint::Min(4),          // 7: Live Typography Preview Frame
        ])
        .split(inner);

    // Row 0: Subtitles ON/OFF & \an8 Collision
    let r0_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Subtitles [c]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if sub_on { "ON " } else { "OFF" },
                Style::default()
                    .fg(if sub_on { theme.accent } else { theme.text_dim })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        r0_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Collision [\\an8]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.subtitle_style.collision_avoidance {
                    "ON"
                } else {
                    "OFF"
                },
                Style::default()
                    .fg(if state.config.subtitle_style.collision_avoidance {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.text_dim
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        r0_cols[1],
    );
    hits.push((r0_cols[0], PlayOptionsHit::ToggleSubtitles));
    hits.push((r0_cols[1], PlayOptionsHit::ToggleSubCollision));

    // Row 1: Sub Offset controls
    let d_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(15),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(6),
        ])
        .split(rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Sub Offset: {:+}ms", state.live_playback.sub_delay_ms),
            Style::default().fg(theme.text),
        )),
        d_cols[0],
    );
    let sub_steps = [
        ("[-500]", PlayOptionsHit::SubDelayDelta(-500)),
        ("[-100]", PlayOptionsHit::SubDelayDelta(-100)),
        ("[ 0 ]", PlayOptionsHit::SetSubDelay(0)),
        ("[+100]", PlayOptionsHit::SubDelayDelta(100)),
        ("[+500]", PlayOptionsHit::SubDelayDelta(500)),
    ];
    for (idx, (lbl, hit)) in sub_steps.into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(lbl, Style::default().fg(theme.secondary))),
            d_cols[idx + 1],
        );
        hits.push((d_cols[idx + 1], hit));
    }

    // Row 2 & 3: Live Sub Tracks
    let total_subs = state.live_playback.sub_tracks.len();
    frame.render_widget(
        Paragraph::new(Span::styled(
            if total_subs > 0 {
                format!(" Live Subtitle Tracks ({total_subs}):")
            } else {
                " Subtitle Tracks (Populates on Launch):".to_string()
            },
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[2],
    );

    let mut tr_lines = Vec::new();
    if total_subs == 0 {
        tr_lines.push(Line::from(Span::styled(
            format!("   Default Language: {}", state.config.default_sub_lang),
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = track_h as usize;
        let scroll = state
            .studio
            .sub_track_scroll
            .min(total_subs.saturating_sub(cap));
        for tr in state
            .live_playback
            .sub_tracks
            .iter()
            .skip(scroll)
            .take(cap)
            .cloned()
        {
            let active = state.live_playback.active_sid == Some(tr.id) || tr.selected;
            let row = tr_lines.len() as u16;
            let max_w = rows[3].width.saturating_sub(4) as usize;
            let lbl = truncate_words(&tr.display_label(), max_w);
            tr_lines.push(Line::from(Span::styled(
                format!(" {} {lbl:<max_w$} ", if active { ">" } else { " " }),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            )));
            push_line_hit(hits, rows[3], row, PlayOptionsHit::SelectSubTrack(tr.id));
        }
    }
    frame.render_widget(Paragraph::new(tr_lines), rows[3]);

    // Row 4 & 5: Style Preset Selector
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Typography Preset [y] (libass):",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[4],
    );
    let presets = [
        (SubtitlePreset::StudioMinimal, "Studio Min"),
        (SubtitlePreset::NetflixClean, "Netflix"),
        (SubtitlePreset::CinemaGold, "Cinema Gold"),
        (SubtitlePreset::OledPlate, "OLED Plate"),
    ];
    let p_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(23),
            Constraint::Percentage(26),
            Constraint::Percentage(25),
        ])
        .split(rows[5]);
    for (idx, (preset, lbl)) in presets.into_iter().enumerate() {
        let active = state.config.subtitle_style.preset == preset;
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" [{lbl}]"),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.secondary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
            )),
            p_cols[idx],
        );
        hits.push((p_cols[idx], PlayOptionsHit::SelectSubPreset(preset)));
    }

    // Row 6: Font / Border / Margin specs
    let st_cfg: &SubtitleStyleConfig = &state.config.subtitle_style;
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_words(
                &format!(
                    " Font: {} · Size:{} · Border:{:.1}px · MarginY:{}",
                    st_cfg.font_family, st_cfg.font_size, st_cfg.border_size, st_cfg.margin_y
                ),
                rows[6].width as usize,
            ),
            Style::default().fg(theme.text_dim),
        )),
        rows[6],
    );

    // Row 7: Live Typography Preview Frame
    let prev_blk = Block::default()
        .title(Span::styled(
            format!(" Live Preview ({}) ", st_cfg.preset.label()),
            Style::default().fg(theme.accent),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.divider));
    let prev_inner = prev_blk.inner(rows[7]);
    frame.render_widget(prev_blk, rows[7]);
    if prev_inner.width > 8 && prev_inner.height >= 2 {
        let fg_col = parse_hex_color(&st_cfg.primary_color, Color::White);
        let mut text_style = Style::default().fg(fg_col).add_modifier(Modifier::BOLD);
        if st_cfg.backdrop_opacity > 0.05 {
            text_style = text_style.bg(Color::Rgb(16, 18, 24));
        }
        let sample_1 = if sub_on {
            "\"Shooting with an old MFT camera\""
        } else {
            "(Subtitles currently hidden — press [c] to enable)"
        };
        let sample_2 = if sub_on {
            "still delivers rich cinema color."
        } else {
            ""
        };
        let style_to_use = if sub_on {
            text_style
        } else {
            Style::default().fg(theme.text_dim)
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    truncate_words(sample_1, prev_inner.width as usize),
                    style_to_use,
                )),
                Line::from(Span::styled(
                    truncate_words(sample_2, prev_inner.width as usize),
                    style_to_use,
                )),
            ])
            .alignment(Alignment::Center),
            prev_inner,
        );
    }

    push_pane_hit(hits, area, 0);
}

fn render_sponsorblock_timeline_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" SponsorBlock Guard & Timeline Map ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 10 {
        return;
    }

    let dur = if state.active_playback_dur > 1.0 {
        state.active_playback_dur
    } else {
        state
            .active_selected_video()
            .and_then(|v| v.duration)
            .unwrap_or(600) as f64
    };
    let pos = state.active_playback_pos;
    let hits = &mut state.live_playback.play_hitboxes;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Mode row 1
            Constraint::Length(1), // 1: Mode row 2
            Constraint::Length(1), // 2: Skip Outros + Saved summary
            Constraint::Length(1), // 3: Timeline header
            Constraint::Length(1), // 4: Visual SponsorBlock Timeline Bar
            Constraint::Length(1), // 5: Legend
            Constraint::Min(4),    // 6: Segment List
        ])
        .split(inner);

    // Rows 0 & 1: 2x2 SponsorBlockMode selector
    let modes_r0 = [SponsorBlockMode::AutoSkipAll, SponsorBlockMode::SponsorOnly];
    let modes_r1 = [SponsorBlockMode::ShowChaptersOnly, SponsorBlockMode::Off];
    for (row_idx, pair) in [modes_r0, modes_r1].into_iter().enumerate() {
        let m_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[row_idx]);
        for (c_idx, mode) in pair.into_iter().enumerate() {
            let active = state.config.sponsorblock_mode == mode;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!(" {} {}", if active { "(*)" } else { "( )" }, mode.label()),
                    if active {
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                )),
                m_cols[c_idx],
            );
            hits.push((m_cols[c_idx], PlayOptionsHit::SelectSponsorBlock(mode)));
        }
    }

    // Row 2: Skip Outros + Skips session count
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Skip Outros [o]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.skip_outro {
                    "ON"
                } else {
                    "OFF (Protected)"
                },
                Style::default()
                    .fg(if state.config.skip_outro {
                        Color::Rgb(251, 191, 36)
                    } else {
                        Color::Rgb(52, 211, 153)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  |  Session Skips: {}", state.sponsor_skips_session),
                Style::default().fg(theme.secondary),
            ),
        ])),
        rows[2],
    );
    hits.push((rows[2], PlayOptionsHit::ToggleSkipOutro));

    // Row 3 & 4: Visual Timeline Map
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " Timeline Distribution (00:00 .. {} · ▼ Playhead):",
                format_hms(dur)
            ),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[3],
    );

    let bar_w = rows[4].width.saturating_sub(4).max(8) as usize;
    let mut t_spans = vec![Span::styled(" ├", Style::default().fg(theme.text_dim))];
    t_spans.extend(sponsor_timeline_spans(
        &state.detail_sponsor_segments,
        pos,
        dur,
        bar_w,
        &theme,
    ));
    t_spans.push(Span::styled("┤ ", Style::default().fg(theme.text_dim)));
    frame.render_widget(Paragraph::new(Line::from(t_spans)), rows[4]);
    if dur > 1.0 {
        hits.push((rows[4], PlayOptionsHit::SeekRatio(-1.0)));
    }

    // Row 5: Legend
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" █ ", Style::default().fg(sponsor_color("sponsor"))),
            Span::styled("Sponsor  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("selfpromo"))),
            Span::styled("SelfPromo  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("intro"))),
            Span::styled("Intro  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("outro"))),
            Span::styled("Outro", Style::default().fg(theme.text_dim)),
        ])),
        rows[5],
    );

    // Row 6: Clickable Detected Segments
    let mut seg_lines = Vec::new();
    let total_segs = state.detail_sponsor_segments.len();
    if total_segs == 0 {
        seg_lines.push(Line::from(Span::styled(
            " No SponsorBlock segments detected for this video.",
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = rows[6].height as usize;
        let cursor = state.studio.sponsor_cursor.min(total_segs - 1);
        let scroll = state
            .studio
            .sponsor_scroll
            .min(total_segs.saturating_sub(cap));
        for (rel_idx, seg) in state
            .detail_sponsor_segments
            .iter()
            .skip(scroll)
            .take(cap)
            .enumerate()
        {
            let abs_idx = scroll + rel_idx;
            let row = seg_lines.len() as u16;
            let selected = focused && abs_idx == cursor;
            let len_s = (seg.end_time - seg.start_time).max(0.0);
            let status = if seg.category == "outro" && !state.config.skip_outro {
                "[Protected]"
            } else if !state
                .config
                .sponsorblock_mode
                .auto_skips_category_with_outro(&seg.category, state.config.skip_outro)
            {
                "[BadgeOnly]"
            } else if pos >= seg.end_time {
                "[Skipped ✓]"
            } else if pos >= seg.start_time && pos < seg.end_time {
                "[Active]   "
            } else {
                "[Queued]   "
            };
            let cat_lbl = category_label(&seg.category);
            let cat_w = rows[6].width.saturating_sub(36).max(8) as usize;
            let txt = format!(
                " {} {}–{} ({:>3.0}s) {:<cat_w$} {}",
                if selected { ">" } else { " " },
                format_hms(seg.start_time),
                format_hms(seg.end_time),
                len_s,
                truncate_words(cat_lbl, cat_w),
                status
            );
            seg_lines.push(Line::from(Span::styled(
                txt,
                if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(sponsor_color(&seg.category))
                },
            )));
            push_line_hit(
                hits,
                rows[6],
                row,
                PlayOptionsHit::JumpSponsorSegment(seg.start_time),
            );
        }
    }
    frame.render_widget(Paragraph::new(seg_lines), rows[6]);
    push_pane_hit(hits, area, 1);
}
