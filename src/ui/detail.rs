//! Bento Video Detail Command Deck (`Screen::Detail`).
//! Layout:
//! - Left Column  : 16:9 Ambilight Artwork + Engagement/Sentiment/SponsorBlock + Chapters (bottom-left)
//! - Center Column: Narrowed Synopsis, Links & Tags + Compact Playback & Mastering Readout
//! - Right Column : Expanded Full-Height YouTube Comments Section with Word-Safe Wrapping & Live Composer

use crate::api::dislike::format_compact_count;
use crate::api::innertube::VideoVote;
use crate::db::cache::{format_duration_secs, VideoChapter, VideoItem};
use crate::state::{
    occluded_by_modal, ActiveOverlay, AppState, CommentNodeHitbox, CommentSort, CommentTreeRow,
    DetailHit,
};
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
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{FEFF}' | '\u{200E}' | '\u{200F}'))
        .collect();
    let normalized = cleaned.replace("\r\n", "\n").replace(['\t', '\u{00a0}'], " ");

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
pub fn truncate_word_boundary(text: &str, max_width: usize) -> String {
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
    state.detail_hitboxes.clear();
    state.detail_comment_hitboxes.clear();
    state.detail_comment_node_hitboxes.clear();

    let Some(video) = state.detail_video.clone() else {
        return;
    };
    let pal = state.active_palette_for(Some(&video.id));
    let area = frame.area();
    let now_unix = unix_now_secs();

    let show_np = state.has_live_ipc() && area.height >= 24;
    let hdr_h: u16 = if area.height >= 22 { 4 } else { 3 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(if show_np {
            vec![
                Constraint::Length(hdr_h),
                Constraint::Min(14),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(hdr_h),
                Constraint::Min(14),
                Constraint::Length(1),
            ]
        })
        .split(area);

    render_header_bar(frame, rows[0], state, &video, &pal, now_unix);

    // Aspect-locked left column matching a 16:9 thumbnail (`~3.55 * inner_rows + 2`)
    let art_inner_h = ((rows[1].height as f32 * 0.25).round() as u16).clamp(5, 8);
    let art_outer_h = (art_inner_h + 2).min(rows[1].height.saturating_sub(14).max(6));
    let left_w = ((art_inner_h as f32 * 3.55).round() as u16 + 2)
        .clamp(30, 38)
        .min((rows[1].width * 29 / 100).max(28));

    if rows[1].width >= 104 {
        let remaining_w = rows[1].width.saturating_sub(left_w);
        let synopsis_w = ((remaining_w as u32 * 35 / 100) as u16).clamp(28, 42);
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
        render_comments_panel(frame, cols[2], state, &video, &pal);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Min(32)])
            .split(rows[1]);

        render_left_column(frame, cols[0], state, &video, &pal, art_outer_h, now_unix, modal_rect);

        let right_stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
            .split(cols[1]);

        render_synopsis_panel(frame, right_stack[0], state, &video, &pal);
        render_comments_panel(frame, right_stack[1], state, &video, &pal);
    }

    if show_np {
        crate::ui::play_options::render_now_playing_bar(frame, rows[2], state);
        render_footer_legend(frame, rows[3], state, &pal);
    } else {
        state.live_playback.now_playing_hitboxes.clear();
        render_footer_legend(frame, rows[2], state, &pal);
    }
}

fn effective_vote_counts(state: &AppState, video: &VideoItem) -> (Option<i64>, Option<i64>, Option<f64>) {
    let base_likes = state
        .detail_dislikes
        .as_ref()
        .map(|d| d.likes)
        .or_else(|| video.like_count.map(|c| c as i64));
    let base_dislikes = state
        .detail_dislikes
        .as_ref()
        .map(|d| d.dislikes)
        .or_else(|| video.dislike_count.map(|c| c as i64));

    let likes = base_likes.map(|l| match state.detail_video_vote {
        VideoVote::Liked => l.saturating_add(1),
        _ => l,
    });
    let dislikes = base_dislikes.map(|d| match state.detail_video_vote {
        VideoVote::Disliked => d.saturating_add(1),
        _ => d,
    });
    let pct = match (likes, dislikes) {
        (Some(l), Some(d)) if (l + d) > 0 => Some((l as f64 / (l + d) as f64) * 100.0),
        _ => state
            .detail_dislikes
            .as_ref()
            .map(|d| d.like_ratio_pct())
            .or_else(|| video.like_ratio_pct()),
    };
    (likes, dislikes, pct)
}

fn render_header_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
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
        .title(Span::styled(
            format!(" Detail Deck · {pane_label} "),
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary));
    let inner = hdr_blk.inner(area);
    frame.render_widget(hdr_blk, area);
    if inner.width < 16 || inner.height == 0 {
        return;
    }

    let inner_w = inner.width as usize;
    let uploader_pill = format!("[ {} > ]", video.display_uploader());
    let uploader_w = uploader_pill.chars().count();
    let views_compact = video.compact_views(true).unwrap_or_default();
    let age_compact = video.compact_age(now_unix, true).unwrap_or_default();
    let meta_tail = [views_compact, age_compact, video.display_duration()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut right_badges_len = meta_tail.chars().count();
    if state.is_in_continue_watching(&video.id) {
        right_badges_len += 11;
    }
    if state.offline_paths.contains_key(&video.id) {
        right_badges_len += 10;
    }
    let prefix_w = if video.is_new_release(now_unix) { 6 } else { 0 };
    let title_budget = inner_w
        .saturating_sub(prefix_w + uploader_w + right_badges_len + 8)
        .max(12);
    let shown_title = marquee_window(&video.title, title_budget, state.ticker_step);

    let mut row1_spans = Vec::new();
    let mut cur_x = inner.x;
    if video.is_new_release(now_unix) {
        row1_spans.push(Span::styled(
            " NEW ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        ));
        row1_spans.push(Span::raw(" "));
        cur_x += 6;
    }
    let title_seg = format!("{shown_title}  ");
    cur_x += title_seg.chars().count() as u16;
    row1_spans.push(Span::styled(
        title_seg,
        Style::default()
            .fg(pal.primary)
            .add_modifier(Modifier::BOLD),
    ));

    let ch_x = cur_x;
    row1_spans.push(Span::styled(
        uploader_pill.clone(),
        Style::default()
            .fg(Color::Black)
            .bg(pal.secondary)
            .add_modifier(Modifier::BOLD),
    ));
    if ch_x < inner.x + inner.width {
        let max_ch_w = (inner.x + inner.width).saturating_sub(ch_x);
        state.detail_hitboxes.push((
            Rect::new(ch_x, inner.y, (uploader_w as u16).min(max_ch_w), 1),
            DetailHit::OpenChannel(video.clone()),
        ));
    }

    if !meta_tail.is_empty() {
        row1_spans.push(Span::styled(
            format!("  {meta_tail}"),
            Style::default().fg(pal.text_dim),
        ));
    }
    if state.is_in_continue_watching(&video.id) {
        row1_spans.push(Span::styled(
            "  [IN QUEUE]",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if state.offline_paths.contains_key(&video.id) {
        row1_spans.push(Span::styled(
            "  [OFFLINE]",
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let mut hdr_lines = vec![Line::from(row1_spans)];

    if inner.height >= 2 {
        let (likes_opt, dislikes_opt, _) = effective_vote_counts(state, video);
        let like_lbl = likes_opt
            .map(|c| format!("[ + Like {} ]", format_compact_count(c)))
            .unwrap_or_else(|| "[ + Like ]".to_string());
        let dislike_lbl = dislikes_opt
            .map(|c| format!("[ - Dislike {} ]", format_compact_count(c)))
            .unwrap_or_else(|| "[ - Dislike ]".to_string());
        let queue_lbl = if state.is_in_continue_watching(&video.id) {
            "[ In Queue ]"
        } else {
            "[ + Queue ]"
        };
        let wl_lbl = if state.watch_later_sources.contains_key(&video.id) {
            "[ Saved WL ]"
        } else {
            "[ Watch Later ]"
        };

        let like_st = if state.detail_video_vote == VideoVote::Liked {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.text).add_modifier(Modifier::BOLD)
        };
        let dislike_st = if state.detail_video_vote == VideoVote::Disliked {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(248, 113, 113))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.text_dim)
        };

        let mut buttons: Vec<(String, Style, DetailHit)> = vec![(
            if video.is_live_stream() {
                "[ Play Live Space ]".to_string()
            } else {
                "[ Play Space ]".to_string()
            },
            Style::default()
                .fg(Color::Black)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD),
            DetailHit::Play,
        )];
        if video.is_live_stream() {
            buttons.push((
                "[ From Start S ]".to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(56, 189, 248))
                    .add_modifier(Modifier::BOLD),
                DetailHit::PlayLiveFromStart,
            ));
            buttons.push((
                "[ Radio R ]".to_string(),
                Style::default()
                    .fg(pal.primary)
                    .add_modifier(Modifier::BOLD),
                DetailHit::PlayAudioRadio,
            ));
            buttons.push((
                format!("[ {} L ]", state.config.live_mode.short_badge()),
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
                DetailHit::CycleLiveMode,
            ));
        }
        buttons.extend([
            (like_lbl, like_st, DetailHit::VoteLike),
            (dislike_lbl, dislike_st, DetailHit::VoteDislike),
            (
                queue_lbl.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                DetailHit::PinQueue,
            ),
            (
                wl_lbl.to_string(),
                Style::default().fg(Color::Rgb(251, 191, 36)),
                DetailHit::WatchLater,
            ),
            (
                "[ Download ]".to_string(),
                Style::default().fg(pal.secondary),
                DetailHit::Download,
            ),
        ]);

        let row2_y = inner.y + 1;
        let mut bx = inner.x;
        let mut r2_spans = Vec::new();
        for (idx, (lbl, st, hit)) in buttons.into_iter().enumerate() {
            if idx > 0 {
                r2_spans.push(Span::raw("  "));
                bx += 2;
            }
            let w = lbl.chars().count() as u16;
            if bx + w <= inner.x + inner.width {
                state
                    .detail_hitboxes
                    .push((Rect::new(bx, row2_y, w, 1), hit));
                r2_spans.push(Span::styled(lbl, st));
                bx += w;
            } else {
                break;
            }
        }
        if let Some((pos, dur)) = state.progress_map.get(&video.id) {
            if *dur > 0.0 {
                let pct = ((*pos / *dur) * 100.0).round() as u32;
                let res_txt = format!("Resume {} · {pct}%", format_duration_secs(*pos as u64));
                let res_w = res_txt.chars().count() as u16;
                if bx + res_w + 2 <= inner.x + inner.width {
                    let gap = (inner.x + inner.width).saturating_sub(bx + res_w);
                    r2_spans.push(Span::raw(" ".repeat(gap as usize)));
                    r2_spans.push(Span::styled(
                        res_txt,
                        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                    ));
                }
            }
        }
        hdr_lines.push(Line::from(r2_spans));
    }

    frame.render_widget(Paragraph::new(hdr_lines), inner);
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
    let show_creator = area.height >= 30;
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_creator {
            vec![
                Constraint::Length(art_outer_h),
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Min(7),
            ]
        } else {
            vec![
                Constraint::Length(art_outer_h),
                Constraint::Length(7),
                Constraint::Min(7),
            ]
        })
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

    // 2. Creator Card (explicit click-through to Channel catalog)
    let (stats_area, ch_area) = if show_creator {
        render_creator_card(frame, left_rows[1], state, video, pal);
        (left_rows[2], left_rows[3])
    } else {
        (left_rows[1], left_rows[2])
    };

    // 3. Engagement, Sentiment, Like/Dislike Pills & SponsorBlock Telemetry
    render_engagement_card(frame, stats_area, state, video, pal, now_unix);

    // 4. Chapters + Dynamic "More from @Creator" auto-fill (zero empty vertical gap)
    render_chapters_and_creator_fill(frame, ch_area, state, video, pal);
}

fn render_creator_card(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let blk = Block::default()
        .title(Span::styled(
            " Creator [c] ",
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 10 || inner.height == 0 {
        return;
    }

    let up_name = video.display_uploader();
    let initial = up_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('Y');
    let known_cnt = state.same_creator_videos(video, 20).len();
    let ch_btn = "[ Open Channel > ]";
    let btn_w = ch_btn.chars().count() as u16;
    let name_w = (inner.width as usize).saturating_sub(btn_w as usize + 8).max(6);
    let shown_name = truncate_word_boundary(up_name, name_w);
    let pad = (inner.width as usize).saturating_sub(6 + shown_name.chars().count() + btn_w as usize);

    let line1 = Line::from(vec![
        Span::styled(
            format!("[{initial}] "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            shown_name,
            Style::default().fg(pal.text).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(pad.max(1))),
        Span::styled(
            ch_btn,
            Style::default()
                .fg(Color::Black)
                .bg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let mut lines = vec![line1];
    if inner.height >= 2 {
        let info = if known_cnt > 0 {
            format!("Click to browse {known_cnt} cached uploads + full catalog")
        } else {
            "Click or press [c] to open full channel catalog".to_string()
        };
        lines.push(Line::from(Span::styled(
            truncate_word_boundary(&info, inner.width as usize),
            Style::default().fg(pal.text_dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    state
        .detail_hitboxes
        .push((area, DetailHit::OpenChannel(video.clone())));
}

fn render_engagement_card(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    video: &VideoItem,
    pal: &ThemePalette,
    now_unix: i64,
) {
    let blk = Block::default()
        .title(Span::styled(
            " Engagement & Votes ",
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let stats_inner = blk.inner(area);
    frame.render_widget(blk, area);
    if stats_inner.width < 12 || stats_inner.height == 0 {
        return;
    }

    let (likes_opt, dislikes_opt, like_pct) = effective_vote_counts(state, video);
    let like_pill = likes_opt
        .map(|l| format!("[u] +{}", format_compact_count(l)))
        .unwrap_or_else(|| "[u] +Like".to_string());
    let dislike_pill = dislikes_opt
        .map(|d| format!("[v] -{}", format_compact_count(d)))
        .unwrap_or_else(|| "[v] -Dislike".to_string());

    let like_st = if state.detail_video_vote == VideoVote::Liked {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(52, 211, 153))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Rgb(52, 211, 153))
            .add_modifier(Modifier::BOLD)
    };
    let dislike_st = if state.detail_video_vote == VideoVote::Disliked {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(248, 113, 113))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(248, 113, 113))
    };

    let lw = like_pill.chars().count() as u16;
    let dw = dislike_pill.chars().count() as u16;
    state.detail_hitboxes.push((
        Rect::new(stats_inner.x, stats_inner.y, lw.min(stats_inner.width), 1),
        DetailHit::VoteLike,
    ));
    let dx = stats_inner.x + lw + 2;
    if dx < stats_inner.x + stats_inner.width {
        state.detail_hitboxes.push((
            Rect::new(
                dx,
                stats_inner.y,
                dw.min((stats_inner.x + stats_inner.width).saturating_sub(dx)),
                1,
            ),
            DetailHit::VoteDislike,
        ));
    }

    let pct_badge = like_pct
        .map(|p| format!("  {:.1}%", p))
        .unwrap_or_default();
    let vote_row = Line::from(vec![
        Span::styled(like_pill, like_st),
        Span::raw("  "),
        Span::styled(dislike_pill, dislike_st),
        Span::styled(pct_badge, Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ]);

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
    let thread_total = state.detail_comments.len();
    let total_replies: usize = state
        .detail_comments
        .iter()
        .map(|c| (c.reply_count as usize).max(c.replies.len()))
        .sum();
    let talk_line = format!("Talk   : {thread_total} threads · {total_replies} replies");

    let stat_lines = vec![
        vote_row,
        build_sentiment_gauge(like_pct, stats_inner.width as usize, pal),
        Line::from(vec![
            Span::styled("Reach  : ", Style::default().fg(pal.text_dim)),
            Span::styled(
                truncate_word_boundary(&format!("{views_str} · {age_str}"), (stats_inner.width as usize).saturating_sub(9)),
                Style::default().fg(pal.text),
            ),
        ]),
        Line::from(vec![
            Span::styled("Runtime: ", Style::default().fg(pal.text_dim)),
            Span::styled(
                truncate_word_boundary(&format!("{} · {}", video.display_duration(), resume_str), (stats_inner.width as usize).saturating_sub(9)),
                Style::default().fg(pal.primary),
            ),
        ]),
        sb_bar,
        Line::from(Span::styled(
            truncate_word_boundary(&sb_label, stats_inner.width as usize),
            Style::default().fg(pal.text_dim),
        )),
        Line::from(Span::styled(
            truncate_word_boundary(&talk_line, stats_inner.width as usize),
            Style::default().fg(pal.secondary),
        )),
    ];
    frame.render_widget(Paragraph::new(stat_lines), stats_inner);
}

fn render_center_column(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    let up_next = state.detail_up_next_videos(6);
    if area.height >= 26 && !up_next.is_empty() {
        let up_h = (up_next.len() as u16 + 2).clamp(5, 9);
        let center_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(9),
                Constraint::Length(up_h),
                Constraint::Length(5),
            ])
            .split(area);
        render_synopsis_panel(frame, center_rows[0], state, video, pal);
        render_up_next_panel(frame, center_rows[1], state, &up_next, pal);
        render_mastering_panel(frame, center_rows[2], state, video, pal);
    } else {
        let center_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(6)])
            .split(area);
        render_synopsis_panel(frame, center_rows[0], state, video, pal);
        render_mastering_panel(frame, center_rows[1], state, video, pal);
    }
}

fn render_up_next_panel(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    items: &[VideoItem],
    pal: &ThemePalette,
) {
    let blk = Block::default()
        .title(Span::styled(
            format!(" Up Next · {} Queued ", items.len()),
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 12 || inner.height == 0 {
        return;
    }

    let max_rows = inner.height as usize;
    let mut lines = Vec::new();
    for (i, item) in items.iter().take(max_rows).enumerate() {
        let y = inner.y + i as u16;
        let dur = item.display_duration();
        let up = item.display_uploader();
        let meta = format!(" {up} · {dur}");
        let meta_w = meta.chars().count();
        let title_w = (inner.width as usize).saturating_sub(meta_w + 3).max(8);
        let t_str = truncate_word_boundary(&item.title, title_w);
        let pad = (inner.width as usize).saturating_sub(2 + t_str.chars().count() + meta_w);
        lines.push(Line::from(vec![
            Span::styled(if i == 0 { "> " } else { "  " }, Style::default().fg(pal.accent)),
            Span::styled(
                t_str,
                Style::default().fg(if i == 0 { pal.primary } else { pal.text }),
            ),
            Span::raw(" ".repeat(pad.max(1))),
            Span::styled(meta.trim_start().to_string(), Style::default().fg(pal.text_dim)),
        ]));
        state.detail_hitboxes.push((
            Rect::new(inner.x, y, inner.width, 1),
            DetailHit::SelectUpNext(item.clone()),
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_synopsis_panel(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
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
    state
        .detail_hitboxes
        .push((area, DetailHit::FocusPane(0)));

    let fallback_desc = if state.detail_metadata_loading {
        "Loading full metadata, chapters, subtitles, and SponsorBlock cues..."
    } else {
        "No description provided for this video."
    };
    let desc_raw = video
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_desc);
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
            " Playback & Mastering ([p] Studio) ",
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
        "Offline Vault".to_string()
    } else if video.is_live_stream() {
        format!(
            "Live ({} · {}s buf)",
            state.config.live_mode.short_badge(),
            state.config.live_mode.readahead_secs()
        )
    } else {
        "Adaptive Live".to_string()
    };

    let third_line = if video.is_live_stream() {
        Line::from(vec![
            Span::styled("Live  : ", Style::default().fg(pal.text_dim)),
            Span::styled(
                format!(
                    "{} · wait {:.1}s [L Cycle · E Edge]",
                    state.config.live_mode.label(),
                    state.config.live_mode.cache_pause_wait_secs()
                ),
                Style::default().fg(Color::Rgb(52, 211, 153)),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("Shader: ", Style::default().fg(pal.text_dim)),
            Span::styled(
                format!("{} · {source_str}", state.config.shader_mode.short_label()),
                Style::default().fg(pal.accent),
            ),
        ])
    };

    let lines = vec![
        Line::from(vec![
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
            Span::styled(" · ", Style::default().fg(pal.divider)),
            Span::styled(
                state.config.performance_mode.short_badge(),
                Style::default().fg(Color::Rgb(56, 189, 248)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(pal.divider)),
            Span::styled(
                state.config.motion_mode.short_badge(),
                Style::default().fg(pal.secondary),
            ),
        ]),
        Line::from(vec![
            Span::styled("Audio : ", Style::default().fg(pal.text_dim)),
            Span::styled(
                format!("{} · Subs {sub_summary}", state.config.dub_preference.label()),
                Style::default().fg(pal.text),
            ),
        ]),
        third_line,
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

fn render_chapters_and_creator_fill(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
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
    state
        .detail_hitboxes
        .push((area, DetailHit::FocusPane(1)));

    if ch_inner.width < 12 || ch_inner.height == 0 {
        return;
    }

    let vis_h = ch_inner.height as usize;
    let creator_more = state.same_creator_videos(video, vis_h);
    let reserve_more = if !creator_more.is_empty() && vis_h > chapters.len() + 2 {
        (vis_h - chapters.len()).min(creator_more.len() + 2)
    } else if chapters.is_empty() && !creator_more.is_empty() {
        vis_h
    } else {
        0
    };
    let chap_rows = vis_h.saturating_sub(reserve_more);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if chapters.is_empty() {
        lines.push(Line::from(Span::styled(
            "No chapters listed in metadata.",
            Style::default().fg(pal.text_dim),
        )));
    } else {
        let cur = state.detail_chapter_cursor.min(chapters.len().saturating_sub(1));
        let scroll_off = if cur >= chap_rows.max(1) {
            cur + 1 - chap_rows.max(1)
        } else {
            0
        };
        let title_budget = (ch_inner.width as usize).saturating_sub(17).max(8);

        for (i, c) in chapters
            .iter()
            .enumerate()
            .skip(scroll_off)
            .take(chap_rows)
        {
            let row_y = ch_inner.y + lines.len() as u16;
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
            lines.push(Line::from(vec![
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
            ]));
            state.detail_hitboxes.push((
                Rect::new(ch_inner.x, row_y, ch_inner.width, 1),
                DetailHit::SelectChapter(i),
            ));
        }
    }

    let remaining = vis_h.saturating_sub(lines.len());
    if remaining >= 2 && !creator_more.is_empty() {
        lines.push(Line::from(Span::styled(
            truncate_word_boundary(
                &format!("More from {} [c]", video.display_uploader()),
                ch_inner.width as usize,
            ),
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        )));
        let max_vids = remaining.saturating_sub(2);
        for v in creator_more.into_iter().take(max_vids) {
            let row_y = ch_inner.y + lines.len() as u16;
            let dur = v.display_duration();
            let tb = (ch_inner.width as usize)
                .saturating_sub(dur.chars().count() + 4)
                .max(6);
            let t = truncate_word_boundary(&v.title, tb);
            let pad = (ch_inner.width as usize)
                .saturating_sub(2 + t.chars().count() + dur.chars().count());
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().fg(pal.text_dim)),
                Span::styled(t, Style::default().fg(pal.text)),
                Span::raw(" ".repeat(pad.max(1))),
                Span::styled(dur, Style::default().fg(pal.text_dim)),
            ]));
            state.detail_hitboxes.push((
                Rect::new(ch_inner.x, row_y, ch_inner.width, 1),
                DetailHit::SelectUpNext(v),
            ));
        }
        if lines.len() < vis_h {
            let row_y = ch_inner.y + lines.len() as u16;
            lines.push(Line::from(Span::styled(
                "  [ View full channel catalog  c ]",
                Style::default()
                    .fg(pal.secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            state.detail_hitboxes.push((
                Rect::new(ch_inner.x, row_y, ch_inner.width, 1),
                DetailHit::OpenChannel(video.clone()),
            ));
        }
    }

    frame.render_widget(Paragraph::new(lines), ch_inner);
}

fn depth_rail_color(depth: usize, pal: &ThemePalette) -> Color {
    match depth % 5 {
        0 => pal.accent,
        1 => pal.secondary,
        2 => Color::Cyan,
        3 => Color::Rgb(168, 85, 247),
        _ => Color::Rgb(52, 211, 153),
    }
}

fn build_node_rail_spans(
    row: &CommentTreeRow,
    pal: &ThemePalette,
    bg: Color,
    is_header: bool,
) -> (Vec<Span<'static>>, usize) {
    if row.depth == 0 {
        return if is_header {
            (Vec::new(), 0)
        } else {
            (
                vec![Span::styled("| ", Style::default().fg(pal.divider).bg(bg))],
                2,
            )
        };
    }

    let mut spans = Vec::with_capacity(row.depth + 1);
    let mut cols = 0usize;
    for k in 0..(row.depth - 1) {
        let active = row.rail_active.get(k).copied().unwrap_or(false);
        let col = depth_rail_color(k, pal);
        if active {
            spans.push(Span::styled("|   ", Style::default().fg(col).bg(bg)));
        } else {
            spans.push(Span::styled("    ", Style::default().bg(bg)));
        }
        cols += 4;
    }

    let cur_col = depth_rail_color(row.depth - 1, pal);
    if is_header {
        let branch = if row.is_last_sibling { "+-- " } else { "|-- " };
        spans.push(Span::styled(branch, Style::default().fg(cur_col).bg(bg)));
        cols += 4;
    } else {
        let cont = if row.is_last_sibling { "    " } else { "|   " };
        spans.push(Span::styled(cont, Style::default().fg(cur_col).bg(bg)));
        spans.push(Span::styled("| ", Style::default().fg(pal.divider).bg(bg)));
        cols += 6;
    }
    (spans, cols)
}

fn render_comments_panel(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    video: &VideoItem,
    pal: &ThemePalette,
) {
    state.detail_comments_panel_rect = area;

    let focused = state.detail_focus_pane == 2
        || state.detail_comment_composing
        || state.detail_translate_editing;
    let total = state.detail_comments.len();
    let title = if state.detail_translate_editing {
        format!(" Comments ({total}) [Translate Language] ")
    } else if state.detail_comment_composing {
        if let Some(ref rt) = state.detail_reply_target {
            format!(" Comments ({total}) [Replying to {}] ", rt.author)
        } else {
            format!(" Comments ({total}) [WRITING · Enter Post · Esc Cancel] ")
        }
    } else {
        format!(" Comments ({total}) ")
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
    state
        .detail_hitboxes
        .push((area, DetailHit::FocusPane(2)));

    if inner.width < 12 || inner.height < 7 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .split(inner);

    state.detail_comment_composer_rect = chunks[0];

    // 1. Composer Box OR Inline Translation Target Language Picker
    let comp_border = if state.detail_translate_editing {
        pal.secondary
    } else if state.detail_comment_composing {
        pal.accent
    } else {
        pal.border_idle
    };
    let comp_title = if state.detail_translate_editing {
        " Translate To Any Language (e.g. Arabic, Japanese, es, fr, ko + Enter) ".to_string()
    } else if state.detail_comment_composing {
        if let Some(ref rt) = state.detail_reply_target {
            format!(" Replying to {} (Enter Post · Esc Cancel) ", rt.author)
        } else {
            " New Comment (Enter Post · Esc Cancel) ".to_string()
        }
    } else if state.auth_vault.active_account().is_some() {
        " [m] Comment · [r] Reply · [t] Translate ".to_string()
    } else {
        " [m] Comment · [r] Reply · [t] Translate (Local save) ".to_string()
    };
    let comp_blk = Block::default()
        .title(Span::styled(
            truncate_word_boundary(&comp_title, chunks[0].width.saturating_sub(2) as usize),
            Style::default()
                .fg(if state.detail_translate_editing {
                    pal.secondary
                } else if state.detail_comment_composing {
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
    let comp_line = if state.detail_translate_editing {
        let total_chars = state.detail_translate_input.chars().count();
        let tail: String = if total_chars > comp_w {
            state
                .detail_translate_input
                .chars()
                .skip(total_chars - comp_w)
                .collect()
        } else {
            state.detail_translate_input.clone()
        };
        Line::from(vec![
            Span::styled("Target: ", Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD)),
            Span::styled(tail, Style::default().fg(pal.text)),
            Span::styled("_", Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD)),
        ])
    } else if state.detail_comment_composing {
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
            Span::styled("_", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(Span::styled(
            truncate_word_boundary(
                "Click or press [m] to comment · [r] reply · [t] translate · [g] pick any language",
                comp_w,
            ),
            Style::default().fg(pal.text_dim),
        ))
    };
    frame.render_widget(Paragraph::new(comp_line), comp_inner);

    // 2. Sort & Translate Language Strip
    let sort_area = chunks[1];
    let top_active = state.detail_comment_sort == CommentSort::Top;
    let top_lbl = "[ Top ]";
    let new_lbl = "[ Newest ]";
    let lang_badge = crate::api::translate::lang_badge(&state.detail_translate_lang);
    let lang_pill = format!("[ Lang: {lang_badge} > ]");
    let tr_sel_lbl = "[ t Translate ]";

    let top_st = if top_active {
        Style::default().fg(Color::Black).bg(pal.primary).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.text_dim)
    };
    let new_st = if !top_active {
        Style::default().fg(Color::Black).bg(pal.primary).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.text_dim)
    };
    let top_w = top_lbl.chars().count() as u16;
    let new_w = new_lbl.chars().count() as u16;
    state.detail_hitboxes.push((
        Rect::new(sort_area.x, sort_area.y, top_w.min(sort_area.width), 1),
        DetailHit::SortComments(CommentSort::Top),
    ));
    let nx = sort_area.x + top_w + 1;
    if nx < sort_area.x + sort_area.width {
        state.detail_hitboxes.push((
            Rect::new(
                nx,
                sort_area.y,
                new_w.min((sort_area.x + sort_area.width).saturating_sub(nx)),
                1,
            ),
            DetailHit::SortComments(CommentSort::Newest),
        ));
    }
    let total_replies: usize = state
        .detail_comments
        .iter()
        .map(|c| (c.reply_count as usize).max(c.replies.len()))
        .sum();
    let count_seg = format!("  {total}·{total_replies}");
    let left_used = top_w + 1 + new_w + count_seg.chars().count() as u16;
    let lang_w = lang_pill.chars().count() as u16;
    let tr_w = tr_sel_lbl.chars().count() as u16;

    let mut sort_spans = vec![
        Span::styled(top_lbl, top_st),
        Span::raw(" "),
        Span::styled(new_lbl, new_st),
        Span::styled(count_seg, Style::default().fg(pal.text_dim)),
    ];

    if left_used + lang_w + tr_w + 2 <= sort_area.width {
        let pad = sort_area.width.saturating_sub(left_used + lang_w + tr_w + 1);
        sort_spans.push(Span::raw(" ".repeat(pad as usize)));
        let tr_x = sort_area.x + left_used + pad;
        state.detail_hitboxes.push((
            Rect::new(tr_x, sort_area.y, tr_w, 1),
            DetailHit::TranslateNode(state.detail_comment_cursor),
        ));
        let lx = tr_x + tr_w + 1;
        state.detail_hitboxes.push((
            Rect::new(lx, sort_area.y, lang_w, 1),
            DetailHit::EditTranslateLang,
        ));
        sort_spans.push(Span::styled(
            tr_sel_lbl,
            Style::default().fg(Color::Rgb(56, 189, 248)).add_modifier(Modifier::BOLD),
        ));
        sort_spans.push(Span::raw(" "));
        sort_spans.push(Span::styled(
            lang_pill,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(56, 189, 248))
                .add_modifier(Modifier::BOLD),
        ));
    } else if left_used + lang_w < sort_area.width {
        let pad = sort_area.width.saturating_sub(left_used + lang_w);
        sort_spans.push(Span::raw(" ".repeat(pad as usize)));
        let lx = sort_area.x + left_used + pad;
        state.detail_hitboxes.push((
            Rect::new(lx, sort_area.y, lang_w, 1),
            DetailHit::EditTranslateLang,
        ));
        sort_spans.push(Span::styled(
            lang_pill,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(56, 189, 248))
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(sort_spans)), sort_area);

    // 3. Flattened Multi-Depth Reddit Tree
    let list_area = chunks[2];
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
            Paragraph::new("No comments returned for this video yet.\nPress [m] to start the discussion.")
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(pal.text_dim)),
            list_area,
        );
        return;
    }

    let flat_rows = state.build_comment_tree();
    if flat_rows.is_empty() {
        return;
    }
    let cur = state.detail_comment_cursor.min(flat_rows.len().saturating_sub(1));
    let creator_low = video.display_uploader().trim().trim_start_matches('@').to_lowercase();
    let vis_h = list_area.height.max(1) as usize;

    struct PendingNodeMeta {
        s_line: usize,
        e_line: usize,
        flat_idx: usize,
        node_id: String,
        root_idx: usize,
        fold_col_range: Option<(u16, u16)>,
        reply_col_range: Option<(u16, u16)>,
        translate_col_range: Option<(u16, u16)>,
    }

    let mut all_lines: Vec<Line<'static>> = Vec::new();
    let mut selected_start_line = 0usize;
    let mut selected_end_line = 0usize;
    let mut node_metas: Vec<PendingNodeMeta> = Vec::with_capacity(flat_rows.len());

    for (flat_idx, node) in flat_rows.iter().enumerate() {
        let s_line = all_lines.len();
        let sel = flat_idx == cur;
        if sel {
            selected_start_line = s_line;
        }
        let row_bg = if sel {
            Color::Rgb(30, 36, 52)
        } else {
            Color::Reset
        };

        let (mut hdr_spans, prefix_w) = build_node_rail_spans(node, pal, row_bg, true);
        let mut cursor_x = prefix_w as u16;
        let c = &node.comment;

        if node.depth == 0 {
            let marker = if sel && node.has_children && !node.collapsed {
                "v "
            } else if sel {
                "> "
            } else if node.has_children && !node.collapsed {
                "v "
            } else {
                "  "
            };
            cursor_x += marker.chars().count() as u16;
            hdr_spans.push(Span::styled(
                marker,
                Style::default()
                    .fg(if sel { pal.primary } else { pal.text_dim })
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let auth_norm = c.author.trim().trim_start_matches('@').to_lowercase();
        let is_creator = !creator_low.is_empty() && auth_norm == creator_low;
        let author_fg = if c.is_self {
            Color::Cyan
        } else if is_creator {
            Color::Rgb(52, 211, 153)
        } else {
            pal.secondary
        };
        cursor_x += c.author.chars().count() as u16;
        hdr_spans.push(Span::styled(
            c.author.clone(),
            Style::default()
                .fg(author_fg)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD),
        ));

        if is_creator {
            let tag = " [CREATOR]";
            cursor_x += tag.chars().count() as u16;
            hdr_spans.push(Span::styled(
                tag,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ));
        } else if c.is_self {
            let tag = " [YOU]";
            cursor_x += tag.chars().count() as u16;
            hdr_spans.push(Span::styled(
                tag,
                Style::default()
                    .fg(Color::Cyan)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if c.is_pinned {
            let tag = " [PINNED]";
            cursor_x += tag.chars().count() as u16;
            hdr_spans.push(Span::styled(
                tag,
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if !c.published_text.is_empty() {
            let pub_s = format!(" · {}", c.published_text);
            cursor_x += pub_s.chars().count() as u16;
            hdr_spans.push(Span::styled(
                pub_s,
                Style::default().fg(pal.text_dim).bg(row_bg),
            ));
        }

        let clean_likes = c.like_count_text.trim();
        if !clean_likes.is_empty() && clean_likes != "0" {
            let lk = format!(" · {clean_likes} likes");
            cursor_x += lk.chars().count() as u16;
            hdr_spans.push(Span::styled(
                lk,
                Style::default().fg(pal.accent).bg(row_bg),
            ));
        }

        let mut fold_col_range = None;
        if node.has_children {
            hdr_spans.push(Span::styled(" ", Style::default().bg(row_bg)));
            cursor_x += 1;
            let fold_lbl = if c.replies_loading {
                "[Loading...]".to_string()
            } else if node.collapsed {
                format!("[+ {}]", node.child_count.max(1))
            } else {
                "[-]".to_string()
            };
            let fw = fold_lbl.chars().count() as u16;
            fold_col_range = Some((cursor_x, fw));
            cursor_x += fw;
            hdr_spans.push(Span::styled(
                fold_lbl,
                Style::default()
                    .fg(pal.secondary)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let mut reply_col_range = None;
        let mut translate_col_range = None;
        if sel {
            hdr_spans.push(Span::styled(" ", Style::default().bg(row_bg)));
            cursor_x += 1;
            let rep_lbl = "[Reply r]";
            let rw = rep_lbl.chars().count() as u16;
            reply_col_range = Some((cursor_x, rw));
            cursor_x += rw;
            hdr_spans.push(Span::styled(
                rep_lbl,
                Style::default()
                    .fg(Color::Black)
                    .bg(pal.accent)
                    .add_modifier(Modifier::BOLD),
            ));

            hdr_spans.push(Span::styled(" ", Style::default().bg(row_bg)));
            cursor_x += 1;
            let is_translated = state.detail_comment_translations.contains_key(&c.id);
            let tr_lbl = if is_translated { "[Orig t]" } else { "[Translate t]" };
            let tw = tr_lbl.chars().count() as u16;
            translate_col_range = Some((cursor_x, tw));
            hdr_spans.push(Span::styled(
                tr_lbl,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(56, 189, 248))
                    .add_modifier(Modifier::BOLD),
            ));
        }

        all_lines.push(Line::from(hdr_spans));

        let (_, body_prefix_cols) = build_node_rail_spans(node, pal, row_bg, false);
        let body_w = (list_area.width as usize)
            .saturating_sub(body_prefix_cols + 1)
            .max(12);
        if state.detail_comment_translating.contains(&c.id) {
            for wrapped in wrap_words(&c.text, body_w) {
                let (mut body_spans, _) = build_node_rail_spans(node, pal, row_bg, false);
                body_spans.push(Span::styled(
                    wrapped,
                    Style::default().fg(pal.text_dim).bg(row_bg),
                ));
                all_lines.push(Line::from(body_spans));
            }
            let (mut tr_spans, _) = build_node_rail_spans(node, pal, row_bg, false);
            tr_spans.push(Span::styled(
                format!(
                    "[Translating to {}...]",
                    crate::api::translate::lang_badge(&state.detail_translate_lang)
                ),
                Style::default()
                    .fg(Color::Rgb(56, 189, 248))
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            all_lines.push(Line::from(tr_spans));
        } else if let Some(tr) = state.detail_comment_translations.get(&c.id) {
            let src_tag = tr
                .source_lang
                .as_deref()
                .map(crate::api::translate::lang_badge)
                .map(|s| format!(" <- {s}"))
                .unwrap_or_default();
            let tgt_tag = crate::api::translate::lang_badge(&tr.target_lang);
            let header_tag = format!("[{tgt_tag}{src_tag}]");
            let (mut tag_spans, _) = build_node_rail_spans(node, pal, row_bg, false);
            tag_spans.push(Span::styled(
                header_tag,
                Style::default()
                    .fg(Color::Rgb(56, 189, 248))
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            all_lines.push(Line::from(tag_spans));
            for wrapped in wrap_words(&tr.text, body_w) {
                let (mut body_spans, _) = build_node_rail_spans(node, pal, row_bg, false);
                body_spans.push(Span::styled(
                    wrapped,
                    Style::default()
                        .fg(if sel { pal.text } else { Color::Rgb(218, 232, 248) })
                        .bg(row_bg),
                ));
                all_lines.push(Line::from(body_spans));
            }
        } else {
            for wrapped in wrap_words(&c.text, body_w) {
                let (mut body_spans, _) = build_node_rail_spans(node, pal, row_bg, false);
                body_spans.push(Span::styled(
                    wrapped,
                    Style::default()
                        .fg(if sel {
                            pal.text
                        } else if node.depth == 0 {
                            Color::Rgb(210, 216, 228)
                        } else {
                            Color::Rgb(192, 200, 214)
                        })
                        .bg(row_bg),
                ));
                all_lines.push(Line::from(body_spans));
            }
        }

        if node.depth == 0 && !node.collapsed {
            if c.replies_loading && c.replies.is_empty() {
                all_lines.push(Line::from(vec![
                    Span::styled("+-- ", Style::default().fg(pal.accent)),
                    Span::styled("Loading reply thread...", Style::default().fg(pal.text_dim)),
                ]));
            } else if c.replies_loading {
                all_lines.push(Line::from(vec![
                    Span::styled("+-- ", Style::default().fg(pal.accent)),
                    Span::styled("Loading more replies...", Style::default().fg(pal.text_dim)),
                ]));
            } else if c.replies_continuation.is_some() {
                all_lines.push(Line::from(vec![
                    Span::styled("+-- ", Style::default().fg(pal.accent)),
                    Span::styled(
                        "Press [n] to load more replies",
                        Style::default().fg(pal.text_dim),
                    ),
                ]));
            }
        }

        let next_is_new_root = flat_rows
            .get(flat_idx + 1)
            .map(|nr| nr.depth == 0)
            .unwrap_or(true);
        if next_is_new_root {
            all_lines.push(Line::from(Span::styled(
                "─".repeat((list_area.width as usize).saturating_sub(1).min(56)),
                Style::default().fg(pal.divider),
            )));
        }

        let e_line = all_lines.len().saturating_sub(1);
        if sel {
            selected_end_line = e_line;
        }
        node_metas.push(PendingNodeMeta {
            s_line,
            e_line,
            flat_idx,
            node_id: c.id.clone(),
            root_idx: node.root_idx,
            fold_col_range,
            reply_col_range,
            translate_col_range,
        });
    }

    let scroll_y = if selected_end_line >= vis_h {
        (selected_end_line + 1)
            .saturating_sub(vis_h)
            .min(selected_start_line)
    } else {
        0
    } as u16;

    let sy = scroll_y as usize;
    for m in node_metas {
        if m.e_line >= sy && m.s_line < sy + vis_h {
            let y0 = list_area.y + m.s_line.saturating_sub(sy) as u16;
            let y1 = list_area.y + m.e_line.saturating_sub(sy).min(vis_h.saturating_sub(1)) as u16;
            state.detail_comment_hitboxes.push((y0, y1, m.root_idx));

            let fold_rect = m.fold_col_range.and_then(|(off_x, w)| {
                let rx = list_area.x + off_x;
                if rx < list_area.x + list_area.width {
                    Some(Rect::new(rx, y0, w.min((list_area.x + list_area.width) - rx), 1))
                } else {
                    None
                }
            });
            let reply_rect = m.reply_col_range.and_then(|(off_x, w)| {
                let rx = list_area.x + off_x;
                if rx < list_area.x + list_area.width {
                    Some(Rect::new(rx, y0, w.min((list_area.x + list_area.width) - rx), 1))
                } else {
                    None
                }
            });
            let translate_rect = m.translate_col_range.and_then(|(off_x, w)| {
                let rx = list_area.x + off_x;
                if rx < list_area.x + list_area.width {
                    Some(Rect::new(rx, y0, w.min((list_area.x + list_area.width) - rx), 1))
                } else {
                    None
                }
            });
            state.detail_comment_node_hitboxes.push(CommentNodeHitbox {
                y0,
                y1,
                flat_idx: m.flat_idx,
                node_id: m.node_id,
                fold_rect,
                reply_rect,
                translate_rect,
            });
        }
    }

    frame.render_widget(Paragraph::new(all_lines).scroll((scroll_y, 0)), list_area);
}

fn render_footer_legend(frame: &mut Frame, area: Rect, state: &AppState, pal: &ThemePalette) {
    let enter_hint = match state.detail_focus_pane {
        1 => "[Enter] Play Chapter",
        2 => "[r] Reply · [t] Translate · [g] Lang · [h/l] Fold",
        _ => "[Enter/Space] Play",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{enter_hint} "),
                Style::default()
                    .fg(pal.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("· [u/v] Vote ", Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled("· [c] Channel ", Style::default().fg(pal.secondary)),
            Span::styled("· [Tab] Focus ", Style::default().fg(pal.primary)),
            Span::styled("· [p] Studio ", Style::default().fg(pal.text)),
            Span::styled("· [a] +Queue · [w] +WL · [Esc] Back", Style::default().fg(pal.text_dim)),
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
        st.detail_video_vote = VideoVote::Liked;
        st.detail_comments = vec![VideoComment {
            id: "cmt1".to_string(),
            author: "@viewer_one".to_string(),
            published_text: "1 hour ago".to_string(),
            text: "This comment should wrap cleanly between whole words with zero concatenation issues anywhere.".to_string(),
            like_count_text: "42".to_string(),
            reply_count: 2,
            is_pinned: true,
            is_self: false,
            replies_expanded: true,
            replies: vec![
                VideoComment {
                    id: "rep1".to_string(),
                    author: "@reply_author".to_string(),
                    published_text: "20m ago".to_string(),
                    text: "Here is a nested reply in the thread.".to_string(),
                    like_count_text: "5".to_string(),
                    ..Default::default()
                },
                VideoComment {
                    id: "rep2".to_string(),
                    author: "@sub_replier".to_string(),
                    published_text: "10m ago".to_string(),
                    text: "@reply_author Replying directly to your sub-comment cleanly!".to_string(),
                    like_count_text: "0".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];

        for (w, h) in [(140u16, 42u16), (96, 30)] {
            let backend = TestBackend::new(w, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| render_detail_screen(f, &mut st, None)).unwrap();

            let buf = term.backend().buffer();
            let text: String = buf.content().iter().map(|c| c.symbol()).collect();
            assert!(text.contains("Chapters (4)"), "Expected 4 parsed chapters in left column");
            assert!(text.contains("Comments (1)"), "Expected Comments panel");
            assert!(!text.contains("Reddit Tree"), "Header must not show Reddit Tree label");
            assert!(text.contains("@viewer_one"), "Expected author inside Comments panel");
            assert!(text.contains("@reply_author"), "Expected nested reply author inside expanded thread");
            assert!(text.contains("@sub_replier"), "Expected level-2 sub-reply in Reddit tree");
            assert!(!text.contains("·  likes"), "0-like sub-comment must not render blank likes label");
            assert!(!st.detail_comment_node_hitboxes.is_empty(), "Expected tree node hitboxes");
            assert!(st.detail_hitboxes.iter().any(|(_, h)| matches!(h, DetailHit::VoteLike)));
            assert!(st.detail_hitboxes.iter().any(|(_, h)| matches!(h, DetailHit::OpenChannel(_))));
            assert!(!text.contains("concatenationissues"), "Words must never concatenate");
            assert!(text.contains("27.5K"), "Expected compact views/likes telemetry");
        }
    }
}
