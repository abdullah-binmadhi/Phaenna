//! Bento Video Detail Command Deck (`Screen::Detail`).
//! Eliminates dead space via aspect-locked 16:9 Ambilight artwork,
//! visual ReturnYouTubeDislike sentiment gauge, SponsorBlock timeline bar,
//! syntax-highlighted synopsis + extracted links/tags, active mastering preset readout,
//! always-visible interactive Chapter jump list, and Up Next / Channel drawer.

use crate::api::dislike::format_compact_count;
use crate::api::sponsorblock::category_label;
use crate::db::cache::{format_duration_secs, VideoChapter, VideoItem};
use crate::state::{occluded_by_modal, ActiveOverlay, AppState};
use crate::ui::components::cover::render_thumbnail_cover;
use crate::ui::theme::ThemePalette;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix_detail() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn sponsor_color(category: &str) -> Color {
    match category {
        "sponsor" => Color::Rgb(0, 212, 0),
        "selfpromo" => Color::Rgb(255, 255, 0),
        "interaction" => Color::Rgb(204, 0, 255),
        "intro" => Color::Rgb(0, 255, 255),
        "outro" => Color::Rgb(96, 165, 250),
        _ => Color::Rgb(255, 118, 142),
    }
}

fn render_sponsor_timeline_line(state: &AppState, total_secs: f64, width: usize) -> Line<'static> {
    if total_secs <= 1.0 || width < 8 {
        return Line::from("");
    }
    let w = width.min(64);
    let mut spans = Vec::with_capacity(w);
    for col in 0..w {
        let frac = (col as f64 + 0.5) / w as f64;
        let t = frac * total_secs;
        let mut hit = None;
        for seg in &state.detail_sponsor_segments {
            if t >= seg.start_time && t <= seg.end_time {
                hit = Some(seg.category.as_str());
                break;
            }
        }
        if let Some(cat) = hit {
            spans.push(Span::styled("━", Style::default().fg(sponsor_color(cat))));
        } else {
            spans.push(Span::styled("─", Style::default().fg(Color::Rgb(70, 78, 98))));
        }
    }
    Line::from(spans)
}

fn build_sentiment_gauge(like_pct: Option<f64>, width: usize, pal: &ThemePalette) -> Line<'static> {
    let w = width.clamp(8, 36);
    match like_pct {
        Some(pct) => {
            let clamped = pct.clamp(0.0, 100.0);
            let filled = ((clamped / 100.0) * w as f64).round() as usize;
            let rest = w.saturating_sub(filled);
            Line::from(vec![
                Span::styled(
                    "━".repeat(filled),
                    Style::default().fg(Color::Rgb(52, 211, 153)),
                ),
                Span::styled(
                    "─".repeat(rest),
                    Style::default().fg(Color::Rgb(248, 113, 113)),
                ),
            ])
        }
        None => Line::from(Span::styled(
            "─".repeat(w),
            Style::default().fg(pal.divider),
        )),
    }
}

fn looks_like_timestamp(token: &str) -> bool {
    let t = token.trim_matches(|c: char| !c.is_ascii_digit() && c != ':');
    let parts: Vec<&str> = t.split(':').collect();
    matches!(parts.len(), 2 | 3)
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()))
}

fn style_description_lines(
    desc: &str,
    video: &VideoItem,
    pal: &ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut urls = Vec::new();
    let mut hashtags = Vec::new();

    for raw_line in desc.lines() {
        let mut spans = Vec::new();
        for (idx, tok) in raw_line.split(' ').enumerate() {
            if idx > 0 {
                spans.push(Span::raw(" "));
            }
            if tok.starts_with("http://") || tok.starts_with("https://") {
                if urls.len() < 4 && !urls.iter().any(|u| u == tok) {
                    urls.push(tok.to_string());
                }
                spans.push(Span::styled(
                    tok.to_string(),
                    Style::default()
                        .fg(pal.secondary)
                        .add_modifier(Modifier::UNDERLINED),
                ));
            } else if tok.starts_with('#') && tok.len() > 1 {
                if hashtags.len() < 8 && !hashtags.iter().any(|h| h == tok) {
                    hashtags.push(tok.to_string());
                }
                spans.push(Span::styled(
                    tok.to_string(),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                ));
            } else if looks_like_timestamp(tok) {
                spans.push(Span::styled(
                    tok.to_string(),
                    Style::default()
                        .fg(pal.primary)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(tok.to_string(), Style::default().fg(pal.text)));
            }
        }
        lines.push(Line::from(spans));
    }

    if let Some(ref tags) = video.tags {
        for tag in tags.iter().take(6) {
            let pill = format!("#{}", tag.replace(' ', ""));
            if hashtags.len() < 8 && !hashtags.iter().any(|h| h.eq_ignore_ascii_case(&pill)) {
                hashtags.push(pill);
            }
        }
    }

    if !urls.is_empty() || !hashtags.is_empty() {
        lines.push(Line::from(""));
        if !urls.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("LINKS ({}):", urls.len()),
                Style::default()
                    .fg(pal.secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            for u in urls {
                lines.push(Line::from(vec![
                    Span::styled("  > ", Style::default().fg(pal.accent)),
                    Span::styled(u, Style::default().fg(pal.secondary)),
                ]));
            }
        }
        if !hashtags.is_empty() {
            let mut tag_spans = vec![Span::styled(
                "TAGS: ",
                Style::default()
                    .fg(pal.accent)
                    .add_modifier(Modifier::BOLD),
            )];
            for h in hashtags {
                tag_spans.push(Span::styled(
                    format!("[{h}] "),
                    Style::default().fg(pal.primary),
                ));
            }
            lines.push(Line::from(tag_spans));
        }
    }

    lines
}

pub fn render_detail_screen(frame: &mut Frame, state: &mut AppState, modal_rect: Option<Rect>) {
    let Some(video) = state.detail_video.clone() else {
        return;
    };
    let pal = state.active_palette_for(Some(&video.id));
    let area = frame.area();
    let now_unix = now_unix_detail();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(1),
        ])
        .split(area);

    render_header_bar(frame, rows[0], state, &video, &pal, now_unix);

    // Aspect-locked left column width matching a 16:9 thumbnail (`~3.55 * inner_rows + 2`)
    let art_inner_h = ((rows[1].height as f32 * 0.34).round() as u16).clamp(8, 11);
    let art_outer_h = (art_inner_h + 2).min(rows[1].height.saturating_sub(10).max(8));
    let left_w = ((art_inner_h as f32 * 3.55).round() as u16 + 2)
        .clamp(30, 42)
        .min((rows[1].width * 38 / 100).max(28));

    if rows[1].width >= 108 {
        let right_w = if rows[1].width >= 136 { 38 } else { 32 };
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w),
                Constraint::Min(34),
                Constraint::Length(right_w),
            ])
            .split(rows[1]);

        render_left_column(frame, cols[0], state, &video, &pal, art_outer_h, now_unix, modal_rect);
        render_center_column(frame, cols[1], state, &video, &pal);
        render_right_column(frame, cols[2], state, &video, &pal);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Min(32)])
            .split(rows[1]);

        render_left_column(frame, cols[0], state, &video, &pal, art_outer_h, now_unix, modal_rect);

        let right_stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(46),
                Constraint::Percentage(32),
                Constraint::Percentage(22),
            ])
            .split(cols[1]);

        render_synopsis_panel(frame, right_stack[0], state, &video, &pal);
        render_chapters_panel(frame, right_stack[1], state, &video, &pal);
        render_related_panel(frame, right_stack[2], state, &pal);
    }

    render_footer_legend(frame, rows[2], state, &pal);
}

fn render_header_bar(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
    now_unix: i64,
) {
    let pane_label = match state.detail_focus_pane {
        1 => "FOCUS: CHAPTERS",
        2 => "FOCUS: UP NEXT / CH",
        _ => "FOCUS: SYNOPSIS",
    };

    let hdr_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary));

    let mut spans = Vec::new();
    if video.is_new_release(now_unix) {
        spans.push(Span::styled(
            " NEW ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        format!(" > {} ", video.title),
        Style::default()
            .fg(pal.primary)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(age) = video.compact_age(now_unix, true) {
        spans.push(Span::styled(
            format!("· {} · {} ", video.display_uploader(), age),
            Style::default().fg(pal.secondary),
        ));
    } else {
        spans.push(Span::styled(
            format!("· {} ", video.display_uploader()),
            Style::default().fg(pal.secondary),
        ));
    }
    if state.is_in_continue_watching(&video.id) {
        spans.push(Span::styled(
            "[IN QUEUE] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if state.offline_paths.contains_key(&video.id) {
        spans.push(Span::styled(
            "[OFFLINE] ",
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        format!("[Tab: {pane_label}]"),
        Style::default().fg(pal.text_dim),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)).block(hdr_blk), area);
}

#[allow(clippy::too_many_arguments)]
fn render_left_column(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    video: &VideoItem,
    pal: &ThemePalette,
    art_outer_h: u16,
    now_unix: i64,
    modal_rect: Option<Rect>,
) {
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(art_outer_h),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Min(5),
        ])
        .split(area);

    // 1. Aspect-locked 16:9 Artwork Stage
    let art_blk = Block::default()
        .title(Span::styled(
            if state.config.ambilight_enabled {
                " Ambilight 16:9 "
            } else {
                " Preview 16:9 "
            },
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_focus));
    let art_inner = art_blk.inner(left_rows[0]);
    frame.render_widget(art_blk, left_rows[0]);

    if !occluded_by_modal(art_inner, modal_rect) && state.overlay == ActiveOverlay::None {
        render_thumbnail_cover(frame, art_inner, &video.id, state, "Loading thumbnail...");
    }

    // 2. Engagement & ReturnYouTubeDislike Telemetry Card
    let stats_blk = Block::default()
        .title(Span::styled(
            " Engagement & Sentiment ",
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let stats_inner = stats_blk.inner(left_rows[1]);
    frame.render_widget(stats_blk, left_rows[1]);

    let views_str = video.compact_views(true).unwrap_or_else(|| "Views loading".into());
    let age_str = video
        .compact_age(now_unix, true)
        .unwrap_or_else(|| "Age unknown".into());
    let resume_str = state
        .progress_map
        .get(&video.id)
        .map(|(p, d)| {
            let pct = if *d > 0.0 {
                ((*p / *d) * 100.0).round() as u32
            } else {
                0
            };
            format!("Resume @ {} ({}%)", format_duration_secs(*p as u64), pct)
        })
        .unwrap_or_else(|| "Unwatched · 00:00".to_string());

    let (like_pct, sentiment_txt) = if let Some(ref d) = state.detail_dislikes {
        (
            Some(d.like_ratio_pct()),
            format!(
                "Likes {} · Dislikes {} ({:.1}%)",
                format_compact_count(d.likes),
                format_compact_count(d.dislikes),
                d.like_ratio_pct()
            ),
        )
    } else if let Some(l) = video.like_count {
        (
            video.like_ratio_pct(),
            format!("{} likes (fetching dislikes...)", format_compact_count(l as i64)),
        )
    } else {
        (None, "Fetching ReturnYouTubeDislike...".to_string())
    };

    let stat_lines = vec![
        Line::from(vec![
            Span::styled("Reach  : ", Style::default().fg(pal.text_dim)),
            Span::styled(format!("{views_str} · {age_str}"), Style::default().fg(pal.text)),
        ]),
        Line::from(vec![
            Span::styled("Runtime: ", Style::default().fg(pal.text_dim)),
            Span::styled(
                format!("{} · {}", video.display_duration(), resume_str),
                Style::default().fg(pal.primary),
            ),
        ]),
        build_sentiment_gauge(like_pct, stats_inner.width as usize, pal),
        Line::from(Span::styled(sentiment_txt, Style::default().fg(pal.accent))),
    ];
    frame.render_widget(Paragraph::new(stat_lines).wrap(Wrap { trim: true }), stats_inner);

    // 3. SponsorBlock Timeline Bar
    let sb_blk = Block::default()
        .title(Span::styled(
            format!(
                " SponsorBlock ({} · {}) ",
                state.detail_sponsor_segments.len(),
                state.config.sponsorblock_mode.label()
            ),
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let sb_inner = sb_blk.inner(left_rows[2]);
    frame.render_widget(sb_blk, left_rows[2]);

    let total_dur = video.duration.unwrap_or(600) as f64;
    let bar = render_sponsor_timeline_line(state, total_dur, sb_inner.width as usize);
    let sb_summary = if state.detail_sponsor_segments.is_empty() {
        "Clean stream · no sponsor segments reported".to_string()
    } else {
        state
            .detail_sponsor_segments
            .iter()
            .take(2)
            .map(|s| {
                format!(
                    "{} {}-{}",
                    category_label(&s.category),
                    format_duration_secs(s.start_time as u64),
                    format_duration_secs(s.end_time as u64)
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    frame.render_widget(
        Paragraph::new(vec![
            bar,
            Line::from(Span::styled(sb_summary, Style::default().fg(pal.text_dim))),
        ]),
        sb_inner,
    );

    // 4. Live Interactive Deck Actions
    let act_blk = Block::default()
        .title(Span::styled(
            " Deck Controls ",
            Style::default()
                .fg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let cw_label = if state.is_in_continue_watching(&video.id) {
        "[a] In Continue Watching [Pinned]"
    } else {
        "[a] Pin to Continue Watching"
    };
    let wl_label = if state.watch_later_sources.contains_key(&video.id) {
        "[w] Saved in Watch Later [Active]"
    } else {
        "[w] Save to Watch Later"
    };
    let dl_label = if state.offline_paths.contains_key(&video.id) {
        "[D] Cached Offline [Ready]"
    } else if let Some(ref dj) = state.active_download {
        if dj.video_id == video.id {
            "[D] Downloading..."
        } else {
            "[D] Download Offline"
        }
    } else {
        "[D] Download to Offline Vault"
    };

    let act_lines = vec![
        Line::from(vec![
            Span::styled("[Enter/Space] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("Play via {}", state.config.player.label()), Style::default().fg(pal.text)),
        ]),
        Line::from(Span::styled(cw_label, Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(wl_label, Style::default().fg(Color::Rgb(251, 191, 36)))),
        Line::from(Span::styled(dl_label, Style::default().fg(pal.primary))),
        Line::from(Span::styled(
            "[p] Quality & Subs · [c] Channel Feed",
            Style::default().fg(pal.text_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(act_lines).block(act_blk), left_rows[3]);
}

fn render_center_column(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let center_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(7)])
        .split(area);

    render_synopsis_panel(frame, center_rows[0], state, video, pal);
    render_mastering_panel(frame, center_rows[1], state, video, pal);
}

fn render_synopsis_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let focused = state.detail_focus_pane == 0;
    let desc_blk = Block::default()
        .title(Span::styled(
            if focused {
                " Synopsis, Links & Tags [Focused · j/k scroll] "
            } else {
                " Synopsis, Links & Tags "
            },
            Style::default()
                .fg(if focused { pal.primary } else { pal.secondary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(if focused {
            pal.border_focus
        } else {
            pal.border_idle
        }));

    let desc_raw = video
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("Loading full metadata, chapters, subtitles, and SponsorBlock cues...");
    let lines = style_description_lines(desc_raw, video, pal);

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.detail_scroll, 0))
            .block(desc_blk),
        area,
    );
}

fn render_mastering_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let blk = Block::default()
        .title(Span::styled(
            " Playback & Mastering Deck ([p] Override) ",
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));

    let sub_summary = if state.detail_subtitles.is_empty() {
        "Auto / Fetching tracks...".to_string()
    } else {
        let sel = state
            .detail_subtitles
            .get(state.detail_selected_sub_idx)
            .map(|t| t.label.as_str())
            .unwrap_or("Default");
        format!("{} tracks ({sel})", state.detail_subtitles.len())
    };
    let source_str = if state.offline_paths.contains_key(&video.id) {
        "Offline Vault (Zero Buffer)"
    } else {
        "Adaptive Live Stream"
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Player  : ", Style::default().fg(pal.text_dim)),
            Span::styled(state.config.player.label(), Style::default().fg(pal.primary).add_modifier(Modifier::BOLD)),
            Span::styled("   Quality: ", Style::default().fg(pal.text_dim)),
            Span::styled(state.config.quality.label(), Style::default().fg(pal.accent)),
        ]),
        Line::from(vec![
            Span::styled("Shader  : ", Style::default().fg(pal.text_dim)),
            Span::styled(state.config.shader_mode.label(), Style::default().fg(pal.secondary)),
        ]),
        Line::from(vec![
            Span::styled("Subs    : ", Style::default().fg(pal.text_dim)),
            Span::styled(sub_summary, Style::default().fg(pal.text)),
        ]),
        Line::from(vec![
            Span::styled("Delivery: ", Style::default().fg(pal.text_dim)),
            Span::styled(source_str, Style::default().fg(pal.accent)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_right_column(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(area);

    render_chapters_panel(frame, right_rows[0], state, video, pal);
    render_related_panel(frame, right_rows[1], state, pal);
}

fn chapter_duration_badge(ch: &VideoChapter) -> String {
    let secs = (ch.end_time - ch.start_time).max(0.0).round() as u64;
    if secs == 0 {
        String::new()
    } else if secs < 60 {
        format!("({secs}s)")
    } else {
        let m = secs / 60;
        let s = secs % 60;
        if s == 0 {
            format!("({m}m)")
        } else {
            format!("({m}m {s:02}s)")
        }
    }
}

fn render_chapters_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let chapters = video.effective_chapters();
    let focused = state.detail_focus_pane == 1;
    let ch_blk = Block::default()
        .title(Span::styled(
            if focused {
                format!(" Chapters ({}) [Focused · Enter Jump] ", chapters.len())
            } else {
                format!(" Chapters ({}) [Tab Jump] ", chapters.len())
            },
            Style::default()
                .fg(if focused { pal.primary } else { pal.accent })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(if focused {
            pal.border_focus
        } else {
            pal.border_idle
        }));
    let ch_inner = ch_blk.inner(area);
    frame.render_widget(ch_blk, area);

    if chapters.is_empty() {
        frame.render_widget(
            Paragraph::new("No chapter markers found in video metadata or description.")
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(pal.text_dim)),
            ch_inner,
        );
        return;
    }

    let cur = state.detail_chapter_cursor.min(chapters.len().saturating_sub(1));
    let vis_h = ch_inner.height.max(1) as usize;
    let scroll_off = if cur >= vis_h { cur + 1 - vis_h } else { 0 };
    let title_budget = (ch_inner.width as usize).saturating_sub(17).max(8);

    let lines: Vec<Line> = chapters
        .iter()
        .enumerate()
        .skip(scroll_off)
        .take(vis_h)
        .map(|(i, c)| {
            let sel = i == cur;
            let row_bg = if sel && focused {
                pal.primary
            } else if sel {
                Color::Rgb(30, 36, 52)
            } else {
                Color::Reset
            };
            let title_fg = if sel && focused {
                Color::Black
            } else if sel {
                pal.primary
            } else {
                pal.text
            };
            let marker = if sel { "> " } else { "  " };
            let short_t: String = c.title.chars().take(title_budget).collect();
            let dur = chapter_duration_badge(c);
            Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(if sel && focused { Color::Black } else { pal.accent })
                        .bg(row_bg),
                ),
                Span::styled(
                    format!("{:>5} ", format_duration_secs(c.start_time as u64)),
                    Style::default()
                        .fg(if sel && focused { Color::Black } else { pal.accent })
                        .bg(row_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{short_t:<title_budget$}"),
                    Style::default().fg(title_fg).bg(row_bg),
                ),
                Span::styled(
                    format!(" {dur}"),
                    Style::default()
                        .fg(if sel && focused { Color::Black } else { pal.text_dim })
                        .bg(row_bg),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), ch_inner);
}

fn render_related_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    pal: &ThemePalette,
) {
    let entries = state.detail_related_entries();
    let focused = state.detail_focus_pane == 2;
    let blk = Block::default()
        .title(Span::styled(
            if focused {
                format!(" Up Next & Channel ({}) [Enter Open] ", entries.len())
            } else {
                format!(" Up Next & Channel ({}) ", entries.len())
            },
            Style::default()
                .fg(if focused { pal.primary } else { pal.secondary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(if focused {
            pal.border_focus
        } else {
            pal.border_idle
        }));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("Press [c] to explore this channel's full catalog, or [a] to add videos to Continue Watching.")
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(pal.text_dim)),
            inner,
        );
        return;
    }

    let cur = state.detail_related_cursor.min(entries.len().saturating_sub(1));
    let vis_h = inner.height.max(1) as usize;
    let scroll_off = if cur >= vis_h { cur + 1 - vis_h } else { 0 };
    let title_budget = (inner.width as usize).saturating_sub(16).max(8);

    let lines: Vec<Line> = entries
        .iter()
        .enumerate()
        .skip(scroll_off)
        .take(vis_h)
        .map(|(i, (tag, v))| {
            let sel = i == cur;
            let row_bg = if sel && focused {
                pal.primary
            } else if sel {
                Color::Rgb(30, 36, 52)
            } else {
                Color::Reset
            };
            let title_fg = if sel && focused {
                Color::Black
            } else if sel {
                pal.primary
            } else {
                pal.text
            };
            let tag_color = if sel && focused {
                Color::Black
            } else if *tag == "QUEUE" {
                Color::Cyan
            } else {
                pal.accent
            };
            let short_t: String = v.title.chars().take(title_budget).collect();
            let dur = v.display_duration();
            Line::from(vec![
                Span::styled(
                    format!("[{tag:<5}] "),
                    Style::default()
                        .fg(tag_color)
                        .bg(row_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{short_t:<title_budget$}"),
                    Style::default().fg(title_fg).bg(row_bg),
                ),
                Span::styled(
                    format!(" {dur}"),
                    Style::default()
                        .fg(if sel && focused { Color::Black } else { pal.text_dim })
                        .bg(row_bg),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_footer_legend(frame: &mut Frame, area: Rect, state: &AppState, pal: &ThemePalette) {
    let enter_hint = match state.detail_focus_pane {
        1 => "[Enter] Play Chapter",
        2 => "[Enter] Open Related Video",
        _ => "[Enter/Space] Play Video",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{enter_hint} "),
                Style::default()
                    .fg(pal.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("· [Tab] Switch Pane ", Style::default().fg(pal.secondary)),
            Span::styled("· [j/k] Scroll/Select ", Style::default().fg(pal.primary)),
            Span::styled("· [p] Options ", Style::default().fg(pal.text)),
            Span::styled("· [c] Channel ", Style::default().fg(pal.text)),
            Span::styled("· [a] +Queue ", Style::default().fg(Color::Cyan)),
            Span::styled("· [w] +WatchLater ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("· [D] Offline · [Esc] Back", Style::default().fg(pal.text_dim)),
        ]))
        .alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_detail_bento_deck_renders_wide_and_compact() {
        let mut st = AppState::new(Config::default());
        st.detail_video = Some(VideoItem {
            id: "det_001".to_string(),
            title: "Inside the $18M Historic Tudor Revival Estate".to_string(),
            uploader: Some("Enes Yilmazer".to_string()),
            duration: Some(1755),
            view_count: Some(27_540),
            like_count: Some(1840),
            dislike_count: Some(42),
            published_text: Some("6 hours ago".to_string()),
            description: Some(
                "Architectural tour.\n0:00 Intro\n1:00 History\n1:30 Exterior\n3:32 Interior\n#Architecture https://example.com"
                    .to_string(),
            ),
            ..Default::default()
        });
        st.categories[0].1.push(VideoItem {
            id: "cw_rel_1".to_string(),
            title: "Queued Followup Episode".to_string(),
            uploader: Some("DevLab".to_string()),
            duration: Some(610),
            ..Default::default()
        });

        for (w, h) in [(140u16, 40u16), (96, 30)] {
            let backend = TestBackend::new(w, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| render_detail_screen(f, &mut st, None)).unwrap();

            let buf = term.backend().buffer();
            let text: String = buf.content().iter().map(|c| c.symbol()).collect();
            assert!(text.contains("Chapters (4)"), "Expected 4 parsed chapters");
            assert!(text.contains("Up Next & Channel"), "Expected Up Next drawer");
            assert!(text.contains("27.5K views"), "Expected view count in detail telemetry");
        }
    }
}
