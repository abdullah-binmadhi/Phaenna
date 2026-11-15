//! Instant Local SQLite + 350ms Debounced YouTube Live Search Modal (`/`).
//! Split-pane results + Live 16:9 Preview Inspector so every search hit shows
//! edge-to-edge cover art, views, release age, and synopsis with zero dead space.

use crate::state::{centered_rect, AppState};
use crate::ui::components::cover::{render_video_preview_inspector, unix_now_secs};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render_search_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = state.base_theme();
    let area = centered_rect(88, 84, frame.area());
    frame.render_widget(Clear, area);
    let now_unix = unix_now_secs();

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA — INSTANT LOCAL + LIVE SEARCH ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(inner);

    let input_blk = Block::default()
        .title(Span::styled(" Query ", Style::default().fg(theme.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus));

    let status_suffix = if state.is_searching_remote {
        "  (fetching YouTube...)"
    } else if state.search_pending_remote {
        "  (waiting debounce...)"
    } else {
        ""
    };

    let input_line = Line::from(vec![
        Span::styled(
            &state.search_query,
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(theme.primary)),
        Span::styled(status_suffix, Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(input_line).block(input_blk), chunks[0]);

    let combined = state.combined_search_items();
    let meta = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} results ", combined.len()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "({} remote · {} local SQLite cache)",
                state.search_results.len(),
                state.search_local_results.len()
            ),
            Style::default().fg(theme.text_dim),
        ),
    ]));
    frame.render_widget(meta, chunks[1]);

    if combined.is_empty() {
        let msg = if state.search_query.trim().is_empty() {
            "Start typing to search local SQLite cache instantaneously + live YouTube (350ms debounce)."
        } else if state.is_searching_remote {
            "Searching YouTube..."
        } else {
            "No matching videos found yet. Press Enter to force query."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            chunks[2],
        );
    } else {
        // 58% Left Results List / 42% Right Live 16:9 Preview Inspector
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(chunks[2]);

        let list_blk = Block::default()
            .title(Span::styled(
                " RESULTS ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        let list_inner = list_blk.inner(panes[0]);
        frame.render_widget(list_blk, panes[0]);

        let cur = state.search_cursor.min(combined.len().saturating_sub(1));
        for v in combined.iter().take(12) {
            state.queue_thumb_if_needed(&v.id, &v.best_thumbnail_url());
        }
        if let Some(sel) = combined.get(cur) {
            state.queue_thumb_if_needed(&sel.id, &sel.best_thumbnail_url());
        }

        let total_w = list_inner.width as usize;
        let uploader_w = (total_w / 4).clamp(10, 18);
        let dur_w = 8usize;
        let prefix_w = 8usize;
        let title_w = total_w
            .saturating_sub(prefix_w + uploader_w + dur_w + 4)
            .max(12);

        let items: Vec<ListItem> = combined
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let is_sel = i == cur;
                let watched_badge = if state.watched_ids.contains(&v.id) {
                    "[OK] "
                } else if v.is_new_release(now_unix) {
                    "NEW  "
                } else {
                    "  ·  "
                };
                let badge_col = if state.watched_ids.contains(&v.id) {
                    theme.accent
                } else if v.is_new_release(now_unix) {
                    Color::Rgb(52, 211, 153)
                } else {
                    theme.text_dim
                };
                let dur = v.display_duration();
                let uploader: String = v.display_uploader().chars().take(uploader_w).collect();
                let title_text = if is_sel {
                    marquee_window(&v.title, title_w, state.ticker_step)
                } else {
                    v.title.chars().take(title_w).collect::<String>()
                };
                let line = Line::from(vec![
                    Span::styled(
                        if is_sel { " ▸ " } else { "   " },
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(watched_badge, Style::default().fg(badge_col)),
                    Span::styled(
                        format!("{:<width$}", title_text, width = title_w),
                        Style::default()
                            .fg(if is_sel { theme.primary } else { theme.text })
                            .add_modifier(if is_sel {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("  {:<uw$} · {}", uploader, dur, uw = uploader_w),
                        Style::default().fg(theme.text_dim),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let mut ls = ListState::default();
        ls.select(Some(cur));
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().bg(Color::Rgb(32, 38, 52))),
            list_inner,
            &mut ls,
        );

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
                panes[1],
                state,
                sel,
                now_unix,
                &badges,
                None,
                "[Enter] Detail · [^p] Play · [^a] +CW · [Esc] Close",
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[↑/↓] Select ", Style::default().fg(theme.primary)),
            Span::styled("· [Enter] Detail ", Style::default().fg(theme.accent)),
            Span::styled("· [^a] +Continue Watching ", Style::default().fg(theme.secondary)),
            Span::styled("· [^p] Play · [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        chunks[3],
    );
}
