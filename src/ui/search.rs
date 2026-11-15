//! Instant Local SQLite + 350ms Debounced YouTube Live Search Modal (`/`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render_search_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(76, 78, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " 󰍉 PHAENNA — INSTANT LOCAL + LIVE SEARCH ",
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
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(inner);

    let input_blk = Block::default()
        .title(Span::styled(" Query ", Style::default().fg(theme.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus));

    let status_suffix = if state.is_searching_remote {
        "  (⟳ fetching YouTube...)"
    } else if state.search_pending_remote {
        "  (⏳ waiting debounce...)"
    } else {
        ""
    };

    let input_line = Line::from(vec![
        Span::styled(&state.search_query, Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        Span::styled("█", Style::default().fg(theme.primary)),
        Span::styled(status_suffix, Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(input_line).block(input_blk), chunks[0]);

    let combined = state.combined_search_items();
    let meta = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} results ", combined.len()),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
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
        let items: Vec<ListItem> = combined
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let is_sel = i == state.search_cursor;
                let watched_badge = if state.watched_ids.contains(&v.id) { "✓ " } else { "" };
                let dur = v.display_duration();
                let uploader = v.display_uploader();
                let line = Line::from(vec![
                    Span::styled(
                        if is_sel { " ▸ " } else { "   " },
                        Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(watched_badge, Style::default().fg(theme.accent)),
                    Span::styled(
                        format!("{:<48}", v.title.chars().take(48).collect::<String>()),
                        Style::default()
                            .fg(if is_sel { theme.primary } else { theme.text })
                            .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        format!("  {} · {}", uploader.chars().take(18).collect::<String>(), dur),
                        Style::default().fg(theme.text_dim),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let mut ls = ListState::default();
        ls.select(Some(state.search_cursor.min(combined.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().bg(Color::Rgb(32, 38, 52))),
            chunks[2],
            &mut ls,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[↑/↓] Select ", Style::default().fg(theme.primary)),
            Span::styled("· [Enter] Open Detail ", Style::default().fg(theme.accent)),
            Span::styled("· [p] Play ", Style::default().fg(theme.secondary)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        chunks[3],
    );
}
