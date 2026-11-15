//! Personal "YouTube Wrapped" & Viewing Analytics Dashboard (`a`).
//! Renders Braille 24-hour circadian waves, cumulative curves, duration buckets,
//! 182-day thermal heatmap, SponsorBlock time-saved counter, and Top Channels leaderboard.

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, BorderType, Borders, Clear, Paragraph,
    },
    Frame,
};

fn render_gradient_bar<'a>(label: &str, frac: f64, width: usize, color: Color) -> Line<'a> {
    let filled = ((frac.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    Line::from(vec![
        Span::styled(format!(" {:<9} ", label), Style::default().fg(Color::White)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(Color::Rgb(50, 56, 74))),
    ])
}

pub fn render_analytics_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(92, 88, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA WRAPPED — PERSONAL VIEWING INTELLIGENCE ",
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
            Constraint::Length(4),
            Constraint::Min(9),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

    let a = &state.analytics;
    let saved_mins = a.sponsorblock_saved_secs / 60.0;

    // 1. Hero Metric Bento Strip
    let hero_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[0]);

    let cards = [
        (
            " Watched & Time ",
            format!("{} vids · {:.1}h", a.total_videos_watched, a.total_hours_watched),
            format!("{} in progress ({:.0}% comp)", a.in_progress_videos, a.completion_rate),
            theme.primary,
        ),
        (
            " Viewer Archetype ",
            a.watch_archetype.to_string(),
            a.viewing_cadence.to_string(),
            theme.secondary,
        ),
        (
            " SponsorBlock Saved ",
            format!("{:.1} min skipped", saved_mins),
            format!("Peak Hour: {:02}:00", a.peak_hour),
            theme.accent,
        ),
        (
            " Streak & Vault ",
            format!("{}d active (max {}d)", a.active_streak_days, a.longest_streak_days),
            format!("{} offline · {} WL", a.offline_cached_videos, a.watch_later_count),
            theme.primary,
        ),
    ];

    for (idx, (title, main, sub, col)) in cards.iter().enumerate() {
        let b = Block::default()
            .title(Span::styled(*title, Style::default().fg(*col).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(main.clone(), Style::default().fg(theme.text).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(sub.clone(), Style::default().fg(theme.text_dim))),
            ])
            .block(b),
            hero_cols[idx],
        );
    }

    // 2. Middle Row: 24h Circadian Braille Wave | Duration Buckets & Thermal 182d Grid
    let mid_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let hourly_max = a.hourly_distribution.iter().copied().max().unwrap_or(1).max(1) as f64;
    let hourly = a.hourly_distribution;
    let wave_canvas = Canvas::default()
        .block(
            Block::default()
                .title(Span::styled(" ◐ 24h Circadian Rhythm ", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_idle)),
        )
        .marker(Marker::Braille)
        .x_bounds([0.0, 23.0])
        .y_bounds([0.0, hourly_max * 1.15])
        .paint(move |ctx| {
            let pts: Vec<(f64, f64)> = hourly
                .iter()
                .enumerate()
                .map(|(h, &v)| (h as f64, v as f64))
                .collect();
            for pair in pts.windows(2) {
                ctx.draw(&CanvasLine {
                    x1: pair[0].0,
                    y1: pair[0].1,
                    x2: pair[1].0,
                    y2: pair[1].1,
                    color: Color::Rgb(125, 207, 255),
                });
            }
            ctx.draw(&Points {
                coords: &pts,
                color: Color::Rgb(255, 118, 142),
            });
        });
    frame.render_widget(wave_canvas, mid_cols[0]);

    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(3)])
        .split(mid_cols[1]);

    let dur_max = a
        .duration_buckets
        .iter()
        .map(|(_, c, _)| *c)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let bucket_colors = [theme.accent, theme.secondary, theme.primary, Color::Rgb(251, 191, 36)];
    let mut bucket_lines = Vec::new();
    for (i, (label, cnt, hrs)) in a.duration_buckets.iter().enumerate() {
        let mut l = render_gradient_bar(label, *cnt as f64 / dur_max, 18, bucket_colors[i]);
        l.spans.push(Span::styled(
            format!(" {} ({:.1}h)", cnt, hrs),
            Style::default().fg(theme.text_dim),
        ));
        bucket_lines.push(l);
    }
    frame.render_widget(
        Paragraph::new(bucket_lines).block(
            Block::default()
                .title(Span::styled(" Format Runtime Mix ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_idle)),
        ),
        right_split[0],
    );

    // 182-day thermal heatmap rendered compactly into last 26 weeks
    let heat_blk = Block::default()
        .title(Span::styled(" ⬛ 26-Week Thermal Habit Grid ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let mut heat_rows = Vec::new();
    for dow in 0..7 {
        let mut spans = Vec::with_capacity(28);
        spans.push(Span::raw(" "));
        for wk in 0..26 {
            let idx = wk * 7 + dow;
            let count = a.thermal_activity.get(idx).copied().unwrap_or(0);
            let (ch, col) = match count {
                0 => ("· ", Color::Rgb(45, 50, 66)),
                1 => ("▪ ", Color::Rgb(56, 189, 248)),
                2..=3 => ("▪ ", Color::Rgb(163, 230, 53)),
                _ => ("█ ", Color::Rgb(251, 191, 36)),
            };
            spans.push(Span::styled(ch, Style::default().fg(col)));
        }
        heat_rows.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(heat_rows).block(heat_blk), right_split[1]);

    // 3. Bottom Row: Top Channels Leaderboard | Recent Log
    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[2]);

    let ch_lines: Vec<Line> = if a.top_channels.is_empty() {
        vec![Line::from(Span::styled(
            "Watch videos via mpv or feed history to populate top channels.",
            Style::default().fg(theme.text_dim),
        ))]
    } else {
        a.top_channels
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, c)| {
                Line::from(vec![
                    Span::styled(format!(" {}. ", i + 1), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<26} ", c.channel_name.chars().take(26).collect::<String>()), Style::default().fg(theme.text)),
                    Span::styled(format!("{} vids · {:.1}h", c.video_count, c.total_hours), Style::default().fg(theme.text_dim)),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(ch_lines).block(
            Block::default()
                .title(Span::styled(" Top Channels ", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_idle)),
        ),
        bot_cols[0],
    );

    let recent_lines: Vec<Line> = if a.recent_watch_log.is_empty() {
        vec![Line::from(Span::styled("No recent watch sessions logged yet.", Style::default().fg(theme.text_dim)))]
    } else {
        a.recent_watch_log
            .iter()
            .take(6)
            .map(|r| {
                Line::from(vec![
                    Span::styled(" • ", Style::default().fg(theme.primary)),
                    Span::styled(r.title.chars().take(28).collect::<String>(), Style::default().fg(theme.text)),
                    Span::styled(
                        format!(" ({})", r.channel_name.chars().take(12).collect::<String>()),
                        Style::default().fg(theme.text_dim),
                    ),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(recent_lines).block(
            Block::default()
                .title(Span::styled(" Recent Completions ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_idle)),
        ),
        bot_cols[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [r] Recompute SQLite Telemetry ", Style::default().fg(theme.primary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc / A] Close Dashboard ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_analytics_dashboard_renders_cleanly() {
        let mut st = AppState::new(Config::default());
        st.analytics.total_videos_watched = 42;
        st.analytics.total_hours_watched = 18.4;
        st.analytics.hourly_distribution[22] = 9;
        let backend = TestBackend::new(120, 36);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_analytics_modal(f, &st)).unwrap();
    }
}
