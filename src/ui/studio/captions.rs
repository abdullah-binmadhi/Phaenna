//! Page 5 — `[5] AdBlock & Subs`: Subtitles & Live Typography Studio (43%),
//! and Interactive Ad-Block & SponsorBlock Shield Command Center (57%).

use crate::api::sponsorblock::category_label;
use crate::config::{SponsorBlockMode, SponsorCategoryPolicy};
use crate::services::substyler::{SubtitlePreset, SubtitleStyleConfig};
use crate::state::{AppState, CaptionsTab, PlayOptionsHit};
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
    if area.width < 34 || area.height < 14 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(43), Constraint::Percentage(57)])
        .split(area);

    render_subtitles_preview_pane(frame, cols[0], state);
    render_adblock_shield_pane(frame, cols[1], state);
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
    let blk = section_block(" Subtitles & Typography Studio ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 11 {
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

    let show_tuner = inner.height >= 16;
    let show_div = inner.height >= 19;
    let preview_h: u16 = if inner.height >= 19 { 5 } else { 4 };

    let mut constraints = vec![
        Constraint::Length(1), // 0: Subtitles + Collision
        Constraint::Length(1), // 1: Sub Offset pills
        Constraint::Length(1), // 2: Default Lang pills
        Constraint::Length(1), // 3: Tracks header + [Reload]
        Constraint::Min(2),    // 4: Scrollable sub tracks (absorbs extra vertical space!)
    ];
    if show_div {
        constraints.push(Constraint::Length(1)); // Divider
    }
    constraints.extend([
        Constraint::Length(1), // Typography Preset header
        Constraint::Length(1), // Preset pills
    ]);
    if show_tuner {
        constraints.extend([
            Constraint::Length(1), // Font Family pills
            Constraint::Length(1), // Font Size + Border
            Constraint::Length(1), // MarginY + Shadow
            Constraint::Length(1), // Backdrop BG pills
            Constraint::Length(1), // Ink pills
        ]);
    }
    constraints.push(Constraint::Length(preview_h)); // Fixed 4-5 row Preview Frame (NEVER a blank hole!)

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

    // 1: Sub Offset controls
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
        Some((" Lang :", Style::default().fg(theme.text_dim))),
        &lang_pills,
        hits,
    );
    ri += 1;

    // 3 & 4: Tracks header + [Reload] + list
    let hdr_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(18), Constraint::Length(14)])
        .split(rows[ri]);
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
        hdr_cols[0],
    );
    render_inline_pills(
        frame,
        hdr_cols[1],
        None,
        &[(
            "[Reload Subs]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::ReloadSubtitles,
        )],
        hits,
    );
    ri += 1;

    let tr_area = rows[ri];
    ri += 1;
    let mut tr_lines = Vec::new();
    if total_subs == 0 {
        tr_lines.push(Line::from(Span::styled(
            format!("   Default: {} (libass + \\an8 ready)", def_lang),
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = tr_area.height as usize;
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

    if show_div {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            rows[ri],
        );
        ri += 1;
    }

    // Typography Preset header & pills
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Typography Preset [y] (libass · {}):", st_cfg.font_family),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    let mut preset_pills: Vec<(String, Style, PlayOptionsHit)> = [
        (SubtitlePreset::StudioMinimal, "Studio"),
        (SubtitlePreset::NetflixClean, "Netflix"),
        (SubtitlePreset::CinemaGold, "Gold"),
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

    if show_tuner {
        // Font Family quick-picker
        let cur_font = st_cfg.font_family.as_str();
        let font_pills = [
            ("Trebuchet", "Trebuchet MS"),
            ("Helvetica", "Helvetica Neue"),
            ("Inter", "Inter"),
            ("Mono", "Menlo"),
        ]
        .into_iter()
        .map(|(short, fam)| {
            (
                format!("[{short}]"),
                pill_style(cur_font.eq_ignore_ascii_case(fam), theme.primary, theme.text_dim),
                PlayOptionsHit::SetSubFont(fam),
            )
        })
        .collect::<Vec<_>>();
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Font :", Style::default().fg(theme.text_dim))),
            &font_pills,
            hits,
        );
        ri += 1;

        // Font Size + Border Size
        let fine_a = vec![
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubFontSizeDelta(-2)),
            (format!("{}px", st_cfg.font_size), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubFontSizeDelta(0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubFontSizeDelta(2)),
            ("Bdr:".to_string(), Style::default().fg(theme.text_dim), PlayOptionsHit::SubBorderDelta(0.0)),
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubBorderDelta(-0.4)),
            (format!("{:.1}", st_cfg.border_size), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubBorderDelta(0.0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubBorderDelta(0.4)),
        ];
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Size :", Style::default().fg(theme.text_dim))),
            &fine_a,
            hits,
        );
        ri += 1;

        // MarginY + Shadow Depth
        let fine_b = vec![
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubMarginYDelta(-4)),
            (format!("{}px", st_cfg.margin_y), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubMarginYDelta(0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubMarginYDelta(4)),
            ("Shd:".to_string(), Style::default().fg(theme.text_dim), PlayOptionsHit::SubShadowDelta(0.0)),
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubShadowDelta(-0.3)),
            (format!("{:.1}", st_cfg.shadow_offset), Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::SubShadowDelta(0.0)),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::SubShadowDelta(0.3)),
        ];
        render_inline_pills(
            frame,
            rows[ri],
            Some((" MgnY :", Style::default().fg(theme.text_dim))),
            &fine_b,
            hits,
        );
        ri += 1;

        // Backdrop opacity
        let op = st_cfg.backdrop_opacity;
        let bg_pills = vec![
            ("[Off]".to_string(), pill_style(op <= 0.05, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.0)),
            ("[35%]".to_string(), pill_style((op - 0.35).abs() < 0.08, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.35)),
            ("[65%]".to_string(), pill_style((op - 0.65).abs() < 0.08, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.65)),
            ("[85%]".to_string(), pill_style(op >= 0.80, theme.primary, theme.text_dim), PlayOptionsHit::SetSubBackdrop(0.85)),
        ];
        render_inline_pills(
            frame,
            rows[ri],
            Some((" BG   :", Style::default().fg(theme.text_dim))),
            &bg_pills,
            hits,
        );
        ri += 1;

        // Text Ink
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
            Some((" Ink  :", Style::default().fg(theme.text_dim))),
            &ink_pills,
            hits,
        );
        ri += 1;
    }

    // Bounded 4-5 Row Live Preview Frame
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
        if !sub_on && prev_inner.height >= 3 {
            preview_lines.push(Line::from(Span::styled(
                "(Hidden — press [c] to enable)",
                Style::default().fg(theme.text_dim),
            )));
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

fn render_adblock_shield_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Ad-Block & SponsorBlock Shield Engine ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 28 || inner.height < 12 {
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
    let min_secs = state.config.sponsorblock_min_secs;
    let osd_toast = state.config.sponsorblock_osd_toast;
    let has_ipc = state.has_live_ipc();
    let whitelist = state.config.sponsorblock_whitelist.clone();
    let active_channel = state
        .active_selected_video()
        .map(|v| v.display_uploader().to_string())
        .filter(|s| !s.is_empty());
    let ch_whitelisted = active_channel
        .as_deref()
        .map(|c| state.config.is_channel_whitelisted(c))
        .unwrap_or(false);
    let chapters = state
        .active_selected_video()
        .map(|v| v.effective_chapters())
        .unwrap_or_default();
    let chapter_starts: Vec<f64> = chapters.iter().map(|c| c.start_time).collect();

    let total_segs = state.detail_sponsor_segments.len();
    let total_chaps = chapters.len();
    let active_tab = state.studio.captions_tab.unwrap_or(if total_segs > 0 {
        CaptionsTab::Segments
    } else if total_chaps > 0 {
        CaptionsTab::Chapters
    } else {
        CaptionsTab::Shield
    });

    let show_div = inner.height >= 19;
    let mut constraints = vec![
        Constraint::Length(1), // 0: Shield mode pills
        Constraint::Length(1), // 1: Instant Ad-Block actions
    ];
    if show_div {
        constraints.push(Constraint::Length(1));
    }
    constraints.extend([
        Constraint::Length(1), // Category Matrix header
        Constraint::Length(4), // Interactive 8-Category Policy Matrix
    ]);
    if show_div {
        constraints.push(Constraint::Length(1));
    }
    constraints.extend([
        Constraint::Length(1), // Min len + OSD alert toggle
        Constraint::Length(1), // Stream Ad-Block status + Saved stats
        Constraint::Length(1), // Timeline header
        Constraint::Length(1), // Timeline bar
        Constraint::Length(1), // Timeline legend
        Constraint::Length(1), // 3-Tab Inspector pills
        Constraint::Min(3),    // Inspector content (Segments / Chapters / Shield & WL)
    ]);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ri = 0usize;

    // 0: Shield master mode pills
    let mode_pills = vec![
        ("Aggressive", SponsorBlockMode::AutoSkipAll, Color::Rgb(52, 211, 153)),
        ("Sponsors", SponsorBlockMode::SponsorOnly, theme.primary),
        ("Notify", SponsorBlockMode::ShowChaptersOnly, Color::Rgb(56, 189, 248)),
        ("Custom", SponsorBlockMode::Custom, theme.accent),
        ("OFF", SponsorBlockMode::Off, Color::Rgb(248, 113, 113)),
    ]
    .into_iter()
    .map(|(lbl, m, col)| {
        (
            format!("[{lbl}]"),
            pill_style(sb_mode == m, col, theme.text_dim),
            PlayOptionsHit::SelectSponsorBlock(m),
        )
    })
    .collect::<Vec<_>>();
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Shield:", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
        &mode_pills,
        hits,
    );
    ri += 1;

    // 1: Instant Ad-Block Action Bar
    let action_pills = vec![
        (
            "[R Re-Scan]".to_string(),
            Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
            PlayOptionsHit::ReScanSponsorBlock,
        ),
        (
            "[U Undo Skip]".to_string(),
            Style::default().fg(Color::Rgb(251, 191, 36)).add_modifier(Modifier::BOLD),
            PlayOptionsHit::UndoLastSponsorSkip,
        ),
        (
            "[Skip Now]".to_string(),
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            PlayOptionsHit::SkipActiveSponsorNow,
        ),
        (
            if ch_whitelisted {
                "[W WL: ON ✓]"
            } else {
                "[W Whitelist Ch]"
            }
            .to_string(),
            pill_style(ch_whitelisted, Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::ToggleWhitelistChannel,
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Act   :", Style::default().fg(theme.text_dim))),
        &action_pills,
        hits,
    );
    ri += 1;

    if show_div {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            rows[ri],
        );
        ri += 1;
    }

    // Category header
    frame.render_widget(
        Paragraph::new(Span::styled(
            if ch_whitelisted {
                " Category Policy (CHANNEL WHITELISTED — Auto-Skip Paused):"
            } else {
                " Category Policy (Click any category to toggle SKIP / NOTF / IGNR):"
            },
            Style::default()
                .fg(if ch_whitelisted {
                    Color::Rgb(52, 211, 153)
                } else {
                    theme.primary
                })
                .add_modifier(Modifier::BOLD),
        )),
        rows[ri],
    );
    ri += 1;

    // 8 Interactive Category Cells (4 rows x 2 columns)
    let mat_area = rows[ri];
    ri += 1;
    let cats = [
        ("sponsor", "Sponsor (Paid Ad)"),
        ("selfpromo", "Self-Promo / Merch"),
        ("intro", "Intro / Intermis."),
        ("outro", "Outro / Endcards"),
        ("interaction", "Like / Sub Prompt"),
        ("preview", "Preview / Recap"),
        ("filler", "Filler / Tangents"),
        ("music_offtopic", "Non-Music Section"),
    ];
    let row_splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1); 4])
        .split(mat_area);

    for (row_idx, row_rect) in row_splits.iter().enumerate() {
        let cell_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(*row_rect);
        for (col_idx, cell_rect) in cell_cols.iter().enumerate() {
            let idx = row_idx * 2 + col_idx;
            if let Some(&(key, name)) = cats.get(idx) {
                let eff_pol = state.config.effective_sponsor_policy(key);
                let (badge, bcol) = if ch_whitelisted {
                    ("[WL  ]", Color::Rgb(52, 211, 153))
                } else {
                    match eff_pol {
                        SponsorCategoryPolicy::AutoSkip => ("[SKIP]", Color::Rgb(251, 191, 36)),
                        SponsorCategoryPolicy::NotifyOnly => ("[NOTF]", Color::Rgb(56, 189, 248)),
                        SponsorCategoryPolicy::Ignore => ("[IGNR]", theme.text_dim),
                    }
                };
                let lbl_w = (cell_rect.width as usize).saturating_sub(11).max(8);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" █ ", Style::default().fg(sponsor_color(key))),
                        Span::styled(
                            format!("{:<lbl_w$} ", truncate_words(name, lbl_w)),
                            Style::default().fg(theme.text),
                        ),
                        Span::styled(
                            badge,
                            Style::default().fg(bcol).add_modifier(Modifier::BOLD),
                        ),
                    ])),
                    *cell_rect,
                );
                hits.push((*cell_rect, PlayOptionsHit::CycleSponsorCategory(key)));
            }
        }
    }

    if show_div {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", "─".repeat(inner.width.saturating_sub(2) as usize)),
                Style::default().fg(theme.divider),
            )),
            rows[ri],
        );
        ri += 1;
    }

    // Min Skip Threshold + OSD Alert toggle + Outro toggle
    let filter_pills = vec![
        ("[>=1s]".to_string(), pill_style(min_secs <= 1, theme.primary, theme.text_dim), PlayOptionsHit::SetSponsorMinSecs(1)),
        ("[>=3s]".to_string(), pill_style(min_secs == 3, theme.primary, theme.text_dim), PlayOptionsHit::SetSponsorMinSecs(3)),
        ("[>=5s]".to_string(), pill_style(min_secs >= 5, theme.primary, theme.text_dim), PlayOptionsHit::SetSponsorMinSecs(5)),
        (
            if osd_toast { "[OSD: ON]" } else { "[OSD: OFF]" }.to_string(),
            pill_style(osd_toast, Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::ToggleSponsorOsdToast,
        ),
        (
            if skip_outro { "[Out: SKIP]" } else { "[Out: PROT]" }.to_string(),
            pill_style(skip_outro, Color::Rgb(251, 191, 36), Color::Rgb(52, 211, 153)),
            PlayOptionsHit::ToggleSkipOutro,
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Min   :", Style::default().fg(theme.text_dim))),
        &filter_pills,
        hits,
    );
    ri += 1;

    // Stream Ad-Block status & savings telemetry
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Stream Ads: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                "[BLOCKED ✓ Clean Feed]",
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " | Session: {} Skips | Saved: {}",
                    state.sponsor_skips_session,
                    format_hms(saved_total_secs)
                ),
                Style::default().fg(theme.secondary),
            ),
        ])),
        rows[ri],
    );
    ri += 1;

    // Timeline header
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

    // Timeline bar
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
            Span::styled("█ ", Style::default().fg(sponsor_color("music_offtopic"))),
            Span::styled("NonMusic  ", Style::default().fg(theme.text_dim)),
            Span::styled("┆ Chap", Style::default().fg(theme.secondary)),
        ])),
        rows[ri],
    );
    ri += 1;

    // 3-Tab Switcher
    let tab_pills = vec![
        (
            format!("[Segments ({total_segs})]"),
            pill_style(active_tab == CaptionsTab::Segments, theme.primary, theme.text_dim),
            PlayOptionsHit::SetCaptionsTab(CaptionsTab::Segments),
        ),
        (
            format!("[Chapters ({total_chaps})]"),
            pill_style(active_tab == CaptionsTab::Chapters, theme.secondary, theme.text_dim),
            PlayOptionsHit::SetCaptionsTab(CaptionsTab::Chapters),
        ),
        (
            format!("[Shield & WL ({})]", whitelist.len()),
            pill_style(active_tab == CaptionsTab::Shield, Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::SetCaptionsTab(CaptionsTab::Shield),
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" View [u]:", Style::default().fg(theme.text).add_modifier(Modifier::BOLD))),
        &tab_pills,
        hits,
    );
    ri += 1;

    // 3-Tab Bottom Inspector
    let list_area = rows[ri];
    let cap = list_area.height as usize;
    let mut list_lines = Vec::new();
    match active_tab {
        CaptionsTab::Segments => {
            if total_segs == 0 {
                list_lines.push(Line::from(Span::styled(
                    " No SponsorBlock segments returned for this video yet.",
                    Style::default().fg(theme.text_dim),
                )));
                list_lines.push(Line::from(Span::styled(
                    " Press [R] to Re-Scan API live, or switch to [Shield & WL] via [u].",
                    Style::default().fg(theme.secondary),
                )));
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
                    let pol = state.config.effective_sponsor_policy(&seg.category);
                    let status = if ch_whitelisted {
                        "[WL Allow ]"
                    } else if pol != SponsorCategoryPolicy::AutoSkip {
                        pol.badge()
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
        }
        CaptionsTab::Chapters => {
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
        }
        CaptionsTab::Shield => {
            let ch_lbl = active_channel.as_deref().unwrap_or("Selected Channel");
            list_lines.push(Line::from(vec![
                Span::styled(" Active Creator : ", Style::default().fg(theme.text_dim)),
                Span::styled(
                    truncate_words(ch_lbl, list_area.width.saturating_sub(36).max(10) as usize),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if ch_whitelisted {
                        " [WHITELISTED]"
                    } else {
                        " [Shielded]"
                    },
                    Style::default().fg(if ch_whitelisted {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.secondary
                    }),
                ),
            ]));
            list_lines.push(Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        "YouTube Pre/Mid-Roll Ad-Block: Native stream URL extraction bypasses video ads.",
                        list_area.width.saturating_sub(1) as usize
                    )
                ),
                Style::default().fg(Color::Rgb(52, 211, 153)),
            )));
            list_lines.push(Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        &format!(
                            "Live IPC Sync: {} | Min Skip: >={}s | OSD Toast: {}",
                            if has_ipc { "Active (Hot-updates instantly)" } else { "Ready for launch" },
                            min_secs,
                            if osd_toast { "ON" } else { "OFF" }
                        ),
                        list_area.width.saturating_sub(1) as usize
                    )
                ),
                Style::default().fg(theme.text_dim),
            )));

            if whitelist.is_empty() {
                list_lines.push(Line::from(Span::styled(
                    " Whitelisted Channels: None (Press [W] to allow a creator without skipping).",
                    Style::default().fg(theme.text_dim),
                )));
            } else {
                for ch in whitelist.into_iter().take(cap.saturating_sub(3)) {
                    let row = list_lines.len() as u16;
                    let w_lbl = list_area.width.saturating_sub(18).max(10) as usize;
                    list_lines.push(Line::from(vec![
                        Span::styled(" WL: ", Style::default().fg(Color::Rgb(52, 211, 153))),
                        Span::styled(
                            format!("{:<w_lbl$} ", truncate_words(&ch, w_lbl)),
                            Style::default().fg(theme.text),
                        ),
                        Span::styled("[Remove]", Style::default().fg(Color::Rgb(248, 113, 113))),
                    ]));
                    push_line_hit(hits, list_area, row, PlayOptionsHit::RemoveWhitelistChannel(ch));
                }
            }
        }
    }

    frame.render_widget(Paragraph::new(list_lines), list_area);
    push_pane_hit(hits, area, 1);
}
