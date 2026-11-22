//! Next-Gen Full-Canvas Search Studio (`/`):
//! - Hybrid RRF Relevance + YouTube InnerTube Structured Parser + Power Operators
//! - Zero-Fail Priority Thumbnail Pipeline (16:9 Card Grid & Cinema Split Inspector)
//! - Channel Spotlight, Word-Wrapped Titles (zero mid-word chop), Match Snippets, & Direct URL Jump.

use crate::db::cache::{extract_yt_url_id, VideoItem};
use crate::state::{
    active_modal_rect, ActiveOverlay, AppState, SearchDateFilter, SearchDurationFilter,
    SearchFocus, SearchHit, SearchScope, SearchSortOrder, SearchTypeFilter, SearchViewMode,
    ThumbPriority,
};
use crate::ui::components::cover::{
    render_thumbnail_cover, render_video_preview_inspector, unix_now_secs,
};
use crate::ui::detail::{truncate_word_boundary, wrap_words};
use crate::ui::theme::ThemePalette;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

fn render_pill_strip(
    frame: &mut Frame,
    area: Rect,
    spans_and_hits: Vec<(String, Style, Option<SearchHit>)>,
    hitboxes: &mut Vec<(Rect, SearchHit)>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut cur_x = area.x;
    let right_limit = area.x + area.width;
    let mut line_spans = Vec::with_capacity(spans_and_hits.len());
    for (text, style, hit) in spans_and_hits {
        let len = text.chars().count() as u16;
        if len == 0 {
            continue;
        }
        if cur_x + len > right_limit {
            break;
        }
        if let Some(action) = hit {
            hitboxes.push((Rect::new(cur_x, area.y, len, 1), action));
        }
        cur_x += len;
        line_spans.push(Span::styled(text, style));
    }
    frame.render_widget(Paragraph::new(Line::from(line_spans)), area);
}

fn push_pill(
    out: &mut Vec<(String, Style, Option<SearchHit>)>,
    label: String,
    style: Style,
    hit: Option<SearchHit>,
) {
    out.push((label, style, hit));
    out.push((" ".to_string(), Style::default(), None));
}

pub fn render_search_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = *state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::Search).unwrap_or(frame.area());
    frame.render_widget(Clear, area);
    state.search_hitboxes.clear();
    let now_unix = unix_now_secs();

    let combined = state.combined_search_items();
    let stats_badge = match state.search_latency_ms {
        Some(ms) => format!("{}ms · {} hits", ms, combined.len()),
        None => format!("{} cached", combined.len()),
    };
    let mode_badge = if state.search_view_mode == SearchViewMode::Grid && area.width >= 102 {
        "⊞ Grid"
    } else {
        "☰ Split"
    };

    let outer = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " PHAENNA SEARCH STUDIO ",
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", state.search_scope.label()),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", mode_badge),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(56, 189, 248))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", stats_badge),
                Style::default()
                    .fg(theme.accent)
                    .bg(Color::Rgb(24, 28, 40))
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.width < 24 || inner.height < 14 {
        return;
    }

    let padded = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 0: Query bar
            Constraint::Length(1), // 1: Did-you-mean / Suggestion / Recent chips
            Constraint::Length(2), // 2: Clean 2-row filter ribbon
            Constraint::Min(8),    // 3: Spotlight + Results canvas
            Constraint::Length(1), // 4: Footer controls
        ])
        .split(inner);

    // 0. Query Command Bar + Inline Execute Button
    render_query_bar(frame, padded[0], state, &theme);

    // 1. Suggestion / Did-You-Mean / Recent Chips Strip
    render_chips_bar(frame, padded[1], state, &theme);

    // 2. Clean 2-Row Filter & Sort Ribbon (zero `][`, zero `:[`)
    render_filter_ribbon(frame, padded[2], state, &theme, combined.len());

    // 3. Main Discovery / Direct URL / Grid / Split Stage
    let direct_vid = extract_yt_url_id(&state.search_query);
    let show_channel_banner = direct_vid.is_none()
        && !state.search_channels.is_empty()
        && padded[3].height >= 16
        && state.search_scope != SearchScope::Cached;

    let mut stage_constraints = Vec::new();
    if direct_vid.is_some() {
        stage_constraints.push(Constraint::Length(3));
    }
    if show_channel_banner {
        stage_constraints.push(Constraint::Length(3));
    }
    stage_constraints.push(Constraint::Min(6));

    let stage_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(stage_constraints)
        .split(padded[3]);

    let mut si = 0usize;
    if let Some(ref vid) = direct_vid {
        render_direct_url_banner(frame, stage_chunks[si], state, vid, &theme);
        si += 1;
    }
    if show_channel_banner {
        render_channel_spotlight(frame, stage_chunks[si], state, &theme);
        si += 1;
    }
    let body_area = stage_chunks[si];

    if combined.is_empty() {
        render_empty_or_discovery(frame, body_area, state, &theme);
    } else {
        let use_grid = state.search_view_mode == SearchViewMode::Grid && area.width >= 102;
        if use_grid {
            render_search_grid(frame, body_area, state, &combined, now_unix, &theme);
        } else {
            render_search_split(frame, body_area, state, &combined, now_unix, &theme);
        }
    }

    // 4. Footer Keybindings
    let footer_hints = Line::from(vec![
        Span::styled(
            if state.search_focus == SearchFocus::Query {
                " [↓] Browse "
            } else {
                " [/] Query "
            },
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· [←→↑↓] Navigate ", Style::default().fg(theme.text)),
        Span::styled("· [Enter] Search/Detail ", Style::default().fg(Color::Rgb(56, 189, 248))),
        Span::styled("· [^v] Grid/Split ", Style::default().fg(theme.secondary)),
        Span::styled("· [R] Retry Art ", Style::default().fg(theme.accent)),
        Span::styled("· [Esc] Back", Style::default().fg(theme.text_dim)),
    ]);
    frame.render_widget(
        Paragraph::new(footer_hints).alignment(Alignment::Center),
        padded[4],
    );
}

fn render_query_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &ThemePalette,
) {
    let query_focused = state.search_focus == SearchFocus::Query;
    let border_col = if query_focused {
        theme.border_focus
    } else {
        theme.border_idle
    };
    let title_label = if query_focused {
        " Search YouTube & Library (Typing · [↓] Results) "
    } else {
        " Search YouTube & Library ([/] Edit Query) "
    };
    let blk = Block::default()
        .title(Span::styled(
            title_label,
            Style::default()
                .fg(if query_focused { theme.secondary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if query_focused {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(border_col));
    let q_inner = blk.inner(area);
    frame.render_widget(blk, area);
    if q_inner.width < 12 || q_inner.height == 0 {
        return;
    }

    let status_suffix = if state.is_searching_remote {
        "  (searching...)"
    } else if state.search_pending_remote {
        "  (typing...)"
    } else {
        ""
    };
    let right_btn_w = if q_inner.width >= 48 { 22u16 } else { 11u16 };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(right_btn_w),
        ])
        .split(q_inner);

    let avail_hint_w = cols[0]
        .width
        .saturating_sub(state.search_query.chars().count() as u16 + status_suffix.len() as u16 + 4);
    let tab_hint = if avail_hint_w >= 14 {
        state
            .search_did_you_mean
            .as_ref()
            .or_else(|| state.search_suggestions.first())
            .map(|s| {
                let clipped = truncate_word_boundary(s, avail_hint_w.saturating_sub(10) as usize);
                format!("   [Tab] -> \"{clipped}\"")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let input_line = Line::from(vec![
        Span::styled(" ▸ ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
        Span::styled(
            state.search_query.as_str(),
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if query_focused { "█" } else { "" },
            Style::default().fg(theme.primary),
        ),
        Span::styled(status_suffix, Style::default().fg(theme.accent)),
        Span::styled(tab_hint, Style::default().fg(theme.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(input_line), cols[0]);
    state.search_hitboxes.push((cols[0], SearchHit::FocusQuery));

    let mut right_pills = Vec::new();
    push_pill(
        &mut right_pills,
        "[↵ Search]".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD),
        Some(SearchHit::ExecuteQueryNow),
    );
    if right_btn_w >= 20 {
        push_pill(
            &mut right_pills,
            "[Esc]".to_string(),
            Style::default().fg(theme.text_dim).bg(Color::Rgb(24, 28, 40)),
            Some(SearchHit::CloseSearch),
        );
    }
    render_pill_strip(frame, cols[1], right_pills, &mut state.search_hitboxes);
}

fn render_chips_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &ThemePalette,
) {
    let mut chips: Vec<(String, Style, Option<SearchHit>)> = Vec::new();
    if let Some(ref corr) = state.search_did_you_mean {
        chips.push((
            " Did you mean: ".to_string(),
            Style::default().fg(Color::Rgb(251, 191, 36)).add_modifier(Modifier::BOLD),
            None,
        ));
        push_pill(
            &mut chips,
            format!("[{corr}]"),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(251, 191, 36))
                .add_modifier(Modifier::BOLD),
            Some(SearchHit::ApplySuggestion(corr.clone())),
        );
    }

    let chip_source = if state.search_query.trim().is_empty() && !state.search_history.is_empty() {
        chips.push((" Recent: ".to_string(), Style::default().fg(theme.text_dim), None));
        &state.search_history
    } else {
        chips.push((" Suggest: ".to_string(), Style::default().fg(theme.text_dim), None));
        &state.search_suggestions
    };

    if chip_source.is_empty() && state.search_did_you_mean.is_none() {
        chips.push((
            "Try: \"exact phrase\" · @channel · -short · is:video · is:live · is:4k · URL".to_string(),
            Style::default().fg(theme.text_dim),
            None,
        ));
    } else {
        for sug in chip_source.iter().take(5) {
            push_pill(
                &mut chips,
                format!("[{sug}]"),
                Style::default()
                    .fg(Color::Rgb(56, 189, 248))
                    .bg(Color::Rgb(24, 28, 40)),
                Some(SearchHit::ApplySuggestion(sug.clone())),
            );
        }
    }
    render_pill_strip(frame, area, chips, &mut state.search_hitboxes);
}

fn render_filter_ribbon(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &ThemePalette,
    result_count: usize,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let active_pill = Style::default()
        .fg(Color::Black)
        .bg(theme.primary)
        .add_modifier(Modifier::BOLD);
    let accent_pill = Style::default()
        .fg(Color::Black)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let green_pill = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(52, 211, 153))
        .add_modifier(Modifier::BOLD);
    let idle_pill = Style::default().fg(theme.text_dim);

    // Ribbon Row 0: Type + Date + Duration
    let mut r0: Vec<(String, Style, Option<SearchHit>)> =
        vec![(" Type: ".to_string(), Style::default().fg(theme.secondary), None)];
    for tf in SearchTypeFilter::ALL {
        push_pill(
            &mut r0,
            format!("[{}]", tf.label()),
            if tf == state.search_type_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetType(tf)),
        );
    }
    r0.push((" | Date: ".to_string(), Style::default().fg(theme.secondary), None));
    for df in SearchDateFilter::ALL {
        push_pill(
            &mut r0,
            format!("[{}]", df.label()),
            if df == state.search_date_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetDate(df)),
        );
    }
    r0.push((" | Dur: ".to_string(), Style::default().fg(theme.secondary), None));
    for du in SearchDurationFilter::ALL {
        push_pill(
            &mut r0,
            format!("[{}]", du.label()),
            if du == state.search_duration_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetDuration(du)),
        );
    }
    render_pill_strip(frame, rows[0], r0, &mut state.search_hitboxes);

    // Ribbon Row 1: Sort + Scope + Quality/Watched + View Mode + Load More
    let mut r1: Vec<(String, Style, Option<SearchHit>)> =
        vec![(" Sort: ".to_string(), Style::default().fg(theme.secondary), None)];
    for so in SearchSortOrder::ALL {
        push_pill(
            &mut r1,
            format!("[{}]", so.label()),
            if so == state.search_sort_order { accent_pill } else { idle_pill },
            Some(SearchHit::SetSort(so)),
        );
    }
    r1.push((" | Scope: ".to_string(), Style::default().fg(theme.secondary), None));
    for sc in SearchScope::ALL {
        push_pill(
            &mut r1,
            format!("[{}]", sc.label()),
            if sc == state.search_scope { active_pill } else { idle_pill },
            Some(SearchHit::SetScope(sc)),
        );
    }
    push_pill(
        &mut r1,
        if state.search_hd_4k_only { "[4K/HD:ON]" } else { "[4K/HD]" }.to_string(),
        if state.search_hd_4k_only { green_pill } else { idle_pill },
        Some(SearchHit::ToggleHd4k),
    );
    push_pill(
        &mut r1,
        if state.search_hide_watched { "[Unwatched:ON]" } else { "[Unwatched]" }.to_string(),
        if state.search_hide_watched { green_pill } else { idle_pill },
        Some(SearchHit::ToggleHideWatched),
    );
    push_pill(
        &mut r1,
        "[⊞ Grid]".to_string(),
        if state.search_view_mode == SearchViewMode::Grid {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(56, 189, 248))
                .add_modifier(Modifier::BOLD)
        } else {
            idle_pill
        },
        Some(SearchHit::SetViewMode(SearchViewMode::Grid)),
    );
    push_pill(
        &mut r1,
        "[☰ Split]".to_string(),
        if state.search_view_mode == SearchViewMode::Split {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(56, 189, 248))
                .add_modifier(Modifier::BOLD)
        } else {
            idle_pill
        },
        Some(SearchHit::SetViewMode(SearchViewMode::Split)),
    );
    if state.search_continuation.is_some() {
        push_pill(
            &mut r1,
            format!("[+More ({})]", result_count),
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
            Some(SearchHit::LoadMore),
        );
    }
    render_pill_strip(frame, rows[1], r1, &mut state.search_hitboxes);
}

fn render_direct_url_banner(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    vid: &str,
    theme: &ThemePalette,
) {
    let blk = Block::default()
        .title(Span::styled(
            " DIRECT YOUTUBE URL DETECTED ",
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(52, 211, 153)));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 12 || inner.height == 0 {
        return;
    }

    let mut pills = vec![
        (
            format!(" Video ID: {vid}  "),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            None,
        ),
    ];
    push_pill(
        &mut pills,
        "[Open Detail (Enter)]".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD),
        Some(SearchHit::DirectJumpDetail(vid.to_string())),
    );
    push_pill(
        &mut pills,
        "[Stream Now]".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(52, 211, 153))
            .add_modifier(Modifier::BOLD),
        Some(SearchHit::DirectJumpPlay(vid.to_string())),
    );
    render_pill_strip(frame, inner, pills, &mut state.search_hitboxes);
}

fn render_channel_spotlight(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &ThemePalette,
) {
    let Some(ch) = state.search_channels.first().cloned() else {
        return;
    };
    let blk = Block::default()
        .title(Span::styled(
            " CHANNEL SPOTLIGHT ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.divider));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 16 || inner.height == 0 {
        return;
    }

    let verified_mark = if ch.verified { " ✓" } else { "" };
    let handle_part = ch
        .handle
        .as_deref()
        .map(|h| format!(" · {h}"))
        .unwrap_or_default();
    let sub_part = ch
        .subscriber_text
        .as_deref()
        .map(|s| format!(" · {s}"))
        .unwrap_or_default();
    let max_meta = inner.width.saturating_sub(36) as usize;
    let summary = truncate_word_boundary(
        &format!("◈ {}{}{}{}", ch.title, verified_mark, handle_part, sub_part),
        max_meta.max(12),
    );
    let filter_key = ch.handle.clone().unwrap_or_else(|| ch.title.clone());

    let mut pills = vec![
        (
            format!(" {summary}  "),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            None,
        ),
    ];
    push_pill(
        &mut pills,
        "[Open Channel]".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(theme.secondary)
            .add_modifier(Modifier::BOLD),
        Some(SearchHit::OpenSpotlightChannel(0)),
    );
    push_pill(
        &mut pills,
        "[Filter @]".to_string(),
        Style::default()
            .fg(theme.accent)
            .bg(Color::Rgb(24, 28, 40)),
        Some(SearchHit::FilterByChannel(filter_key)),
    );
    render_pill_strip(frame, inner, pills, &mut state.search_hitboxes);
}

fn render_empty_or_discovery(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &ThemePalette,
) {
    state.search_results_rect = Rect::default();
    let msg = if state.search_query.trim().is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "Type keywords above or use power search operators:",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  @channel / ch:name  ·  \"exact phrase\"  ·  -exclude  ·  is:video  ·  is:short  ·  is:live  ·  is:4k  ·  is:unwatched",
                Style::default().fg(theme.secondary),
            )),
            Line::from(Span::styled(
                "  Or paste any youtube.com / youtu.be link for an instant Direct Jump Card.",
                Style::default().fg(theme.text_dim),
            )),
        ]
    } else if state.is_searching_remote {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "Fetching structured YouTube InnerTube results & HD thumbnails...",
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            )),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "No matching videos under the current query/filters. Press [Enter] to query live or relax Date/Type filters.",
                Style::default().fg(theme.text_dim),
            )),
        ]
    };
    frame.render_widget(Paragraph::new(msg).alignment(Alignment::Center), area);
}

fn build_status_badges(state: &AppState, v: &VideoItem, now_unix: i64) -> Vec<(&'static str, Color)> {
    let mut tags: Vec<(&'static str, Color)> = Vec::new();
    if v.is_live_stream() {
        tags.push(("LIVE", Color::Rgb(248, 113, 113)));
    } else if state.is_in_continue_watching(&v.id) {
        tags.push(("CW", Color::Rgb(56, 189, 248)));
    } else if state.is_video_watched(&v.id) {
        tags.push(("OK", Color::Rgb(167, 139, 250)));
    } else if v.is_new_release(now_unix) {
        tags.push(("NEW", Color::Rgb(52, 211, 153)));
    }
    if state.watch_later_sources.contains_key(&v.id) {
        tags.push(("WL", Color::Rgb(251, 191, 36)));
    }
    tags
}

fn render_search_grid(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    combined: &[VideoItem],
    now_unix: i64,
    theme: &ThemePalette,
) {
    let dock_h: u16 = if area.height >= 15 { 4 } else { 3 };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(dock_h)])
        .split(area);

    let grid_outer = Block::default()
        .title(Span::styled(
            format!(" VISUAL RESULTS ({}) ", combined.len()),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.search_focus == SearchFocus::Results {
            theme.border_focus
        } else {
            theme.border_idle
        }));
    let grid_inner = grid_outer.inner(parts[0]);
    frame.render_widget(grid_outer, parts[0]);
    state.search_results_rect = grid_inner;

    let cols: usize = if grid_inner.width >= 140 {
        4
    } else if grid_inner.width >= 96 {
        3
    } else {
        2
    };
    state.search_grid_cols = cols;

    let vis_rows: usize = if grid_inner.height >= 16 { 2 } else { 1 };
    let cur = state.search_cursor.min(combined.len().saturating_sub(1));
    let cur_row = cur / cols;
    let scroll_row = if cur_row >= vis_rows {
        cur_row + 1 - vis_rows
    } else {
        0
    };
    let start_idx = scroll_row * cols;
    let end_idx = (start_idx + vis_rows * cols).min(combined.len());

    // Strict priority thumbnail queuing: Focused -> Visible -> Prefetch next row
    if let Some(focused_v) = combined.get(cur) {
        state.queue_thumb_with_priority(
            &focused_v.id,
            &focused_v.best_thumbnail_url(),
            ThumbPriority::Focused,
        );
    }
    for (i, v) in combined.iter().enumerate().skip(start_idx).take(vis_rows * cols) {
        if i != cur {
            state.queue_thumb_with_priority(
                &v.id,
                &v.best_thumbnail_url(),
                ThumbPriority::Visible,
            );
        }
    }
    for v in combined.iter().skip(end_idx).take(cols) {
        state.queue_thumb_with_priority(&v.id, &v.best_thumbnail_url(), ThumbPriority::Prefetch);
    }

    let row_constraints = vec![Constraint::Ratio(1, vis_rows as u32); vis_rows];
    let grid_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(grid_inner);

    for r in 0..vis_rows {
        let col_constraints = vec![Constraint::Ratio(1, cols as u32); cols];
        let row_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(grid_rows[r]);

        for c in 0..cols {
            let idx = start_idx + r * cols + c;
            if idx >= combined.len() {
                break;
            }
            let v = &combined[idx];
            let card_rect = row_cols[c];
            let is_sel = idx == cur;
            let card_pal = if is_sel {
                state.active_palette_for(Some(&v.id))
            } else {
                *theme
            };
            let card_blk = Block::default()
                .borders(Borders::ALL)
                .border_type(if is_sel {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
                .border_style(Style::default().fg(if is_sel {
                    card_pal.border_focus
                } else {
                    card_pal.border_idle
                }));
            let card_inner = card_blk.inner(card_rect);
            frame.render_widget(card_blk, card_rect);
            state.search_hitboxes.push((card_rect, SearchHit::SelectRow(idx)));
            if card_inner.width < 10 || card_inner.height < 5 {
                continue;
            }

            let meta_h: u16 = if card_inner.height >= 9 { 4 } else { 3 };
            let thumb_h = card_inner.height.saturating_sub(meta_h).max(2);
            let card_parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(thumb_h), Constraint::Length(meta_h)])
                .split(card_inner);

            render_thumbnail_cover(frame, card_parts[0], &v.id, state, "Loading 16:9...");

            let text_w = card_parts[1].width as usize;
            let wrapped = wrap_words(&v.title, text_w.max(8));
            let title_line0 = wrapped.first().cloned().unwrap_or_default();
            let title_line1 = if wrapped.len() > 2 {
                truncate_word_boundary(&wrapped[1..].join(" "), text_w.max(8))
            } else {
                wrapped.get(1).cloned().unwrap_or_default()
            };

            let ver_mark = if v.channel_verified { " ✓" } else { "" };
            let dur_s = v.display_duration();
            let ch_budget = text_w.saturating_sub(dur_s.chars().count() + 4).max(6);
            let ch_str = truncate_word_boundary(
                &format!("{}{}", v.display_uploader(), ver_mark),
                ch_budget,
            );
            let pad_ch = text_w
                .saturating_sub(ch_str.chars().count() + dur_s.chars().count())
                .max(1);

            let mut meta_lines = vec![
                Line::from(Span::styled(
                    title_line0,
                    Style::default()
                        .fg(if is_sel { card_pal.primary } else { card_pal.text })
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    title_line1,
                    Style::default().fg(if is_sel { card_pal.primary } else { card_pal.text }),
                )),
                Line::from(vec![
                    Span::styled(ch_str, Style::default().fg(card_pal.secondary)),
                    Span::raw(" ".repeat(pad_ch)),
                    Span::styled(
                        dur_s,
                        Style::default()
                            .fg(if v.is_live_stream() {
                                Color::Rgb(248, 113, 113)
                            } else {
                                card_pal.accent
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
            ];

            if meta_h >= 4 {
                let views_s = v.compact_views(true).unwrap_or_else(|| "—".to_string());
                let age_s = v.compact_age(now_unix, true).unwrap_or_else(|| "—".to_string());
                let mut line3 = vec![Span::styled(
                    truncate_word_boundary(&format!("{views_s} · {age_s}"), text_w.saturating_sub(10).max(8)),
                    Style::default().fg(card_pal.text_dim),
                )];
                for (lbl, col) in build_status_badges(state, v, now_unix).into_iter().take(2) {
                    line3.push(Span::raw(" "));
                    line3.push(Span::styled(
                        format!("[{lbl}]"),
                        Style::default().fg(col).add_modifier(Modifier::BOLD),
                    ));
                }
                meta_lines.push(Line::from(line3));
            }

            frame.render_widget(Paragraph::new(meta_lines), card_parts[1]);
        }
    }

    // Bottom Focused Spotlight Dock (full title + match snippet + quick actions)
    if let Some(sel) = combined.get(cur) {
        let pal = state.active_palette_for(Some(&sel.id));
        let dock_blk = Block::default()
            .title(Span::styled(
                format!(" FOCUSED SPOTLIGHT (#{}) ", cur + 1),
                Style::default()
                    .fg(pal.secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal.border_focus));
        let dock_inner = dock_blk.inner(parts[1]);
        frame.render_widget(dock_blk, parts[1]);
        if dock_inner.width >= 20 && dock_inner.height >= 1 {
            let max_w = dock_inner.width.saturating_sub(2) as usize;
            let ver = if sel.channel_verified { " ✓" } else { "" };
            let views = sel.compact_views(true).unwrap_or_else(|| "—".to_string());
            let age = sel.compact_age(now_unix, true).unwrap_or_else(|| "—".to_string());
            let headline = truncate_word_boundary(
                &format!(
                    "▸ {} — {}{} · {} · {} · {}",
                    sel.title,
                    sel.display_uploader(),
                    ver,
                    views,
                    age,
                    sel.display_duration()
                ),
                max_w,
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {headline}"),
                    Style::default().fg(pal.text).add_modifier(Modifier::BOLD),
                ))),
                Rect::new(dock_inner.x, dock_inner.y, dock_inner.width, 1),
            );
            if dock_inner.height >= 2 {
                let mut action_pills = Vec::new();
                push_pill(
                    &mut action_pills,
                    "[Detail (Enter)]".to_string(),
                    Style::default().fg(Color::Black).bg(pal.primary).add_modifier(Modifier::BOLD),
                    Some(SearchHit::ActionDetail),
                );
                push_pill(
                    &mut action_pills,
                    "[Play (^p)]".to_string(),
                    Style::default().fg(Color::Black).bg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD),
                    Some(SearchHit::ActionPlay),
                );
                push_pill(
                    &mut action_pills,
                    "[+Queue (^a)]".to_string(),
                    Style::default().fg(pal.secondary).bg(Color::Rgb(24, 28, 40)),
                    Some(SearchHit::ActionPinCw),
                );
                push_pill(
                    &mut action_pills,
                    "[+WL (^l)]".to_string(),
                    Style::default().fg(Color::Rgb(251, 191, 36)).bg(Color::Rgb(24, 28, 40)),
                    Some(SearchHit::ActionWatchLater),
                );
                push_pill(
                    &mut action_pills,
                    "[Channel (^o)]".to_string(),
                    Style::default().fg(pal.text).bg(Color::Rgb(24, 28, 40)),
                    Some(SearchHit::ActionChannel),
                );
                push_pill(
                    &mut action_pills,
                    "[Retry Art (R)]".to_string(),
                    Style::default().fg(pal.accent).bg(Color::Rgb(24, 28, 40)),
                    Some(SearchHit::RetryThumbs),
                );
                if let Some(ref snip) = sel.search_snippet {
                    let snip_text = truncate_word_boundary(snip, max_w / 2);
                    action_pills.push((
                        format!(" · \"{snip_text}\""),
                        Style::default().fg(pal.text_dim),
                        None,
                    ));
                }
                render_pill_strip(
                    frame,
                    Rect::new(dock_inner.x, dock_inner.y + 1, dock_inner.width, 1),
                    action_pills,
                    &mut state.search_hitboxes,
                );
            }
        }
    }
}

fn render_search_split(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    combined: &[VideoItem],
    now_unix: i64,
    theme: &ThemePalette,
) {
    state.search_grid_cols = 1;
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(area);

    let list_blk = Block::default()
        .title(Span::styled(
            format!(" RESULTS ({}) ", combined.len()),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.search_focus == SearchFocus::Results {
            theme.border_focus
        } else {
            theme.border_idle
        }));
    let list_inner = list_blk.inner(panes[0]);
    frame.render_widget(list_blk, panes[0]);
    state.search_results_rect = list_inner;

    let cur = state.search_cursor.min(combined.len().saturating_sub(1));
    let row_h: usize = if list_inner.height >= 12 { 3 } else { 2 };
    let rows_cap = (list_inner.height as usize / row_h).max(1);
    let scroll_top = if cur >= rows_cap { cur - rows_cap + 1 } else { 0 };

    if let Some(focused_v) = combined.get(cur) {
        state.queue_thumb_with_priority(
            &focused_v.id,
            &focused_v.best_thumbnail_url(),
            ThumbPriority::Focused,
        );
    }
    for (i, v) in combined.iter().enumerate().skip(scroll_top).take(rows_cap) {
        if i != cur {
            state.queue_thumb_with_priority(
                &v.id,
                &v.best_thumbnail_url(),
                ThumbPriority::Visible,
            );
        }
    }
    for v in combined.iter().skip(scroll_top + rows_cap).take(4) {
        state.queue_thumb_with_priority(&v.id, &v.best_thumbnail_url(), ThumbPriority::Prefetch);
    }

    let show_inline_thumb = row_h >= 3 && list_inner.width >= 38;
    for (slot, (idx, v)) in combined
        .iter()
        .enumerate()
        .skip(scroll_top)
        .take(rows_cap)
        .enumerate()
    {
        let y = list_inner.y + (slot * row_h) as u16;
        let row_rect = Rect::new(list_inner.x, y, list_inner.width, row_h as u16);
        let is_sel = idx == cur;
        let row_bg = if is_sel {
            Color::Rgb(30, 36, 52)
        } else {
            Color::Reset
        };

        let text_area = if show_inline_thumb {
            let row_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(10), Constraint::Min(16)])
                .split(row_rect);
            render_thumbnail_cover(frame, row_split[0], &v.id, state, "...");
            row_split[1]
        } else {
            row_rect
        };

        let tw = text_area.width.saturating_sub(10).max(10) as usize;
        let title_str = truncate_word_boundary(&v.title, tw);
        let badges = build_status_badges(state, v, now_unix);
        let badge_tag = badges
            .first()
            .map(|(b, _)| format!("[{b}]"))
            .unwrap_or_default();
        let badge_col = badges.first().map(|(_, c)| *c).unwrap_or(theme.text_dim);

        let line1 = Line::from(vec![
            Span::styled(
                if is_sel { " ▸ " } else { "   " },
                Style::default()
                    .fg(theme.primary)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{badge_tag} "),
                Style::default().fg(badge_col).bg(row_bg),
            ),
            Span::styled(
                title_str,
                Style::default()
                    .fg(if is_sel { theme.primary } else { theme.text })
                    .bg(row_bg)
                    .add_modifier(if is_sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]);

        let ver = if v.channel_verified { " ✓" } else { "" };
        let views_s = v.compact_views(false).unwrap_or_else(|| "—".to_string());
        let date_s = v.compact_age(now_unix, false).unwrap_or_else(|| "—".to_string());
        let dur_s = v.display_duration();
        let meta_line = truncate_word_boundary(
            &format!("{}{ver} · {views_s} · {date_s} · {dur_s}", v.display_uploader()),
            text_area.width.saturating_sub(4) as usize,
        );
        let mut lines = vec![
            line1,
            Line::from(Span::styled(
                format!("   {meta_line}"),
                Style::default().fg(theme.text_dim).bg(row_bg),
            )),
        ];
        if row_h >= 3 {
            let snip = v
                .search_snippet
                .as_deref()
                .map(|s| truncate_word_boundary(s, text_area.width.saturating_sub(5) as usize))
                .unwrap_or_default();
            lines.push(Line::from(Span::styled(
                if snip.is_empty() {
                    String::new()
                } else {
                    format!("   \"{snip}\"")
                },
                Style::default().fg(theme.secondary).bg(row_bg),
            )));
        }

        frame.render_widget(Paragraph::new(lines).style(Style::default().bg(row_bg)), text_area);
        state.search_hitboxes.push((row_rect, SearchHit::SelectRow(idx)));
    }

    // Right Pane: Focused 16:9 Preview Inspector + Quick Action Strip
    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(2)])
        .split(panes[1]);

    if let Some(sel) = combined.get(cur) {
        let badges = build_status_badges(state, sel, now_unix);
        render_video_preview_inspector(
            frame,
            right_split[0],
            state,
            sel,
            now_unix,
            &badges,
            sel.yt_rank.map(|r| format!("YT #{r}")).as_deref(),
            "",
        );
    }

    let mut actions = Vec::new();
    push_pill(
        &mut actions,
        "[Detail]".to_string(),
        Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD),
        Some(SearchHit::ActionDetail),
    );
    push_pill(
        &mut actions,
        "[Play(^p)]".to_string(),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(52, 211, 153))
            .add_modifier(Modifier::BOLD),
        Some(SearchHit::ActionPlay),
    );
    push_pill(
        &mut actions,
        "[+Queue]".to_string(),
        Style::default().fg(theme.secondary).bg(Color::Rgb(24, 28, 40)),
        Some(SearchHit::ActionPinCw),
    );
    push_pill(
        &mut actions,
        "[+WL]".to_string(),
        Style::default().fg(Color::Rgb(251, 191, 36)).bg(Color::Rgb(24, 28, 40)),
        Some(SearchHit::ActionWatchLater),
    );
    push_pill(
        &mut actions,
        "[Channel]".to_string(),
        Style::default().fg(theme.text).bg(Color::Rgb(24, 28, 40)),
        Some(SearchHit::ActionChannel),
    );
    push_pill(
        &mut actions,
        "[Retry(R)]".to_string(),
        Style::default().fg(theme.accent).bg(Color::Rgb(24, 28, 40)),
        Some(SearchHit::RetryThumbs),
    );
    render_pill_strip(frame, right_split[1], actions, &mut state.search_hitboxes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::innertube::SearchChannelCard;
    use crate::config::Config;
    use crate::db::cache::VideoItem;
    use crate::state::FailedThumbEntry;
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, Instant};

    fn buffer_text(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn test_search_studio_filters_ranking_and_render() {
        let mut st = AppState::new(Config::default());
        st.overlay = ActiveOverlay::Search;
        st.search_query = "system architecture".to_string();
        st.search_suggestions = vec![
            "system architecture interview".to_string(),
            "system architecture patterns".to_string(),
        ];
        st.search_channels = vec![SearchChannelCard {
            channel_id: "UCgkcs".into(),
            title: "Gaurav Sen".into(),
            handle: Some("@gkcs".into()),
            subscriber_text: Some("612K subscribers".into()),
            verified: true,
            ..Default::default()
        }];
        st.search_results = vec![
            VideoItem {
                id: "vid_short_1".into(),
                title: "Quick Architecture Trivia".into(),
                uploader: Some("ShortClips".into()),
                duration: Some(45),
                duration_string: Some("SHORT".into()),
                view_count: Some(500_000),
                yt_rank: Some(2),
                ..Default::default()
            },
            VideoItem {
                id: "vid_long_01".into(),
                title: "Software Architecture Patterns in Production Distributed Systems".into(),
                uploader: Some("ByteByteGo".into()),
                duration: Some(1224),
                view_count: Some(1_400_000),
                channel_verified: true,
                search_snippet: Some("Compare layered, event-driven, and hexagonal architecture.".into()),
                yt_rank: Some(0),
                ..Default::default()
            },
        ];
        // Exhausted thumb retries must show NO COVER / retry hint, never infinite "Loading 16:9..."
        st.failed_thumbs.insert(
            "vid_long_01".into(),
            FailedThumbEntry {
                attempts: 3,
                next_retry_at: Instant::now() + Duration::from_secs(30),
            },
        );
        // Seed vid_short_1 with an already-loaded image so only the failed item needs fallback art
        let img_ok = image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 36));
        st.raw_thumbs.insert("vid_short_1".into(), img_ok);

        assert_eq!(st.combined_search_items().len(), 2);
        st.search_type_filter = SearchTypeFilter::Videos;
        let only_vids = st.combined_search_items();
        assert_eq!(only_vids.len(), 1);
        assert_eq!(only_vids[0].id, "vid_long_01");

        st.search_type_filter = SearchTypeFilter::All;
        for mode in [SearchViewMode::Grid, SearchViewMode::Split] {
            st.search_view_mode = mode;
            for (w, h) in [(100, 28), (136, 36), (156, 40)] {
                let backend = TestBackend::new(w, h);
                let mut term = Terminal::new(backend).unwrap();
                term.draw(|f| render_search_modal(f, &mut st)).unwrap();
                let txt = buffer_text(&term);
                for line in txt.lines() {
                    assert!(
                        !line.contains("]["),
                        "Found concatenated '][' on {mode:?} {w}x{h}: {line:?}"
                    );
                    assert!(
                        !line.contains(":["),
                        "Found concatenated ':[' on {mode:?} {w}x{h}: {line:?}"
                    );
                }
                assert!(
                    !txt.contains("Distr..."),
                    "Broken mid-word chop on {mode:?} {w}x{h}:\n{txt}"
                );
                assert!(
                    !txt.contains("Loading 16:9..."),
                    "Stuck Loading 16:9 despite exhausted retries on {mode:?} {w}x{h}"
                );
                assert!(
                    txt.contains("NO COVER") || txt.contains("[R]"),
                    "Expected NO COVER retry fallback on {mode:?} {w}x{h}:\n{txt}"
                );
                assert!(txt.contains("CHANNEL SPOTLIGHT"));
                assert!(!st.search_hitboxes.is_empty());
            }
        }
    }
}
