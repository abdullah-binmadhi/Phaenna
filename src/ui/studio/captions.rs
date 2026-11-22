//! Page 5 — `[5] Captions & Skip`: Subtitles, Language Quick-Picker, Live Typography Fine-Tuner,
//! SponsorBlock 7-Category Guard Matrix, Chapter-Marked Timeline, and Dual Segments/Chapters Jump List.

use crate::api::sponsorblock::category_label;
use crate::config::SponsorBlockMode;
use crate::services::substyler::{SubtitlePreset, SubtitleStyleConfig};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_hms, push_line_hit, push_pane_hit, render_inline_pills, section_block, sponsor_color,
    sponsor_timeline_with_chapters_spans, truncate_words,
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

fn render_subtitles_preview_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Subtitles, Language & Typography Studio ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 11 {
        return;
    }

    let live = state.has_live_ipc();
    let sub_on = if live {
        state.live_playback.sub_visible
    } else {
        state.config.subtitles_enabled
    };
    let collision_on = state.config.subtitle_style.collision_avoidance;
    let sub_delay_ms = state.live_playback.sub_delay_ms;
    let def_lang = state.config.default_sub_lang.clone();
    let st_cfg: SubtitleStyleConfig = state.config.subtitle_style.clone();
    let total_subs = state.live_playback.sub_tracks.len();
    let sub_scroll = state.studio.sub_track_scroll;
    let sub_tracks = state.live_playback.sub_tracks.clone();
    let active_sid = state.live_playback.active_sid;

    let show_tuner = inner.height >= 15;
    let track_h: u16 = if inner.height >= 19 { 3 } else { 2 };
    let preview_h: u16 = if inner.height >= 20 { 6 } else { 4 };

    let mut constraints = vec![
        Constraint::Length(1),         // 0: Subtitles ON/OFF + Collision
        Constraint::Length(1),         // 1: Sub Offset pills
        Constraint::Length(1),         // 2: Default Lang pills
        Constraint::Length(1),         // 3: Tracks header
        Constraint::Length(track_h),   // 4: Sub tracks list
        Constraint::Length(1),         // 5: Typography header
        Constraint::Length(1),         // 6: Preset pills
    ];
    if show_tuner {
        constraints.extend([
            Constraint::Length(1), // Fine-tuner row 1: Font Size + Border
            Constraint::Length(1), // Fine-tuner row 2: MarginY + Backdrop
            Constraint::Length(1), // Fine-tuner row 3: Text Ink
        ]);
    }
    constraints.push(Constraint::Min(preview_h)); // Framed live preview box

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ri = 0usize;

    // 0: Subtitles ON/OFF & \an8 Collision
    let r0_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[ri]);
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
                if collision_on { "ON" } else { "OFF" },
                Style::default()
                    .fg(if collision_on {
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
    ri += 1;

    // 1: Sub Offset controls via render_inline_pills (fixes `][` bug!)
    let offset_prefix = format!(" Offset {:+}ms:", sub_delay_ms);
    let sub_steps = vec![
        ("[-500]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubDelayDelta(-500)),
        ("[-100]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubDelayDelta(-100)),
        (
            "[0ms]".to_string(),
            pill_style(sub_delay_ms == 0, theme.primary, theme.secondary),
            PlayOptionsHit::SetSubDelay(0),
        ),
        ("[+100]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubDelayDelta(100)),
        ("[+500]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubDelayDelta(500)),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((&offset_prefix, Style::default().fg(theme.text))),
        &sub_steps,
        hits,
    );
    ri += 1;

    // 2: Default Language Quick-Picker
    let def_lower = def_lang.to_lowercase();
    let lang_pills = ["en", "ar", "ja", "es", "fr", "de"]
        .into_iter()
        .map(|code| {
            let active = def_lower == code
                || (code == "en" && def_lower.starts_with("english"))
                || (code == "ar" && def_lower.starts_with("arab"));
            (
                format!("[{code}]"),
                pill_style(active, theme.accent, theme.text_dim),
                PlayOptionsHit::SetDefaultSubLang(code),
            )
        })
        .collect::<Vec<_>>();
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Default Lang:", Style::default().fg(theme.text_dim))),
        &lang_pills,
        hits,
    );
    ri += 1;

    // 3 & 4: Live Sub Tracks
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
        rows[ri],
    );
    ri += 1;

    let tr_area = rows[ri];
    ri += 1;
    let mut tr_lines = Vec::new();
    if total_subs == 0 {
        tr_lines.push(Line::from(Span::styled(
            format!("   Default Language: {} (libass + \\an8 ready)", def_lang),
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = track_h as usize;
        let scroll = sub_scroll.min(total_subs.saturating_sub(cap));
        for tr in sub_tracks.iter().skip(scroll).take(cap) {
            let active = active_sid == Some(tr.id) || tr.selected;
            let row = tr_lines.len() as u16;
            let max_w = tr_area.width.saturating_sub(4) as usize;
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
            push_line_hit(hits, tr_area, row, PlayOptionsHit::SelectSubTrack(tr.id));
        }
    }
    frame.render_widget(Paragraph::new(tr_lines), tr_area);

    // 5 & 6: Style Preset Selector
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " Typography Preset [y] (libass · {}):",
                st_cfg.font_family
            ),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    let mut preset_pills: Vec<(String, Style, PlayOptionsHit)> = [
        (SubtitlePreset::StudioMinimal, "Studio Min"),
        (SubtitlePreset::NetflixClean, "Netflix"),
        (SubtitlePreset::CinemaGold, "Cinema Gold"),
        (SubtitlePreset::OledPlate, "OLED"),
    ]
    .into_iter()
    .map(|(preset, lbl)| {
        (
            format!("[{lbl}]"),
            pill_style(st_cfg.preset == preset, theme.secondary, theme.text_dim),
            PlayOptionsHit::SelectSubPreset(preset),
        )
    })
    .collect();
    if st_cfg.preset == SubtitlePreset::Custom {
        preset_pills.push((
            "[Custom*]".to_string(),
            pill_style(true, theme.accent, theme.text),
            PlayOptionsHit::SelectSubPreset(SubtitlePreset::StudioMinimal),
        ));
    }
    render_inline_pills(frame, rows[ri], Some((" ", Style::default())), &preset_pills, hits);
    ri += 1;

    // Live Typography Fine-Tuner Rows
    if show_tuner {
        // Row A: Font Size + Border Size
        let fine_a = vec![
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubFontSizeDelta(-2)),
            (format!("{}px", st_cfg.font_size), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubFontSizeDelta(0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubFontSizeDelta(2)),
            ("Border:".to_string(), Style::default().fg(theme.text_dim), PlayOptionsHit::SubBorderDelta(0.0)),
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubBorderDelta(-0.4)),
            (format!("{:.1}", st_cfg.border_size), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubBorderDelta(0.0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubBorderDelta(0.4)),
        ];
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Font:", Style::default().fg(theme.text_dim))),
            &fine_a,
            hits,
        );
        ri += 1;

        // Row B: MarginY + Backdrop
        let op = st_cfg.backdrop_opacity;
        let fine_b = vec![
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubMarginYDelta(-4)),
            (format!("{}px", st_cfg.margin_y), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubMarginYDelta(0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubMarginYDelta(4)),
            ("BG:".to_string(), Style::default().fg(theme.text_dim), PlayOptionsHit::SubMarginYDelta(0)),
            ("[Off]".to_string(), pill_style(op <= 0.05, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.0)),
            ("[35%]".to_string(), pill_style((op - 0.35).abs() < 0.08, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.35)),
            ("[65%]".to_string(), pill_style((op - 0.65).abs() < 0.08, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.65)),
            ("[85%]".to_string(), pill_style(op >= 0.80, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.85)),
        ];
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Margin:", Style::default().fg(theme.text_dim))),
            &fine_b,
            hits,
        );
        ri += 1;

        // Row C: Text Ink
        let cur_ink = st_cfg.primary_color.to_uppercase();
        let ink_pills = [
            ("White", "#FFFFFF", Color::White),
            ("Gold", "#FCD34D", Color::Rgb(252, 211, 77)),
            ("Cyan", "#67E8F9", Color::Rgb(103, 232, 249)),
            ("Mint", "#6EE7B7", Color::Rgb(110, 231, 183)),
        ]
        .into_iter()
        .map(|(lbl, hex, fg)| {
            let active = cur_ink == hex || (hex == "#FFFFFF" && cur_ink == "#F3F4F6");
            (
                format!("[{lbl}]"),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(fg)
                },
                PlayOptionsHit::SetSubColor(hex),
            )
        })
        .collect::<Vec<_>>();
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Ink   :", Style::default().fg(theme.text_dim))),
            &ink_pills,
            hits,
        );
        ri += 1;
    }

    // Bottom: Crisp Right-Sized Live Typography Preview Frame
    let prev_rect = rows[ri];
    let prev_blk = Block::default()
        .title(Span::styled(
            format!(" Live Preview ({} · {}px) ", st_cfg.preset.label(), st_cfg.font_size),
            Style::default().fg(theme.accent),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.divider));
    let prev_inner = prev_blk.inner(prev_rect);
    frame.render_widget(prev_blk, prev_rect);
    if prev_inner.width > 8 && prev_inner.height >= 2 {
        let fg_col = parse_hex_color(&st_cfg.primary_color, Color::White);
        let mut text_style = Style::default()
            .fg(if sub_on { fg_col } else { theme.text_dim })
            .add_modifier(Modifier::BOLD);
        if st_cfg.backdrop_opacity > 0.05 {
            text_style = text_style.bg(Color::Rgb(16, 18, 24));
        }
        let mut preview_lines = Vec::new();
        if prev_inner.height >= 4 {
            preview_lines.push(Line::from(if sub_on {
                ""
            } else {
                "(Subtitles currently hidden — press [c] to enable)"
            }));
        }
        preview_lines.push(Line::from(Span::styled(
            truncate_words("\"Shooting with an old MFT camera\"", prev_inner.width as usize),
            text_style,
        )));
        preview_lines.push(Line::from(Span::styled(
            truncate_words("still delivers rich cinema color.", prev_inner.width as usize),
            text_style,
        )));
        frame.render_widget(
            Paragraph::new(preview_lines).alignment(Alignment::Center),
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
    if inner.width < 24 || inner.height < 11 {
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
    let sb_mode = state.config.sponsorblock_mode;
    let skip_outro = state.config.skip_outro;
    let saved_total_secs = state.analytics.sponsorblock_saved_secs;
    let chapters = state
        .active_selected_video()
        .map(|v| v.effective_chapters())
        .unwrap_or_default();
    let chapter_starts: Vec<f64> = chapters.iter().map(|c| c.start_time).collect();

    let total_segs = state.detail_sponsor_segments.len();
    let total_chaps = chapters.len();
    let show_chapters = match state.studio.captions_use_chapters {
        Some(v) => v,
        None => total_segs == 0 && total_chaps > 0,
    };

    let show_matrix = inner.height >= 17;
    let mut constraints = vec![
        Constraint::Length(1), // 0: Mode row 1
        Constraint::Length(1), // 1: Mode row 2
        Constraint::Length(1), // 2: Skip Outros + Saved summary
    ];
    if show_matrix {
        constraints.push(Constraint::Length(4)); // Category Guard Matrix (4 rows x 2 cols)
    }
    constraints.extend([
        Constraint::Length(1), // Timeline header
        Constraint::Length(1), // Visual Timeline Bar
        Constraint::Length(1), // Legend
        Constraint::Length(1), // Jump List Tab switcher ([Segments] [Chapters])
        Constraint::Min(3),    // Scrollable Segments or Chapters List
    ]);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ri = 0usize;

    // Rows 0 & 1: 2x2 SponsorBlockMode selector
    let modes_r0 = [SponsorBlockMode::AutoSkipAll, SponsorBlockMode::SponsorOnly];
    let modes_r1 = [SponsorBlockMode::ShowChaptersOnly, SponsorBlockMode::Off];
    for pair in [modes_r0, modes_r1] {
        let m_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[ri]);
        for (c_idx, mode) in pair.into_iter().enumerate() {
            let active = sb_mode == mode;
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
        ri += 1;
    }

    // Row 2: Skip Outros + Session Skips + Lifetime Saved
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Outros [o]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if skip_outro { "SKIP" } else { "PROTECTED" },
                Style::default()
                    .fg(if skip_outro {
                        Color::Rgb(251, 191, 36)
                    } else {
                        Color::Rgb(52, 211, 153)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " | Session: {} | Saved: {}",
                    state.sponsor_skips_session,
                    format_hms(saved_total_secs)
                ),
                Style::default().fg(theme.secondary),
            ),
        ])),
        rows[ri],
    );
    hits.push((rows[ri], PlayOptionsHit::ToggleSkipOutro));
    ri += 1;

    // 7-Category SponsorBlock Guard Matrix (4 rows x 2 cols)
    if show_matrix {
        let cats = [
            ("sponsor", "Sponsor"),
            ("selfpromo", "Self-Promo"),
            ("intro", "Intro"),
            ("outro", "Outro"),
            ("interaction", "Interact"),
            ("preview", "Preview"),
            ("filler", "Filler"),
        ];
        let mut matrix_lines = Vec::with_capacity(4);
        let pair_w = (rows[ri].width as usize / 2).max(16);
        for row_idx in 0..4 {
            let mut spans = Vec::new();
            for col_idx in 0..2 {
                let idx = row_idx * 2 + col_idx;
                if let Some(&(key, name)) = cats.get(idx) {
                    let (badge, bcol) = if key == "outro" && !skip_outro {
                        ("[PROT]", Color::Rgb(52, 211, 153))
                    } else if sb_mode == SponsorBlockMode::Off {
                        ("[OFF ]", theme.text_dim)
                    } else if sb_mode.auto_skips_category_with_outro(key, skip_outro) {
                        ("[SKIP]", Color::Rgb(251, 191, 36))
                    } else {
                        ("[BADG]", Color::Rgb(56, 189, 248))
                    };
                    let lbl_w = pair_w.saturating_sub(10).max(8);
                    spans.push(Span::styled(" █ ", Style::default().fg(sponsor_color(key))));
                    spans.push(Span::styled(
                        format!("{name:<lbl_w$}"),
                        Style::default().fg(theme.text),
                    ));
                    spans.push(Span::styled(
                        format!("{badge} "),
                        Style::default().fg(bcol).add_modifier(Modifier::BOLD),
                    ));
                } else {
                    let lbl_w = pair_w.saturating_sub(10).max(8);
                    let saved_lbl = format!("Saved {}", format_hms(saved_total_secs));
                    spans.push(Span::styled(" ⏱ ", Style::default().fg(theme.secondary)));
                    spans.push(Span::styled(
                        truncate_words(&saved_lbl, lbl_w + 6),
                        Style::default().fg(theme.secondary),
                    ));
                }
            }
            matrix_lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(matrix_lines), rows[ri]);
        ri += 1;
    }

    // Timeline Distribution Header
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " Timeline (00:00 .. {} · ▼ Playhead · ┆ Chapters):",
                format_hms(dur)
            ),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    // Timeline bar with Sponsor segments AND Chapter boundary ticks
    let bar_w = rows[ri].width.saturating_sub(4).max(8) as usize;
    let mut t_spans = vec![Span::styled(" ├", Style::default().fg(theme.text_dim))];
    t_spans.extend(sponsor_timeline_with_chapters_spans(
        &state.detail_sponsor_segments,
        &chapter_starts,
        pos,
        dur,
        bar_w,
        &theme,
    ));
    t_spans.push(Span::styled("┤ ", Style::default().fg(theme.text_dim)));
    frame.render_widget(Paragraph::new(Line::from(t_spans)), rows[ri]);
    if dur > 1.0 {
        hits.push((rows[ri], PlayOptionsHit::SeekRatio(-1.0)));
    }
    ri += 1;

    // Color Legend
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" █ ", Style::default().fg(sponsor_color("sponsor"))),
            Span::styled("Sponsor  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("selfpromo"))),
            Span::styled("SelfPromo  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("intro"))),
            Span::styled("Intro  ", Style::default().fg(theme.text_dim)),
            Span::styled("█ ", Style::default().fg(sponsor_color("outro"))),
            Span::styled("Outro  ", Style::default().fg(theme.text_dim)),
            Span::styled("┆ Chap", Style::default().fg(theme.secondary)),
        ])),
        rows[ri],
    );
    ri += 1;

    // Jump List Mode Tabs via render_inline_pills
    let tab_pills = vec![
        (
            format!("[SB Segments ({total_segs})]"),
            pill_style(!show_chapters, theme.primary, theme.text_dim),
            PlayOptionsHit::SetCaptionsListTab(false),
        ),
        (
            format!("[Chapters ({total_chaps})]"),
            pill_style(show_chapters, theme.secondary, theme.text_dim),
            PlayOptionsHit::SetCaptionsListTab(true),
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Jump List [u]:", Style::default().fg(theme.text).add_modifier(Modifier::BOLD))),
        &tab_pills,
        hits,
    );
    ri += 1;

    // Dual-Mode Clickable Segments or Chapters List
    let list_area = rows[ri];
    let cap = list_area.height as usize;
    let mut list_lines = Vec::new();
    if show_chapters {
        if chapters.is_empty() {
            list_lines.push(Line::from(Span::styled(
                " No video chapters available in description or metadata.",
                Style::default().fg(theme.text_dim),
            )));
        } else {
            let cursor = state.studio.chapter_cursor.min(chapters.len() - 1);
            let scroll = state
                .studio
                .chapter_scroll
                .min(chapters.len().saturating_sub(cap));
            for (rel_idx, ch) in chapters.iter().skip(scroll).take(cap).enumerate() {
                let abs_idx = scroll + rel_idx;
                let row = list_lines.len() as u16;
                let next_start = chapters
                    .get(abs_idx + 1)
                    .map(|c| c.start_time)
                    .unwrap_or(dur.max(ch.start_time + 1.0));
                let is_playing = pos >= ch.start_time && pos < next_start;
                let selected = focused && abs_idx == cursor;
                let title_w = list_area.width.saturating_sub(21).max(10) as usize;
                let status = if is_playing { "[ACT]" } else { "Jump " };
                let txt = format!(
                    " {} {:>5}  {:<title_w$}  {}",
                    if selected { ">" } else { " " },
                    format_hms(ch.start_time),
                    truncate_words(&ch.title, title_w),
                    status
                );
                list_lines.push(Line::from(Span::styled(
                    txt,
                    if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme.primary)
                            .add_modifier(Modifier::BOLD)
                    } else if is_playing {
                        Style::default()
                            .fg(Color::Rgb(52, 211, 153))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                )));
                push_line_hit(
                    hits,
                    list_area,
                    row,
                    PlayOptionsHit::JumpSponsorSegment(ch.start_time),
                );
            }
        }
    } else if total_segs == 0 {
        list_lines.push(Line::from(Span::styled(
            " No SponsorBlock segments detected for this video.",
            Style::default().fg(theme.text_dim),
        )));
        if total_chaps > 0 {
            list_lines.push(Line::from(Span::styled(
                format!(" Tip: Click [Chapters ({total_chaps})] or press [u] to browse chapter jumps."),
                Style::default().fg(theme.secondary),
            )));
        } else {
            list_lines.push(Line::from(Span::styled(
                " Categories guarded: Sponsor, Self-Promo, Intro, Outro, Interaction.",
                Style::default().fg(theme.secondary),
            )));
        }
    } else {
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
            let row = list_lines.len() as u16;
            let selected = focused && abs_idx == cursor;
            let len_s = (seg.end_time - seg.start_time).max(0.0);
            let status = if seg.category == "outro" && !skip_outro {
                "[Protected]"
            } else if !sb_mode.auto_skips_category_with_outro(&seg.category, skip_outro) {
                "[BadgeOnly]"
            } else if pos >= seg.end_time {
                "[Skipped ✓]"
            } else if pos >= seg.start_time && pos < seg.end_time {
                "[Active]   "
            } else {
                "[Queued]   "
            };
            let cat_lbl = category_label(&seg.category);
            let cat_w = list_area.width.saturating_sub(36).max(8) as usize;
            let txt = format!(
                " {} {}–{} ({:>3.0}s) {:<cat_w$} {}",
                if selected { ">" } else { " " },
                format_hms(seg.start_time),
                format_hms(seg.end_time),
                len_s,
                truncate_words(cat_lbl, cat_w),
                status
            );
            list_lines.push(Line::from(Span::styled(
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
                list_area,
                row,
                PlayOptionsHit::JumpSponsorSegment(seg.start_time),
            );
        }
    }
    frame.render_widget(Paragraph::new(list_lines), list_area);
    push_pane_hit(hits, area, 1);
}
