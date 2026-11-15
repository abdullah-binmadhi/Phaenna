//! Channel Drill-Down Screen (`Screen::Channel`).
//! Launched from `[c]` inside Video Detail; pushes cleanly so `Esc` pops back.

use crate::state::AppState;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub fn render_channel_screen(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = frame.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    let header_blk = Block::default()
        .title(Span::styled(
            format!(" 📺 CHANNEL UPLOADS — {} ", state.channel_title),
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let subtitle = if state.is_loading_channel {
        "⟳ Fetching latest uploads & Atom feed...".to_string()
    } else {
        format!(
            "{} videos indexed · ID: {}",
            state.channel_videos.len(),
            state.channel_id.as_deref().unwrap_or("handle")
        )
    };
    frame.render_widget(
        Paragraph::new(Span::styled(subtitle, Style::default().fg(theme.accent))).block(header_blk),
        rows[0],
    );

    let list_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let list_inner = list_blk.inner(rows[1]);
    frame.render_widget(list_blk, rows[1]);

    if state.channel_videos.is_empty() {
        frame.render_widget(
            Paragraph::new("Loading channel catalog...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            list_inner,
        );
    } else {
        let max_vis = list_inner.height as usize;
        let cur = state.channel_cursor.min(state.channel_videos.len().saturating_sub(1));
        let start = if cur >= max_vis && max_vis > 0 { cur + 1 - max_vis } else { 0 };
        let mut lines = Vec::new();
        for (idx, v) in state.channel_videos.iter().enumerate().skip(start).take(max_vis) {
            let sel = idx == cur;
            let bg = if sel { Color::Rgb(32, 38, 54) } else { Color::Reset };
            let badge = if state.watched_ids.contains(&v.id) {
                Span::styled("✓ ", Style::default().fg(theme.accent).bg(bg))
            } else {
                Span::styled("  ", Style::default().bg(bg))
            };
            lines.push(Line::from(vec![
                Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(theme.primary).bg(bg)),
                badge,
                Span::styled(
                    format!("{:<58} ", v.title.chars().take(58).collect::<String>()),
                    Style::default().fg(if sel { theme.primary } else { theme.text }).bg(bg).add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(
                    format!("[{}]", v.display_duration()),
                    Style::default().fg(theme.accent).bg(bg),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), list_inner);
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[j/k] Navigate ", Style::default().fg(theme.primary)),
            Span::styled("· [Enter] Open Detail ", Style::default().fg(theme.accent)),
            Span::styled("· [p] Play Immediate ", Style::default().fg(theme.secondary)),
            Span::styled("· [Esc] Back to Detail", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
