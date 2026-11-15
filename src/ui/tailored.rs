//! Discovery Crucible (`t`): 6-Step Taste Constellation Wizard,
//! Braille Pentagon Persona Radar, and Real-Time Live Vibe Tuner.

use crate::services::tailored::{build_questions, synthesize_persona};
use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, BorderType, Borders, Clear, Paragraph, Wrap,
    },
    Frame,
};

pub fn render_tailored_wizard(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(78, 72, frame.area());
    frame.render_widget(Clear, area);

    let questions = build_questions();
    let step = state.wizard_step.min(questions.len().saturating_sub(1));
    let q = &questions[step];

    let outer = Block::default()
        .title(Span::styled(
            format!(" ✦ DISCOVERY CRUCIBLE — STEP {}/{} ", step + 1, questions.len()),
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
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!("{} {} ", q.symbol, q.title), Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(Span::styled(q.subtitle, Style::default().fg(theme.text_dim))),
        ]),
        rows[0],
    );

    let mut opt_lines = Vec::new();
    for (idx, opt) in q.options.iter().enumerate() {
        let is_cursor = idx == state.wizard_cursor;
        let is_chosen = if q.is_multi_select {
            state.config.tailored_answers.deal_breakers.contains(&idx)
        } else {
            state.config.tailored_answers.choices.get(step) == Some(&idx)
        };
        let mark = if is_chosen { "x" } else { " " };
        let st = if is_cursor {
            Style::default().fg(Color::Black).bg(theme.secondary).add_modifier(Modifier::BOLD)
        } else if is_chosen {
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        opt_lines.push(Line::from(vec![
            Span::styled(format!(" [{}] {} {} ", mark, opt.symbol, opt.label), st),
        ]));
        opt_lines.push(Line::from(Span::styled(
            format!("       {}", opt.detail),
            Style::default().fg(theme.text_dim),
        )));
        opt_lines.push(Line::from(""));
    }

    frame.render_widget(Paragraph::new(opt_lines).wrap(Wrap { trim: true }), rows[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[↑/↓] Choose ", Style::default().fg(theme.primary)),
            Span::styled(
                if q.is_multi_select { "· [Space] Toggle · [Enter] Finish " } else { "· [Enter] Select & Next " },
                Style::default().fg(theme.accent),
            ),
            Span::styled("· [s] Skip to Results · [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}

pub fn render_tailored_summary(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(84, 76, frame.area());
    frame.render_widget(Clear, area);

    let persona = synthesize_persona(&state.config.tailored_answers);
    let outer = Block::default()
        .title(Span::styled(
            format!(" ✦ YOUR VIEWER PERSONA: {} ", persona.title.to_uppercase()),
            Style::default()
                .fg(Color::Black)
                .bg(persona.primary_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(persona.primary_color));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(10), Constraint::Length(2)])
        .split(inner);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[0]);

    let gauges = persona.gauges;
    let prim = persona.primary_color;
    let acc = persona.accent_color;
    let radar = Canvas::default()
        .block(
            Block::default()
                .title(Span::styled(" ✦ Pentagon Taste Constellation ", Style::default().fg(prim).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_idle)),
        )
        .marker(Marker::Braille)
        .x_bounds([-1.3, 1.3])
        .y_bounds([-1.3, 1.3])
        .paint(move |ctx| {
            let mut pts = Vec::with_capacity(5);
            for (i, &(_, val)) in gauges.iter().enumerate() {
                let angle = (i as f64) * std::f64::consts::TAU / 5.0 - std::f64::consts::FRAC_PI_2;
                let r = (val as f64 / 100.0).clamp(0.15, 1.0);
                pts.push((r * angle.cos(), r * angle.sin()));
            }
            for i in 0..5 {
                let a = pts[i];
                let b = pts[(i + 1) % 5];
                ctx.draw(&CanvasLine {
                    x1: a.0,
                    y1: a.1,
                    x2: b.0,
                    y2: b.1,
                    color: prim,
                });
            }
            ctx.draw(&Points { coords: &pts, color: acc });
        });
    frame.render_widget(radar, cols[0]);

    let mut right_lines = vec![
        Line::from(Span::styled(
            persona.title,
            Style::default().fg(persona.primary_color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(persona.epithet, Style::default().fg(theme.text))),
        Line::from(""),
        Line::from(Span::styled("Dimension Axes:", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))),
    ];
    for (name, val) in persona.gauges {
        let blocks = (val as usize) / 8;
        right_lines.push(Line::from(vec![
            Span::styled(format!(" {:<8} ", name), Style::default().fg(theme.text_dim)),
            Span::styled("█".repeat(blocks), Style::default().fg(persona.accent_color)),
            Span::styled(format!(" {}%", val), Style::default().fg(theme.text)),
        ]));
    }
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled("Resonance: ", Style::default().fg(theme.text_dim)),
        Span::styled(persona.keywords.join(" · "), Style::default().fg(theme.accent)),
    ]));

    frame.render_widget(Paragraph::new(right_lines).wrap(Wrap { trim: true }), cols[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter] View Scored Recommendations ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled("· [r] Retake Quiz ", Style::default().fg(theme.primary)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[1],
    );
}

pub fn render_tailored_results(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(88, 84, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            format!(" TAILORED DISCOVERY SHELF — Vibe Tier: {} ", state.live_tuner.tier_label()),
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

    let tuner_line = Line::from(vec![
        Span::styled("Live Vibe Tuner: ", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("[v] Tier: {} ", state.live_tuner.tier_label()), Style::default().fg(theme.primary)),
        Span::styled(
            format!("· [s] Shorts Excluded: {} ", if state.live_tuner.exclude_shorts { "ON" } else { "OFF" }),
            Style::default().fg(theme.accent),
        ),
        Span::styled(
            format!("· {} scored matches", state.tailored_matches.len()),
            Style::default().fg(theme.text_dim),
        ),
    ]);
    frame.render_widget(Paragraph::new(tuner_line), rows[0]);

    let visible_h = rows[1].height as usize;
    let cursor = state.tailored_cursor.min(state.tailored_matches.len().saturating_sub(1));
    let start = if cursor >= visible_h && visible_h > 0 {
        cursor + 1 - visible_h
    } else {
        0
    };

    let mut lines = Vec::new();
    for (idx, m) in state.tailored_matches.iter().enumerate().skip(start).take(visible_h) {
        let sel = idx == cursor;
        let bg = if sel { Color::Rgb(36, 42, 58) } else { Color::Reset };
        lines.push(Line::from(vec![
            Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(theme.primary).bg(bg)),
            Span::styled(
                format!("[{:>2}%] ", m.match_percentage),
                Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<44} ", m.video.title.chars().take(44).collect::<String>()),
                Style::default().fg(if sel { theme.primary } else { theme.text }).bg(bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} · {} ", m.video.display_uploader().chars().take(16).collect::<String>(), m.reason_tags.join(" ")),
                Style::default().fg(theme.text_dim).bg(bg),
            ),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No cached videos match current Vibe Tuner filters. Press [0] to reset filters.",
            Style::default().fg(theme.text_dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter] Detail ", Style::default().fg(theme.accent)),
            Span::styled("· [v] Cycle Tier ", Style::default().fg(theme.primary)),
            Span::styled("· [s] Toggle Shorts ", Style::default().fg(theme.secondary)),
            Span::styled("· [x] Dismiss ", Style::default().fg(theme.text_dim)),
            Span::styled("· [z] Quiz · [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_tailored_screens_render() {
        let st = AppState::new(Config::default());
        let backend = TestBackend::new(110, 34);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_tailored_summary(f, &st)).unwrap();
        term.draw(|f| render_tailored_wizard(f, &st)).unwrap();
        term.draw(|f| render_tailored_results(f, &st)).unwrap();
    }
}
