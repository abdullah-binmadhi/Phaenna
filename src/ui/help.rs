//! Responsive 2x2 Bento Shortcut Matrix (`?`) & Non-Blocking Floating Toast.

use crate::state::{active_modal_rect, centered_rect, ActiveOverlay, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_help_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(80, 74, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA — KEYBOARD & MOUSE COMMAND MATRIX ",
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

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(46),
            Constraint::Percentage(46),
            Constraint::Length(2),
        ])
        .split(inner);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let key = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(theme.text);

    let cards = [
        (
            top_cols[0],
            " Navigation & Infinite Scroll ",
            vec![
                Line::from(vec![Span::styled(" h/l, j/k     ", key), Span::styled("Move grid / queue cursor", desc)]),
                Line::from(vec![Span::styled(" PgDn / ^d    ", key), Span::styled("Scroll down & auto-load more", desc)]),
                Line::from(vec![Span::styled(" Tab / 1..6   ", key), Span::styled("Switch shelf ([1] Queue · [3] Shorts)", desc)]),
                Line::from(vec![Span::styled(" S            ", key), Span::styled("Switch Shorts: Suggested / Subscribed", desc)]),
                Line::from(vec![Span::styled(" a / +        ", key), Span::styled("Save video to Continue Watching", desc)]),
                Line::from(vec![Span::styled(" x/d · K/J    ", key), Span::styled("Remove / reorder in Continue Watching", desc)]),
            ],
        ),
        (
            top_cols[1],
            " Playback, Queue & Detail ",
            vec![
                Line::from(vec![Span::styled(" Enter / Space", key), Span::styled("Watch in Queue / Play or Detail", desc)]),
                Line::from(vec![Span::styled(" i            ", key), Span::styled("Detail Deck (Tab: Chaps/Comments · m Comment)", desc)]),
                Line::from(vec![Span::styled(" p            ", key), Span::styled("Play Options (Quality/Player/Subs)", desc)]),
                Line::from(vec![Span::styled(" v / X        ", key), Span::styled("Toggle Queue Grid / Clear finished", desc)]),
                Line::from(vec![Span::styled(" D / n        ", key), Span::styled("Offline download / Force load more", desc)]),
            ],
        ),
        (
            bot_cols[0],
            " Discovery & Intelligence ",
            vec![
                Line::from(vec![Span::styled(" / (^a Queue) ", key), Span::styled("Debounced Live Search", desc)]),
                Line::from(vec![Span::styled(" f            ", key), Span::styled("Filter (Shorts/Long) & Sort", desc)]),
                Line::from(vec![Span::styled(" A            ", key), Span::styled("Phaenna Wrapped Analytics", desc)]),
                Line::from(vec![Span::styled(" t            ", key), Span::styled("Tailored Taste Discovery", desc)]),
                Line::from(vec![Span::styled(" c            ", key), Span::styled("7-Day Upload Radar", desc)]),
            ],
        ),
        (
            bot_cols[1],
            " System, Companion & Auth ",
            vec![
                Line::from(vec![Span::styled(" s / ,        ", key), Span::styled("Fuzzy Command Hub", desc)]),
                Line::from(vec![Span::styled(" m / C        ", key), Span::styled("LAN Remote QR / TV Cast", desc)]),
                Line::from(vec![Span::styled(" L / T        ", key), Span::styled("Google Account / Cycle Theme", desc)]),
                Line::from(vec![Span::styled(" ! / U        ", key), Span::styled("System Doctor / Updater", desc)]),
                Line::from(vec![Span::styled(" w / r / q    ", key), Span::styled("WatchLater / Refresh / Quit", desc)]),
            ],
        ),
    ];

    for (rect, title, lines) in cards {
        let blk = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        frame.render_widget(Paragraph::new(lines).block(blk), rect);
    }

    let hint = Paragraph::new(Line::from(vec![
        Span::styled(" [Esc / ?] Close Matrix ", Style::default().fg(theme.text_dim)),
        Span::styled(" · ", Style::default().fg(theme.divider)),
        Span::styled("[s] Open Fuzzy Hub ", Style::default().fg(theme.primary)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(hint, rows[2]);
}

pub fn render_toast(frame: &mut Frame, msg: &str, state: &AppState) {
    let theme = state.base_theme();
    let area = frame.area();
    let max_w = area.width.saturating_sub(4).max(20);
    let toast_w = (msg.chars().count() as u16 + 6).clamp(28, max_w.min(64));
    let toast_h = 3;
    let rect = Rect {
        x: area.width.saturating_sub(toast_w + 2),
        y: area.height.saturating_sub(toast_h + 1),
        width: toast_w,
        height: toast_h,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));
    let p = Paragraph::new(msg)
        .alignment(Alignment::Center)
        .block(block)
        .style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD));
    frame.render_widget(p, rect);
}

fn confirm_quit_rows(modal_area: Rect) -> [Rect; 3] {
    let inner = Rect::new(
        modal_area.x.saturating_add(1),
        modal_area.y.saturating_add(1),
        modal_area.width.saturating_sub(2),
        modal_area.height.saturating_sub(2),
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(2),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);
    [chunks[0], chunks[1], chunks[2]]
}

/// Deterministic hit-test rectangles `(exit_btn, cancel_btn)` for the quit confirmation modal.
pub fn confirm_quit_button_rects(frame_area: Rect) -> (Rect, Rect) {
    let modal = active_modal_rect(frame_area, ActiveOverlay::ConfirmQuit)
        .unwrap_or_else(|| centered_rect(50, 30, frame_area));
    let [_, btn_row, _] = confirm_quit_rows(modal);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(8),
            Constraint::Percentage(39),
            Constraint::Percentage(6),
            Constraint::Percentage(39),
            Constraint::Percentage(8),
        ])
        .split(btn_row);
    (cols[1], cols[3])
}

pub fn render_confirm_quit_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::ConfirmQuit)
        .unwrap_or_else(|| centered_rect(50, 30, frame.area()));
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " CONFIRM EXIT ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));
    frame.render_widget(outer, area);

    let [msg_row, _, hint_row] = confirm_quit_rows(area);
    let (exit_rect, cancel_rect) = confirm_quit_button_rects(frame.area());

    let prompt = Paragraph::new(vec![
        Line::from(Span::styled(
            "Do you want to exit Phaenna?",
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Press Enter on your selection, [y] to exit, or [n / Esc] to cancel.",
            Style::default().fg(theme.text_dim),
        )),
    ])
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true });
    frame.render_widget(prompt, msg_row);

    let exit_active = state.quit_confirm_yes;
    let exit_color = Color::Rgb(239, 68, 68);
    let exit_style = if exit_active {
        Style::default()
            .fg(Color::Black)
            .bg(exit_color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(exit_color).add_modifier(Modifier::BOLD)
    };
    let exit_btn = Paragraph::new("Exit Phaenna (y)")
        .alignment(Alignment::Center)
        .style(exit_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if exit_active {
                    exit_color
                } else {
                    theme.border_idle
                })),
        );
    frame.render_widget(exit_btn, exit_rect);

    let cancel_active = !state.quit_confirm_yes;
    let cancel_style = if cancel_active {
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };
    let cancel_btn = Paragraph::new("Cancel (n)")
        .alignment(Alignment::Center)
        .style(cancel_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if cancel_active {
                    theme.primary
                } else {
                    theme.border_idle
                })),
        );
    frame.render_widget(cancel_btn, cancel_rect);

    frame.render_widget(
        Paragraph::new("[Left/Right] Select · [y] Exit · [n / Esc / q] Cancel")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        hint_row,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_help_and_toast_render() {
        let st = AppState::new(Config::default());
        let backend = TestBackend::new(110, 34);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            render_help_modal(f, &st);
            render_toast(f, "Loaded 24 videos from SQLite WAL", &st);
        })
        .unwrap();
    }

    #[test]
    fn test_confirm_quit_modal_renders_and_exposes_buttons() {
        let mut st = AppState::new(Config::default());
        st.overlay = ActiveOverlay::ConfirmQuit;
        let area = Rect::new(0, 0, 120, 40);
        let (exit_r, cancel_r) = confirm_quit_button_rects(area);
        assert!(exit_r.width > 10 && cancel_r.width > 10);
        assert!(exit_r.x + exit_r.width < cancel_r.x);

        let backend = TestBackend::new(120, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_confirm_quit_modal(f, &st)).unwrap();

        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Do you want to exit Phaenna?"));
        assert!(text.contains("Exit Phaenna (y)"));
        assert!(text.contains("Cancel (n)"));
    }
}
