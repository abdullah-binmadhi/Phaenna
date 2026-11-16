//! Smart Search Studio (`/`):
//! Instant Local SQLite + Fast YouTube InnerTube Search + Autocomplete Suggestion Chips,
//! Multi-Axis Filters (Scope, Format/Shorts, Date, Duration, Hide Watched), 4 Sort Orders,
//! 2-Line Telemetry Results, and Live 16:9 Preview Inspector with Quick Actions.

use crate::state::{
    active_modal_rect, centered_rect, ActiveOverlay, AppState, SearchDateFilter,
    SearchDurationFilter, SearchHit, SearchScope, SearchSortOrder, SearchTypeFilter,
};
use crate::ui::components::cover::{render_video_preview_inspector, unix_now_secs};
use crate::ui::home::marquee_window;
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
    let mut line_spans = Vec::with_capacity(spans_and_hits.len());
    for (text, style, hit) in spans_and_hits {
        let len = text.chars().count() as u16;
        if cur_x + len > area.x + area.width {
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

pub fn render_search_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::Search)
        .unwrap_or_else(|| centered_rect(90, 86, frame.area()));
    frame.render_widget(Clear, area);
    state.search_hitboxes.clear();
    let now_unix = unix_now_secs();

    let outer = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " PHAENNA SEARCH STUDIO ",
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" [{}] ", state.search_scope.label()),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Query box
            Constraint::Length(1), // Suggestion / history chips
            Constraint::Length(3), // Filter & Sort pills
            Constraint::Min(10),   // Results + Live Preview
            Constraint::Length(1), // Footer keybindings
        ])
        .split(inner);

    // 1. Query box + inline Tab autocomplete hint
    let input_blk = Block::default()
        .title(Span::styled(" Query ", Style::default().fg(theme.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus));

    let status_suffix = if state.is_searching_remote {
        "  (searching YouTube...)"
    } else if state.search_pending_remote {
        "  (waiting debounce...)"
    } else {
        ""
    };
    let tab_hint = state
        .search_suggestions
        .first()
        .map(|s| format!("   [Tab] -> \"{s}\""))
        .unwrap_or_default();

    let input_line = Line::from(vec![
        Span::styled(
            &state.search_query,
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(theme.primary)),
        Span::styled(status_suffix, Style::default().fg(theme.accent)),
        Span::styled(tab_hint, Style::default().fg(theme.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(input_line).block(input_blk), chunks[0]);

    // 2. Autocomplete Suggestion / Recent History Chips
    {
        let mut chips: Vec<(String, Style, Option<SearchHit>)> = Vec::new();
        let chip_source = if state.search_query.trim().is_empty() && !state.search_history.is_empty() {
            chips.push((" Recent: ".to_string(), Style::default().fg(theme.text_dim), None));
            &state.search_history
        } else {
            chips.push((" Suggest: ".to_string(), Style::default().fg(theme.text_dim), None));
            &state.search_suggestions
        };
        if chip_source.is_empty() {
            chips.push((
                "Type keywords, @channel, ch:name, or paste a YouTube URL".to_string(),
                Style::default().fg(theme.text_dim),
                None,
            ));
        } else {
            for sug in chip_source.iter().take(5) {
                chips.push((
                    format!("[{sug}]"),
                    Style::default()
                        .fg(Color::Rgb(56, 189, 248))
                        .bg(Color::Rgb(24, 28, 40)),
                    Some(SearchHit::ApplySuggestion(sug.clone())),
                ));
                chips.push((" ".to_string(), Style::default(), None));
            }
        }
        render_pill_strip(frame, chunks[1], chips, &mut state.search_hitboxes);
    }

    // 3. Multi-Axis Filter & Sort Pill Bar
    let filter_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(chunks[2]);

    let active_pill = Style::default()
        .fg(Color::Black)
        .bg(theme.primary)
        .add_modifier(Modifier::BOLD);
    let idle_pill = Style::default().fg(theme.text_dim);

    // Filter Row 0: Scope [^g] + Type [^t] + Hide Watched [^w]
    let mut row0: Vec<(String, Style, Option<SearchHit>)> =
        vec![(" Scope[^g]: ".to_string(), Style::default().fg(theme.secondary), None)];
    for sc in SearchScope::ALL {
        row0.push((
            format!("[{}]", sc.label()),
            if sc == state.search_scope { active_pill } else { idle_pill },
            Some(SearchHit::SetScope(sc)),
        ));
        row0.push((" ".to_string(), Style::default(), None));
    }
    row0.push((" | Type[^t]: ".to_string(), Style::default().fg(theme.secondary), None));
    for tf in SearchTypeFilter::ALL {
        row0.push((
            format!("[{}]", tf.label()),
            if tf == state.search_type_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetType(tf)),
        ));
        row0.push((" ".to_string(), Style::default(), None));
    }
    row0.push((
        if state.search_hide_watched { "[HideWatched:ON]" } else { "[HideWatched:OFF]" }.to_string(),
        if state.search_hide_watched {
            Style::default().fg(Color::Black).bg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD)
        } else {
            idle_pill
        },
        Some(SearchHit::ToggleHideWatched),
    ));
    render_pill_strip(frame, filter_rows[0], row0, &mut state.search_hitboxes);

    // Filter Row 1: Date [^d] + Duration [^u]
    let mut row1: Vec<(String, Style, Option<SearchHit>)> =
        vec![(" Date[^d] : ".to_string(), Style::default().fg(theme.secondary), None)];
    for df in SearchDateFilter::ALL {
        row1.push((
            format!("[{}]", df.label()),
            if df == state.search_date_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetDate(df)),
        ));
        row1.push((" ".to_string(), Style::default(), None));
    }
    row1.push((" | Dur[^u]: ".to_string(), Style::default().fg(theme.secondary), None));
    for du in SearchDurationFilter::ALL {
        row1.push((
            format!("[{}]", du.label()),
            if du == state.search_duration_filter { active_pill } else { idle_pill },
            Some(SearchHit::SetDuration(du)),
        ));
        row1.push((" ".to_string(), Style::default(), None));
    }
    render_pill_strip(frame, filter_rows[1], row1, &mut state.search_hitboxes);

    // Filter Row 2: Sort [^s] + Result Counter + Load More
    let combined = state.combined_search_items();
    let mut row2: Vec<(String, Style, Option<SearchHit>)> =
        vec![(" Sort[^s] : ".to_string(), Style::default().fg(theme.secondary), None)];
    for so in SearchSortOrder::ALL {
        row2.push((
            format!("[{}]", so.label()),
            if so == state.search_sort_order {
                Style::default().fg(Color::Black).bg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                idle_pill
            },
            Some(SearchHit::SetSort(so)),
        ));
        row2.push((" ".to_string(), Style::default(), None));
    }
    row2.push((
        format!(" | {} results ", combined.len()),
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        None,
    ));
    if state.search_continuation.is_some() {
        row2.push((
            "[+Load More (^n)]".to_string(),
            Style::default().fg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD),
            Some(SearchHit::LoadMore),
        ));
    }
    render_pill_strip(frame, filter_rows[2], row2, &mut state.search_hitboxes);

    // 4. Split Panes: 56% Telemetry Results / 44% Live Preview + Quick Actions
    if combined.is_empty() {
        state.search_results_rect = Rect::default();
        let msg = if state.search_query.trim().is_empty() {
            "Start typing to search local SQLite cache + fast YouTube InnerTube with live filters."
        } else if state.is_searching_remote {
            "Fetching matching YouTube videos..."
        } else {
            "No matching videos under current filters. Relax Date/Type filters or press Enter to query."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            chunks[3],
        );
    } else {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(chunks[3]);

        let list_blk = Block::default()
            .title(Span::styled(
                format!(" RESULTS ({}) ", combined.len()),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        let list_inner = list_blk.inner(panes[0]);
        frame.render_widget(list_blk, panes[0]);
        state.search_results_rect = list_inner;

        let cur = state.search_cursor.min(combined.len().saturating_sub(1));
        for v in combined.iter().skip(cur.saturating_sub(4)).take(12) {
            state.queue_thumb_if_needed(&v.id, &v.best_thumbnail_url());
        }

        // Render 2-line rich telemetry rows (Title line + Channel/Views/Date/Duration line)
        let rows_cap = (list_inner.height as usize / 2).max(1);
        let scroll_top = if cur >= rows_cap { cur - rows_cap + 1 } else { 0 };
        let title_w = list_inner.width.saturating_sub(11).max(12) as usize;

        for (slot, (idx, v)) in combined
            .iter()
            .enumerate()
            .skip(scroll_top)
            .take(rows_cap)
            .enumerate()
        {
            let y = list_inner.y + (slot as u16) * 2;
            let row_rect = Rect::new(list_inner.x, y, list_inner.width, 2);
            let is_sel = idx == cur;
            let row_bg = if is_sel { Color::Rgb(32, 38, 52) } else { Color::Reset };

            let (badge, badge_col) = if state.is_in_continue_watching(&v.id) {
                ("[CW] ", Color::Rgb(56, 189, 248))
            } else if state.is_video_watched(&v.id) {
                ("[OK] ", theme.accent)
            } else if v.is_new_release(now_unix) {
                ("NEW  ", Color::Rgb(52, 211, 153))
            } else {
                ("  ·  ", theme.text_dim)
            };

            let title_text = if is_sel {
                marquee_window(&v.title, title_w, state.ticker_step)
            } else {
                v.title.chars().take(title_w).collect()
            };

            let line1 = Line::from(vec![
                Span::styled(
                    if is_sel { " ▸ " } else { "   " },
                    Style::default().fg(theme.primary).bg(row_bg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(badge, Style::default().fg(badge_col).bg(row_bg)),
                Span::styled(
                    format!("{:<w$}", title_text, w = title_w),
                    Style::default()
                        .fg(if is_sel { theme.primary } else { theme.text })
                        .bg(row_bg)
                        .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ]);

            let views_s = v.compact_views(false).unwrap_or_else(|| "—".to_string());
            let date_s = v.compact_age(now_unix, false).unwrap_or_else(|| "—".to_string());
            let dur_s = v.display_duration();
            let up = v.display_uploader();
            let sub_meta = format!("        {up} · {views_s} · {date_s} · {dur_s}");
            let padded_meta = format!("{:<w$}", sub_meta.chars().take(list_inner.width as usize).collect::<String>(), w = list_inner.width as usize);
            let line2 = Line::from(Span::styled(
                padded_meta,
                Style::default().fg(theme.text_dim).bg(row_bg),
            ));

            frame.render_widget(Paragraph::new(vec![line1, line2]), row_rect);
            state.search_hitboxes.push((row_rect, SearchHit::SelectRow(idx)));
        }

        // Right Pane: Live 16:9 Preview Inspector + Interactive Quick Action Buttons
        let right_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(2)])
            .split(panes[1]);

        if let Some(sel) = combined.get(cur) {
            let mut badges: Vec<(&str, Color)> = Vec::new();
            if state.watched_ids.contains(&sel.id) {
                badges.push(("WATCHED", theme.accent));
            }
            if state.watch_later_sources.contains_key(&sel.id) {
                badges.push(("WL", Color::Rgb(251, 191, 36)));
            }
            render_video_preview_inspector(
                frame,
                right_split[0],
                state,
                sel,
                now_unix,
                &badges,
                None,
                "",
            );
        }

        let actions = vec![
            (
                " [Detail(Enter)] ".to_string(),
                Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD),
                Some(SearchHit::ActionDetail),
            ),
            (" ".to_string(), Style::default(), None),
            (
                " [Play(^p)] ".to_string(),
                Style::default().fg(Color::Black).bg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD),
                Some(SearchHit::ActionPlay),
            ),
            (" ".to_string(), Style::default(), None),
            (
                "[+Queue(^a)]".to_string(),
                Style::default().fg(theme.secondary).bg(Color::Rgb(24, 28, 40)),
                Some(SearchHit::ActionPinCw),
            ),
            (" ".to_string(), Style::default(), None),
            (
                "[+WL]".to_string(),
                Style::default().fg(Color::Rgb(251, 191, 36)).bg(Color::Rgb(24, 28, 40)),
                Some(SearchHit::ActionWatchLater),
            ),
            (" ".to_string(), Style::default(), None),
            (
                "[Channel]".to_string(),
                Style::default().fg(theme.text).bg(Color::Rgb(24, 28, 40)),
                Some(SearchHit::ActionChannel),
            ),
        ];
        render_pill_strip(frame, right_split[1], actions, &mut state.search_hitboxes);
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Up/Down] Select ", Style::default().fg(theme.primary)),
            Span::styled("· [Tab] Complete ", Style::default().fg(Color::Rgb(56, 189, 248))),
            Span::styled("· [^t/^d/^u/^s] Filter/Sort ", Style::default().fg(theme.secondary)),
            Span::styled("· [^w] HideWatched ", Style::default().fg(theme.accent)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        chunks[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::cache::VideoItem;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_search_studio_filters_ranking_and_render() {
        let mut st = AppState::new(Config::default());
        st.overlay = ActiveOverlay::Search;
        st.search_query = "xpetu".to_string();
        st.search_suggestions = vec!["xpetu shen".to_string(), "xpetu build".to_string()];
        st.search_results = vec![
            VideoItem {
                id: "vid_short".into(),
                title: "Quick Shen Outplay".into(),
                uploader: Some("xPetu".into()),
                duration: Some(45),
                duration_string: Some("SHORT".into()),
                view_count: Some(50_000),
                ..Default::default()
            },
            VideoItem {
                id: "vid_long".into(),
                title: "Why Everyone Is Playing Full AP Shen".into(),
                uploader: Some("xPetu".into()),
                duration: Some(1224),
                view_count: Some(239_000),
                ..Default::default()
            },
        ];

        assert_eq!(st.combined_search_items().len(), 2);
        st.search_type_filter = SearchTypeFilter::Videos;
        let only_vids = st.combined_search_items();
        assert_eq!(only_vids.len(), 1);
        assert_eq!(only_vids[0].id, "vid_long");

        let backend = TestBackend::new(130, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_search_modal(f, &mut st)).unwrap();
        assert!(!st.search_hitboxes.is_empty());
    }
}
