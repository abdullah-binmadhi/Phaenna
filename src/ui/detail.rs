//! Bento Video Detail Command Deck (`Screen::Detail`).
//! Layout:
//! - Left Column  : 16:9 Ambilight Artwork + Engagement/Sentiment/SponsorBlock + Chapters (bottom-left)
//! - Center Column: Narrowed Synopsis, Links & Tags + Compact Playback & Mastering Readout
//! - Right Column : Expanded Full-Height YouTube Comments Section with Word-Safe Wrapping & Live Composer

use crate::api::dislike::format_compact_count;
use crate::db::cache::{format_duration_secs, VideoChapter, VideoItem};
use crate::state::{occluded_by_modal, ActiveOverlay, AppState};
use crate::ui::components::cover::{render_thumbnail_cover, unix_now_secs};
use crate::ui::home::marquee_window;
use crate::ui::theme::ThemePalette;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

/// Wrap text strictly on word boundaries so words never concatenate across line wraps.
pub fn wrap_words(text: &str, max_width: usize) -> Vec<String> {
    let limit = max_width.max(8);
    let mut out = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace(['\t', '\u{00a0}'], " ");

    for raw_line in normalized.split('\n') {
        let words: Vec<&str> = raw_line.split_whitespace().collect();
        if words.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut curr_len = 0usize;

        for word in words {
            let wlen = word.chars().count();
            if current.is_empty() {
                if wlen <= limit {
                    current.push_str(word);
                    curr_len = wlen;
                } else {
                    let mut chunk = String::new();
                    let mut clen = 0usize;
                    for ch in word.chars() {
                        chunk.push(ch);
                        clen += 1;
                        if clen >= limit {
                            out.push(std::mem::take(&mut chunk));
                            clen = 0;
                        }
                    }
                    if !chunk.is_empty() {
                        current = chunk;
                        curr_len = clen;
                    }
                }
            } else if curr_len + 1 + wlen <= limit {
                current.push(' ');
                current.push_str(word);
                curr_len += 1 + wlen;
            } else {
                out.push(std::mem::take(&mut current));
                curr_len = 0;
                if wlen <= limit {
                    current.push_str(word);
                    curr_len = wlen;
                } else {
                    let mut chunk = String::new();
                    let mut clen = 0usize;
                    for ch in word.chars() {
                        chunk.push(ch);
                        clen += 1;
                        if clen >= limit {
                            out.push(std::mem::take(&mut chunk));
                            clen = 0;
                        }
                    }
                    if !chunk.is_empty() {
                        current = chunk;
                        curr_len = clen;
                    }
                }
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Truncate cleanly at a word boundary when text exceeds `max_width`.
fn truncate_word_boundary(text: &str, max_width: usize) -> String {
    let limit = max_width.max(4);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let budget = limit.saturating_sub(3);
    let prefix: String = collapsed.chars().take(budget).collect();
    if let Some(last_sp) = prefix.rfind(' ') {
        if last_sp >= budget / 3 {
            return format!("{}...", &prefix[..last_sp]);
        }
    }
    format!("{prefix}...")
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
    let w = width.min(48);
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
    max_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut urls = Vec::new();
    let mut hashtags = Vec::new();

    for wrapped_line in wrap_words(desc, max_width) {
        if wrapped_line.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let mut spans = Vec::new();
        for (idx, tok) in wrapped_line.split(' ').enumerate() {
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
                for uline in wrap_words(&u, max_width.saturating_sub(4).max(12)) {
                    lines.push(Line::from(vec![
                        Span::styled("  > ", Style::default().fg(pal.accent)),
                        Span::styled(uline, Style::default().fg(pal.secondary)),
                    ]));
                }
            }
        }
        if !hashtags.is_empty() {
            let joined = hashtags
                .into_iter()
                .map(|h| format!("[{h}]"))
                .collect::<Vec<_>>()
                .join(" ");
            for tline in wrap_words(&format!("TAGS: {joined}"), max_width) {
                lines.push(Line::from(Span::styled(
                    tline,
                    Style::default().fg(pal.primary),
                )));
            }
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
    let now_unix = unix_now_secs();

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
    let art_inner_h = ((rows[1].height as f32 * 0.30).round() as u16).clamp(7, 10);
    let art_outer_h = (art_inner_h + 2).min(rows[1].height.saturating_sub(14).max(7));
    let left_w = ((art_inner_h as f32 * 3.55).round() as u16 + 2)
        .clamp(30, 38)
        .min((rows[1].width * 27 / 100).max(28));

    if rows[1].width >= 104 {
        let remaining_w = rows[1].width.saturating_sub(left_w);
        // Narrow synopsis panel (~36% of remaining) so the Comments section gets ~64% priority width
        let synopsis_w = ((remaining_w as u32 * 36 / 100) as u16).clamp(28, 44);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w),
                Constraint::Length(synopsis_w),
                Constraint::Min(38),
            ])
            .split(rows[1]);

        render_left_column(frame, cols[0], state, &video, &pal, art_outer_h, now_unix, modal_rect);
        render_center_column(frame, cols[1], state, &video, &pal);
        render_comments_panel(frame, cols[2], state, &pal);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Min(32)])
            .split(rows[1]);

        render_left_column(frame, cols[0], state, &video, &pal, art_outer_h, now_unix, modal_rect);

        let right_stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(cols[1]);

        render_synopsis_panel(frame, right_stack[0], state, &video, &pal);
        render_comments_panel(frame, right_stack[1], state, &pal);
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
        2 => "FOCUS: COMMENTS",
        _ => "FOCUS: SYNOPSIS",
    };

    let hdr_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary));
    let inner_w = area.width.saturating_sub(2) as usize;

    let mut suffix_len = video.display_uploader().chars().count() + 18 + pane_label.len();
    if video.is_new_release(now_unix) {
        suffix_len += 6;
    }
    if state.is_in_continue_watching(&video.id) {
        suffix_len += 11;
    }
    if state.offline_paths.contains_key(&video.id) {
        suffix_len += 10;
    }
    let title_budget = inner_w.saturating_sub(suffix_len + 6).max(16);
    let shown_title = marquee_window(&video.title, title_budget, state.ticker_step);

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
        format!(" > {shown_title} "),
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
            Constraint::Length(8),
            Constraint::Min(8),
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

    // 2. Engagement, Sentiment & Compact SponsorBlock Telemetry Card
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
            format!("Resume {} ({}%)", format_duration_secs(*p as u64), pct)
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
            format!("{} likes (syncing...)", format_compact_count(l as i64)),
        )
    } else {
        (None, "Fetching ReturnYouTubeDislike...".to_string())
    };

    let total_dur = video.duration.unwrap_or(600) as f64;
    let sb_bar = render_sponsor_timeline_line(state, total_dur, stats_inner.width as usize);
    let sb_label = if state.detail_sponsor_segments.is_empty() {
        "SB Clean (0 segments)".to_string()
    } else {
        format!(
            "SB: {} segs ({})",
            state.detail_sponsor_segments.len(),
            state.config.sponsorblock_mode.label()
        )
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
        sb_bar,
        Line::from(Span::styled(sb_label, Style::default().fg(pal.text_dim))),
    ];
    frame.render_widget(Paragraph::new(stat_lines).wrap(Wrap { trim: true }), stats_inner);

    // 3. Chapters Panel (relocated to bottom-left in place of SponsorBlock + Deck Controls)
    render_chapters_panel(frame, left_rows[2], state, video, pal);
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
        .constraints([Constraint::Min(10), Constraint::Length(6)])
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
                " Synopsis [Focused · j/k] "
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
    let inner = desc_blk.inner(area);
    frame.render_widget(desc_blk, area);

    let desc_raw = video
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("Loading full metadata, chapters, subtitles, and SponsorBlock cues...");
    let lines = style_description_lines(desc_raw, video, pal, inner.width.max(12) as usize);

    frame.render_widget(
        Paragraph::new(lines).scroll((state.detail_scroll, 0)),
        inner,
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
            " Mastering ([p] Override) ",
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let sub_summary = if state.detail_subtitles.is_empty() {
        "Auto".to_string()
    } else {
        let sel = state
            .detail_subtitles
            .get(state.detail_selected_sub_idx)
            .map(|t| t.label.as_str())
            .unwrap_or("Default");
        format!("{} ({sel})", state.detail_subtitles.len())
    };
    let source_str = if state.offline_paths.contains_key(&video.id) {
        "Offline Vault"
    } else {
        "Adaptive Live"
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Player: ", Style::default().fg(pal.text_dim)),
            Span::styled(
                state.config.player.as_str().to_uppercase(),
                Style::default()
                    .fg(pal.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(pal.divider)),
            Span::styled(
                state.config.quality.short_badge(),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Shader: ", Style::default().fg(pal.text_dim)),
            Span::styled(
                state.config.shader_mode.short_label(),
                Style::default().fg(pal.secondary),
            ),
        ]),
        Line::from(vec![
            Span::styled("Subs  : ", Style::default().fg(pal.text_dim)),
            Span::styled(sub_summary, Style::default().fg(pal.text)),
        ]),
        Line::from(vec![
            Span::styled("Mode  : ", Style::default().fg(pal.text_dim)),
            Span::styled(source_str, Style::default().fg(pal.accent)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
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
                format!(" Chapters ({}) [Focused · Enter] ", chapters.len())
            } else {
                format!(" Chapters ({}) [Tab] ", chapters.len())
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
            Paragraph::new("No chapters found in video metadata or description.")
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
            let short_t = if sel {
                marquee_window(&c.title, title_budget, state.ticker_step)
            } else {
                truncate_word_boundary(&c.title, title_budget)
            };
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

fn render_comments_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    pal: &ThemePalette,
) {
    let focused = state.detail_focus_pane == 2 || state.detail_comment_composing;
    let total = state.detail_comments.len();
    let title = if state.detail_comment_composing {
        format!(" Comments ({total}) [WRITING · Enter Post · Esc Cancel] ")
    } else if focused {
        format!(" Comments ({total}) [Focused · j/k · Enter/r Replies · m Write] ")
    } else {
        format!(" Comments ({total}) [Tab / m Write] ")
    };

    let blk = Block::default()
        .title(Span::styled(
            title,
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

    if inner.width < 12 || inner.height < 6 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(inner);

    // Top Interactive Comment Composer Box
    let comp_border = if state.detail_comment_composing {
        pal.accent
    } else {
        pal.border_idle
    };
    let comp_title = if state.detail_comment_composing {
        " New Comment (Enter to post, Esc to cancel) "
    } else if state.auth_vault.active_account().is_some() {
        " Add Comment [m] "
    } else {
        " Add Comment [m] (Saved locally · Sign in [L] for YouTube sync) "
    };
    let comp_blk = Block::default()
        .title(Span::styled(
            comp_title,
            Style::default()
                .fg(if state.detail_comment_composing {
                    pal.accent
                } else {
                    pal.text_dim
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(comp_border));
    let comp_inner = comp_blk.inner(chunks[0]);
    frame.render_widget(comp_blk, chunks[0]);

    let comp_w = comp_inner.width.saturating_sub(2).max(10) as usize;
    let comp_line = if state.detail_comment_composing {
        let total_chars = state.detail_comment_input.chars().count();
        let tail: String = if total_chars > comp_w {
            state
                .detail_comment_input
                .chars()
                .skip(total_chars - comp_w)
                .collect()
        } else {
            state.detail_comment_input.clone()
        };
        Line::from(vec![
            Span::styled(tail, Style::default().fg(pal.text)),
            Span::styled("█", Style::default().fg(pal.accent)),
        ])
    } else {
        Line::from(Span::styled(
            truncate_word_boundary(
                "Press [m] to write a comment, or [Tab] then [j/k] to read full threads...",
                comp_w,
            ),
            Style::default().fg(pal.text_dim),
        ))
    };
    frame.render_widget(Paragraph::new(comp_line), comp_inner);

    // Body: Word-Safe Comment List
    let list_area = chunks[1];
    if state.detail_comments_loading && state.detail_comments.is_empty() {
        frame.render_widget(
            Paragraph::new("Fetching YouTube comments...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.text_dim)),
            list_area,
        );
        return;
    }

    if state.detail_comments.is_empty() {
        frame.render_widget(
            Paragraph::new("No comments returned for this video yet.\nPress [m] to post the first note/comment.")
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(pal.text_dim)),
            list_area,
        );
        return;
    }

    let cur = state
        .detail_comment_cursor
        .min(state.detail_comments.len().saturating_sub(1));
    let body_w = (list_area.width as usize).saturating_sub(5).max(14);
    let vis_h = list_area.height.max(1) as usize;

    let mut all_lines: Vec<Line<'static>> = Vec::new();
    let mut selected_start_line = 0usize;
    let mut selected_end_line = 0usize;

    for (idx, c) in state.detail_comments.iter().enumerate() {
        let sel = idx == cur;
        if sel {
            selected_start_line = all_lines.len();
        }
        let header_bg = if sel {
            Color::Rgb(30, 36, 52)
        } else {
            Color::Reset
        };
        let marker = if sel { "▸ " } else { "  " };

        let mut hdr_spans = vec![
            Span::styled(
                marker,
                Style::default()
                    .fg(if sel { pal.primary } else { pal.text_dim })
                    .bg(header_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                c.author.clone(),
                Style::default()
                    .fg(if c.is_self { Color::Cyan } else { pal.secondary })
                    .bg(header_bg)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !c.published_text.is_empty() {
            hdr_spans.push(Span::styled(
                format!(" · {}", c.published_text),
                Style::default().fg(pal.text_dim).bg(header_bg),
            ));
        }
        if !c.like_count_text.is_empty() && c.like_count_text != "0" {
            hdr_spans.push(Span::styled(
                format!(" · {} likes", c.like_count_text),
                Style::default().fg(pal.accent).bg(header_bg),
            ));
        }
        if c.reply_count > 0 || !c.replies.is_empty() || c.replies_continuation.is_some() {
            let shown_cnt = c.reply_count.max(c.replies.len() as u32);
            let toggle = if c.replies_expanded { "[-]" } else { "[+]" };
            hdr_spans.push(Span::styled(
                format!(" · {shown_cnt} replies {toggle}"),
                Style::default().fg(pal.secondary).bg(header_bg),
            ));
        }
        if c.is_self {
            hdr_spans.push(Span::styled(
                " [YOU]",
                Style::default()
                    .fg(Color::Cyan)
                    .bg(header_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if c.is_pinned {
            hdr_spans.push(Span::styled(
                " [PINNED]",
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .bg(header_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        all_lines.push(Line::from(hdr_spans));

        for wrapped in wrap_words(&c.text, body_w) {
            all_lines.push(Line::from(vec![
                Span::styled("   ", Style::default().bg(header_bg)),
                Span::styled(
                    wrapped,
                    Style::default()
                        .fg(if sel { pal.text } else { Color::Rgb(205, 212, 224) })
                        .bg(header_bg),
                ),
            ]));
        }

        if c.replies_expanded {
            let reply_w = body_w.saturating_sub(4).max(12);
            if c.replies_loading && c.replies.is_empty() {
                all_lines.push(Line::from(vec![
                    Span::styled("   |- ", Style::default().fg(pal.accent)),
                    Span::styled(
                        "Loading reply thread...",
                        Style::default().fg(pal.text_dim),
                    ),
                ]));
            } else if c.replies.is_empty() {
                all_lines.push(Line::from(vec![
                    Span::styled("   |- ", Style::default().fg(pal.divider)),
                    Span::styled(
                        "No replies returned for this thread.",
                        Style::default().fg(pal.text_dim),
                    ),
                ]));
            } else {
                for r in &c.replies {
                    let mut r_hdr = vec![
                        Span::styled("   |- ", Style::default().fg(pal.accent)),
                        Span::styled(
                            r.author.clone(),
                            Style::default()
                                .fg(if r.is_self { Color::Cyan } else { pal.secondary })
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if !r.published_text.is_empty() {
                        r_hdr.push(Span::styled(
                            format!(" · {}", r.published_text),
                            Style::default().fg(pal.text_dim),
                        ));
                    }
                    if !r.like_count_text.is_empty() && r.like_count_text != "0" {
                        r_hdr.push(Span::styled(
                            format!(" · {} likes", r.like_count_text),
                            Style::default().fg(pal.accent),
                        ));
                    }
                    all_lines.push(Line::from(r_hdr));

                    for rw in wrap_words(&r.text, reply_w) {
                        all_lines.push(Line::from(vec![
                            Span::styled("   |  ", Style::default().fg(pal.divider)),
                            Span::styled(rw, Style::default().fg(Color::Rgb(195, 204, 218))),
                        ]));
                    }
                }
                if c.replies_loading {
                    all_lines.push(Line::from(vec![
                        Span::styled("   |- ", Style::default().fg(pal.accent)),
                        Span::styled("Loading more replies...", Style::default().fg(pal.text_dim)),
                    ]));
                } else if c.replies_continuation.is_some() {
                    all_lines.push(Line::from(vec![
                        Span::styled("   |- ", Style::default().fg(pal.accent)),
                        Span::styled(
                            "Press [n] to load more replies",
                            Style::default().fg(pal.text_dim),
                        ),
                    ]));
                }
            }
        }

        all_lines.push(Line::from(Span::styled(
            "  ".to_string() + &"─".repeat(body_w.min(44)),
            Style::default().fg(pal.divider),
        )));

        if sel {
            selected_end_line = all_lines.len().saturating_sub(1);
        }
    }

    let scroll_y = if selected_end_line >= vis_h {
        (selected_end_line + 1)
            .saturating_sub(vis_h)
            .min(selected_start_line)
    } else {
        0
    } as u16;

    frame.render_widget(Paragraph::new(all_lines).scroll((scroll_y, 0)), list_area);
}

fn render_footer_legend(frame: &mut Frame, area: Rect, state: &AppState, pal: &ThemePalette) {
    let enter_hint = match state.detail_focus_pane {
        1 => "[Enter] Play Chapter",
        2 => "[Enter/r] Replies · [m] Comment",
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
            Span::styled("· [Tab] Focus ", Style::default().fg(pal.secondary)),
            Span::styled("· [j/k] Scroll ", Style::default().fg(pal.primary)),
            Span::styled("· [m] Comment ", Style::default().fg(Color::Cyan)),
            Span::styled("· [p] Options ", Style::default().fg(pal.text)),
            Span::styled("· [c] Channel ", Style::default().fg(pal.text)),
            Span::styled("· [a] +Queue ", Style::default().fg(Color::Cyan)),
            Span::styled("· [w] +WL ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("· [D] Offline · [Esc] Back", Style::default().fg(pal.text_dim)),
        ]))
        .alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::innertube::VideoComment;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_wrap_words_never_concatenates_words() {
        let input = "This is a realistic YouTube comment explaining why the driver layout fits nicely without concatenating words.";
        let wrapped = wrap_words(input, 28);
        assert!(wrapped.len() >= 3);
        for line in &wrapped {
            assert!(line.chars().count() <= 28, "Line exceeded limit: {line}");
        }
        assert_eq!(wrapped.join(" "), input);
    }

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
        st.detail_comments = vec![VideoComment {
            id: "cmt1".to_string(),
            author: "@viewer_one".to_string(),
            published_text: "1 hour ago".to_string(),
            text: "This comment should wrap cleanly between whole words with zero concatenation issues anywhere.".to_string(),
            like_count_text: "42".to_string(),
            reply_count: 1,
            is_pinned: true,
            is_self: false,
            replies_expanded: true,
            replies: vec![VideoComment {
                id: "rep1".to_string(),
                author: "@reply_author".to_string(),
                published_text: "20m ago".to_string(),
                text: "Here is a nested reply in the thread.".to_string(),
                like_count_text: "5".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }];

        for (w, h) in [(140u16, 40u16), (96, 30)] {
            let backend = TestBackend::new(w, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| render_detail_screen(f, &mut st, None)).unwrap();

            let buf = term.backend().buffer();
            let text: String = buf.content().iter().map(|c| c.symbol()).collect();
            assert!(text.contains("Chapters (4)"), "Expected 4 parsed chapters in left column");
            assert!(text.contains("Comments (1)"), "Expected Comments panel");
            assert!(text.contains("@viewer_one"), "Expected author inside Comments panel");
            assert!(text.contains("@reply_author"), "Expected nested reply author inside expanded thread");
            assert!(!text.contains("concatenationissues"), "Words must never concatenate");
            assert!(text.contains("27.5K views"), "Expected view count in detail telemetry");
        }
    }
}
