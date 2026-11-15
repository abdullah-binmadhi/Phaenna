//! AirPlay 2 & DLNA Smart TV Cast Modal (`C`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render_cast_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(68, 60, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " AIRPLAY 2 & DLNA TV CASTING ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.secondary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(inner);

    let selected = state.active_selected_video();
    let header = if state.is_scanning_cast {
        "Scanning LAN via SSDP multicast (239.255.255.250:1900)..."
    } else if let Some(ref v) = selected {
        &v.title
    } else {
        "Select a video first to cast to a discovered Smart TV"
    };

    frame.render_widget(
        Paragraph::new(Span::styled(
            header,
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );

    if state.cast_devices.is_empty() {
        frame.render_widget(
            Paragraph::new("No Smart TV / DLNA MediaRenderer receivers found yet.\nPress [s] to re-scan your local network.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            rows[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .cast_devices
            .iter()
            .enumerate()
            .map(|(i, dev)| {
                let sel = i == state.cast_cursor;
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if sel { " ▸ " } else { "   " },
                        Style::default().fg(theme.primary),
                    ),
                    Span::styled(
                        format!("[{}] ", dev.protocol.badge()),
                        Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(&dev.name, Style::default().fg(theme.text)),
                    Span::styled(format!(" ({})", dev.endpoint()), Style::default().fg(theme.text_dim)),
                ]))
            })
            .collect();
        let mut ls = ListState::default();
        ls.select(Some(state.cast_cursor.min(state.cast_devices.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().bg(Color::Rgb(34, 40, 56))),
            rows[1],
            &mut ls,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [s] Re-Scan LAN ", Style::default().fg(theme.primary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Enter] Cast Selected Video ", Style::default().fg(theme.accent)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc] Close ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
