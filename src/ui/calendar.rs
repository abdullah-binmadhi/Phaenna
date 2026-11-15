//! 7-Day Subscription Upload Radar Calendar (`c`).

use crate::services::calendar::bucket_videos_by_week;
use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use std::collections::HashSet;

pub fn render_calendar_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(90, 84, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            format!(
                " 📅 UPLOAD RADAR — {} ({}) ",
                state.calendar_bounds.range_label,
                state.calendar_filter.label()
            ),
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
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

    let subs_only = state
        .categories
        .iter()
        .find(|(c, _)| *c == crate::state::Category::Subscriptions)
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[]);
    let fallback_pool;
    let pool = if subs_only.is_empty() {
        fallback_pool = state.all_known_videos();
        fallback_pool.as_slice()
    } else {
        subs_only
    };

    let wl_ids: HashSet<String> = state.watch_later_sources.keys().cloned().collect();
    let buckets = bucket_videos_by_week(
        pool,
        &state.calendar_bounds,
        state.calendar_tz_offset,
        &state.watched_ids,
        &wl_ids,
        state.calendar_filter,
    );

    // Day selector header strip
    let mut tab_spans = Vec::new();
    for meta in &state.calendar_bounds.days {
        let count = buckets.get(&meta.day_idx).map(|v| v.len()).unwrap_or(0);
        let is_cur = meta.day_idx == state.calendar_day_cursor;
        let is_today = state.calendar_bounds.today_day_idx == Some(meta.day_idx);
        let label = format!(
            " {}{} ({}) ",
            meta.short_name,
            if is_today { "*" } else { "" },
            count
        );
        let st = if is_cur {
            Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
        } else if is_today {
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        tab_spans.push(Span::styled(label, st));
        tab_spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(tab_spans)).alignment(Alignment::Center), rows[0]);

    let active_day_entries = buckets.get(&state.calendar_day_cursor).map(|v| v.as_slice()).unwrap_or(&[]);
    let cur_meta = state.calendar_bounds.days.get(state.calendar_day_cursor);
    let day_title = cur_meta
        .map(|m| format!(" {} — {} ", m.full_name, m.date_short))
        .unwrap_or_else(|| " Day ".to_string());

    let list_blk = Block::default()
        .title(Span::styled(day_title, Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let list_inner = list_blk.inner(rows[1]);
    frame.render_widget(list_blk, rows[1]);

    if active_day_entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No recorded uploads on this day for the current filter.\nPress [h/l] to browse Mon-Sun or [[] / []] to shift weeks.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            list_inner,
        );
    } else {
        let mut lines = Vec::new();
        let max_vis = list_inner.height as usize;
        let cur = state.calendar_item_cursor.min(active_day_entries.len().saturating_sub(1));
        let start = if cur >= max_vis && max_vis > 0 { cur + 1 - max_vis } else { 0 };
        for (i, entry) in active_day_entries.iter().enumerate().skip(start).take(max_vis) {
            let sel = i == cur;
            let bg = if sel { Color::Rgb(34, 42, 58) } else { Color::Reset };
            let badge = if entry.is_watched {
                Span::styled("✓ ", Style::default().fg(theme.accent).bg(bg))
            } else if entry.is_watch_later {
                Span::styled("★ ", Style::default().fg(Color::Rgb(251, 191, 36)).bg(bg))
            } else {
                Span::styled("• ", Style::default().fg(theme.text_dim).bg(bg))
            };
            lines.push(Line::from(vec![
                Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(theme.primary).bg(bg)),
                Span::styled(format!("{} ", entry.local_hhmm), Style::default().fg(theme.secondary).bg(bg)),
                badge,
                Span::styled(
                    format!("{:<46} ", entry.video.title.chars().take(46).collect::<String>()),
                    Style::default().fg(if sel { theme.primary } else { theme.text }).bg(bg),
                ),
                Span::styled(
                    entry.video.display_uploader().chars().take(20).collect::<String>(),
                    Style::default().fg(theme.text_dim).bg(bg),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), list_inner);
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[h/l] Day ", Style::default().fg(theme.primary)),
            Span::styled("· [[ / ]] Week ", Style::default().fg(theme.secondary)),
            Span::styled("· [f] Filter ", Style::default().fg(theme.accent)),
            Span::styled("· [w] WatchLater ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("· [Enter] Detail · [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
