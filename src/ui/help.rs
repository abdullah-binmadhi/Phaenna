//! Responsive 2x3 Content-Clamped Bento Command Matrix (`?`) & Non-Blocking Toast.

use crate::state::{active_modal_rect, centered_rect, help_modal_rect, ActiveOverlay, AppState, HelpHit, Screen};
use crate::ui::theme::ThemePalette;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

fn truncate_words(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if max_chars <= 3 {
        return trimmed.chars().take(max_chars).collect();
    }
    let limit = max_chars - 3;
    let mut byte_end = 0usize;
    let mut last_space = None;
    for (i, (b_idx, ch)) in trimmed.char_indices().enumerate() {
        if i >= limit {
            break;
        }
        byte_end = b_idx + ch.len_utf8();
        if ch.is_whitespace() {
            last_space = Some(b_idx);
        }
    }
    let cut = last_space.filter(|&p| p >= byte_end / 2).unwrap_or(byte_end);
    format!("{}...", trimmed[..cut].trim_end())
}

fn cmd_row(
    key_str: &str,
    desc_str: &str,
    scope_tag: &str,
    inner_w: u16,
    theme: &ThemePalette,
) -> Line<'static> {
    let key_w = if inner_w >= 50 { 15 } else { 12 };
    let key_fit: String = if key_str.chars().count() > key_w {
        key_str.chars().take(key_w).collect()
    } else {
        key_str.to_string()
    };
    let key_col = format!(" {key_fit:<key_w$} ");
    let fixed_left = key_w + 4; // key_col (key_w + 2) + "│ " (2)

    let tag_full = format!(" [{scope_tag}] ");
    let tag_compact = format!(" [{scope_tag}]");
    let tag_col = if (inner_w as usize) >= fixed_left + 26 + tag_full.chars().count() {
        tag_full
    } else if (inner_w as usize) >= fixed_left + 16 + tag_compact.chars().count() {
        tag_compact
    } else {
        String::new()
    };

    let desc_budget = (inner_w as usize).saturating_sub(fixed_left + tag_col.chars().count());
    let fitted = truncate_words(desc_str, desc_budget);
    let padded_desc = format!("{fitted:<desc_budget$}");

    let mut spans = vec![
        Span::styled(
            key_col,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(theme.divider)),
        Span::styled(padded_desc, Style::default().fg(theme.text)),
    ];
    if !tag_col.is_empty() {
        spans.push(Span::styled(tag_col, Style::default().fg(theme.text_dim)));
    }
    Line::from(spans)
}

fn render_session_strip(frame: &mut Frame, area: Rect, state: &AppState, theme: &ThemePalette) {
    if area.width < 20 || area.height == 0 {
        return;
    }
    let screen_lbl = match state.screen {
        Screen::Home => "HOME",
        Screen::Detail => "DETAIL",
        Screen::Channel => "CHANNEL",
    };
    let shader_lbl = if area.width >= 106 {
        state.config.shader_mode.short_label()
    } else {
        state.config.shader_mode.as_str()
    };
    let left_text = format!(
        " SESSION: {screen_lbl} | Player: {} {} | Shader: {shader_lbl} | Perf: {} {} | Live: {} ",
        state.config.player.as_str(),
        state.config.quality.short_badge(),
        state.config.performance_mode.short_badge(),
        state.config.motion_mode.short_badge(),
        state.config.live_mode.short_badge(),
    );
    let auth_text = if state.auth_vault.is_authenticated() {
        " AUTH: Connected [L] "
    } else {
        " AUTH: Guest [L] "
    };

    let w = area.width as usize;
    let auth_len = auth_text.chars().count();
    let max_left = w.saturating_sub(auth_len + 1);
    let left_fitted = truncate_words(&left_text, max_left);
    let left_len = left_fitted.chars().count();
    let gap = w.saturating_sub(left_len + auth_len);
    let strip_bg = Color::Rgb(24, 26, 38);

    let line = Line::from(vec![
        Span::styled(
            left_fitted,
            Style::default()
                .fg(theme.primary)
                .bg(strip_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(gap), Style::default().bg(strip_bg)),
        Span::styled(
            auth_text,
            Style::default()
                .fg(if state.auth_vault.is_authenticated() {
                    Color::Rgb(52, 211, 153)
                } else {
                    theme.accent
                })
                .bg(strip_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub fn render_help_modal(frame: &mut Frame, state: &mut AppState) {
    state.help_hitboxes.clear();
    let theme = state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::Help)
        .unwrap_or_else(|| help_modal_rect(frame.area()));
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

    if inner.width < 30 || inner.height < 10 {
        return;
    }

    let compact_mode = inner.height < 26;
    let rows = if compact_mode {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Length(1),
            ])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Session strip
                Constraint::Length(8), // Bento Row 1
                Constraint::Length(8), // Bento Row 2
                Constraint::Length(8), // Bento Row 3
                Constraint::Min(1),    // Interactive Quick-Launch Footer
            ])
            .split(inner)
    };

    render_session_strip(frame, rows[0], state, theme);

    let r1_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    let r2_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[2]);
    let r3_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[3]);

    let cards = [
        (
            r1_cols[0],
            " 1. Navigation & Feed Shelves ",
            [
                ("h/l, j/k", "Move grid or queue cursor", "Feed"),
                ("PgDn/^d, PgUp", "Scroll page & load more", "Page"),
                ("g/Home, G/End", "Jump top / end (+fetch)", "Jump"),
                ("Tab/S-Tab, 1..6", "Switch feed shelf 1..6", "Tab"),
                ("S / f", "Shorts src / Filter+Sort", "View"),
                ("r / n / ]", "Refresh all / Next batch", "Sync"),
            ],
        ),
        (
            r1_cols[1],
            " 2. Playback, Queue & Live Stream ",
            [
                ("Space / Enter", "Instant Play / Detail", "Play"),
                ("p", "Open Playback Studio & EQ", "Studio"),
                ("E / S / R / L", "Edge / Start / Radio / Mode", "Live"),
                ("a / +, w, D", "Queue / WatchLater / Save", "Lib"),
                ("v / X", "Queue Grid / Clear done", "Queue"),
                ("x / d, K / J", "Remove / Move Queue slot", "Queue"),
            ],
        ),
        (
            r2_cols[0],
            " 3. Detail Deck & Reddit Comments ",
            [
                ("i / Enter, Esc", "Open Detail / Pop history", "Deck"),
                ("Tab / Shift+Tab", "Focus Info / Chap / Tree", "Pane"),
                ("u / v, c", "Like / Dislike / Channel", "Vote"),
                ("h / l, Enter", "Fold thread / Jump parent", "Tree"),
                ("m / r / n", "Comment / Reply / More", "Post"),
                ("t / T / g", "Translate node/thread/lng", "AI-TL"),
            ],
        ),
        (
            r2_cols[1],
            " 4. Playback Studio (p) & Live IPC ",
            [
                ("h/l/Tab, j/k", "Focus pane / Adjust value", "Studio"),
                ("s / m / t / v", "Shader, Perf, 120Hz, Live", "Eng"),
                ("d / L / n", "AI Dub / Lang / Loudnorm", "Audio"),
                ("c / y / o", "Subs / Style / Skip Outro", "Subs"),
                ("space, -/+, [/]", "Pause / Vol+-5% / Speed", "IPC"),
                ("p/P, i/I, g, X", "PiP / Bento HUD / Gov / Stop", "mpv"),
            ],
        ),
        (
            r3_cols[0],
            " 5. Search (/), Analysis (A) & Radar ",
            [
                ("/, Tab, Enter", "Search / Suggest / Detail", "Find"),
                ("^t ^d ^u ^s", "Type, Date, Dur & Sort", "Filter"),
                ("^w ^g, ^n", "Watched / Scope / Load+", "Scope"),
                ("^p ^a ^l ^c", "Play / Queue / Later / Ch", "Act"),
                ("A, 1..3, h/l", "Tabs, Range, o:YT b/B:Goal", "Stats"),
                ("c, h/l, [/], f", "Radar day, week & status", "Radar"),
            ],
        ),
        (
            r3_cols[1],
            " 6. System, Companion, Auth & Mouse ",
            [
                ("s / ,", "Fuzzy Command Hub", "Sys"),
                ("L / T / U", "Account / Theme / Update", "Auth"),
                ("m / C", "QR Remote / TV Cast Scan", "Cast"),
                ("Left / DblClick", "Click UI / Fold comments", "Mouse"),
                ("Right-Click", "Close modal or step back", "Mouse"),
                ("Mouse Wheel", "Scroll feed, tree & chaps", "Mouse"),
            ],
        ),
    ];

    for (rect, title, entries) in cards {
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
        let card_inner = blk.inner(rect);
        let max_lines = card_inner.height as usize;
        let lines: Vec<Line> = entries
            .iter()
            .take(max_lines)
            .map(|(k, d, tag)| cmd_row(k, d, tag, card_inner.width, theme))
            .collect();
        frame.render_widget(Paragraph::new(lines).block(blk), rect);
    }

    let footer_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(14),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(14),
        ])
        .split(rows[4]);

    let footer_actions = [
        ("[s]", "Fuzzy Hub", HelpHit::OpenSettings, theme.primary),
        ("[/]", "Search", HelpHit::OpenSearch, theme.primary),
        ("[p]", "Studio", HelpHit::OpenStudio, theme.primary),
        ("[A]", "Stats", HelpHit::OpenAnalytics, theme.secondary),
        ("[c]", "Radar", HelpHit::OpenCalendar, theme.secondary),
        ("[L]", "Auth", HelpHit::OpenLogin, theme.accent),
        ("[T]", "Theme", HelpHit::CycleTheme, theme.accent),
        ("[Esc/?]", "Close", HelpHit::Close, theme.text_dim),
    ];

    for (idx, (k, lbl, hit, col)) in footer_actions.into_iter().enumerate() {
        let r = footer_cols[idx];
        let pill = Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{k} "),
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            ),
            Span::styled(lbl, Style::default().fg(theme.text)),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(pill, r);
        state.help_hitboxes.push((r, hit));
    }
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
        let mut st = AppState::new(Config::default());
        let backend = TestBackend::new(110, 34);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            render_help_modal(f, &mut st);
            render_toast(f, "Loaded 24 videos from SQLite WAL", &st);
        })
        .unwrap();
    }

    #[test]
    fn test_help_modal_zero_dead_space_no_word_concat_and_hitboxes() {
        let mut st = AppState::new(Config::default());
        st.overlay = ActiveOverlay::Help;
        let full_area = Rect::new(0, 0, 170, 48);
        let modal = help_modal_rect(full_area);
        assert_eq!(modal.width, 116);
        assert_eq!(modal.height, 28);
        assert_eq!(active_modal_rect(full_area, ActiveOverlay::Help), Some(modal));

        let backend = TestBackend::new(170, 48);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_help_modal(f, &mut st)).unwrap();

        let buf = term.backend().buffer();
        let mut rows_text = Vec::new();
        for y in modal.y..modal.y + modal.height {
            let line: String = (modal.x..modal.x + modal.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            // Zero dead vertical rows inside modal: left and right halves both contain visible content
            let chars: Vec<char> = line.chars().collect();
            let left_non_border: String = chars[1..chars.len() / 2].iter().collect();
            let right_non_border: String = chars[chars.len() / 2..chars.len() - 1].iter().collect();
            assert!(
                !left_non_border.trim().is_empty(),
                "Dead/empty left row at y={y}: {line:?}"
            );
            assert!(
                !right_non_border.trim().is_empty(),
                "Dead/empty right row at y={y}: {line:?}"
            );
            rows_text.push(line);
        }

        let joined = rows_text.join("\n");
        assert!(!joined.contains("SpaceWatch"));
        for card_title in [
            "1. Navigation & Feed Shelves",
            "2. Playback, Queue & Live Stream",
            "3. Detail Deck & Reddit Comments",
            "4. Playback Studio (p) & Live IPC",
            "5. Search (/), Analysis (A) & Radar",
            "6. System, Companion, Auth & Mouse",
        ] {
            assert!(joined.contains(card_title), "Missing card title: {card_title}");
        }

        // Every command table row must have explicit spacing around `│` and before `[Scope]`
        let cmd_lines: Vec<&str> = rows_text
            .iter()
            .filter(|l| l.contains(" │ "))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(cmd_lines.len(), 18, "Expected 18 dual-card rows (36 card entries total)");
        for line in cmd_lines {
            assert!(line.contains(" │ "), "Missing space around divider: {line:?}");
            assert!(line.contains(" ["), "Missing space before scope tag: {line:?}");
        }

        // Verify all 8 interactive footer hitboxes exist and do not overlap
        assert_eq!(st.help_hitboxes.len(), 8);
        for w in st.help_hitboxes.windows(2) {
            assert!(w[0].0.x + w[0].0.width <= w[1].0.x);
        }

        // Verify compact terminal fallback fits cleanly without panicking
        let compact_backend = TestBackend::new(96, 25);
        let mut compact_term = Terminal::new(compact_backend).unwrap();
        compact_term.draw(|f| render_help_modal(f, &mut st)).unwrap();
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
