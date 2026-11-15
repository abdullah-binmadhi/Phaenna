//! 7-Day Subscription Upload Radar Calendar (`c`) — Split-Pane Timeline + Live 16:9 Preview.

use crate::services::calendar::{bucket_videos_by_week, CalendarDayEntry};
use crate::state::{centered_rect, AppState};
use crate::ui::components::cover::{render_video_preview_inspector, unix_now_secs};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use std::collections::HashSet;

pub fn render_calendar_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = state.base_theme();
    let area = centered_rect(90, 86, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            format!(
                " UPLOAD RADAR — {} ({}) ",
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
            Constraint::Min(12),
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
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if is_today {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        tab_spans.push(Span::styled(label, st));
        tab_spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(tab_spans)).alignment(Alignment::Center),
        rows[0],
    );

    let active_day_entries: Vec<CalendarDayEntry> = buckets
        .get(&state.calendar_day_cursor)
        .cloned()
        .unwrap_or_default();
    let cur_meta = state.calendar_bounds.days.get(state.calendar_day_cursor).cloned();
    let day_title = cur_meta
        .as_ref()
        .map(|m| format!(" {} — {} ({} uploads) ", m.full_name, m.date_short, active_day_entries.len()))
        .unwrap_or_else(|| " Day ".to_string());

    // 58% Left Chronological Radar List / 42% Right Live 16:9 Preview Inspector
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[1]);

    let list_blk = Block::default()
        .title(Span::styled(
            day_title,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let list_inner = list_blk.inner(panes[0]);
    frame.render_widget(list_blk, panes[0]);

    if active_day_entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No recorded uploads on this day for the current filter.\nPress [h/l] to browse Mon-Sun or [[ / ]] to shift weeks.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            list_inner,
        );

        let empty_right = Block::default()
            .title(Span::styled(
                " LIVE PREVIEW ",
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        let empty_inner = empty_right.inner(panes[1]);
        frame.render_widget(empty_right, panes[1]);
        frame.render_widget(
            Paragraph::new("Select a day with uploads to preview 16:9 thumbnail, views, release time, and quick playback.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            empty_inner,
        );
    } else {
        let max_vis = list_inner.height.max(1) as usize;
        let cur = state
            .calendar_item_cursor
            .min(active_day_entries.len().saturating_sub(1));
        let start = if cur >= max_vis { cur + 1 - max_vis } else { 0 };

        // Queue visible thumbnails asynchronously so switching rows shows instant art
        for entry in active_day_entries.iter().skip(start).take(max_vis + 2) {
            state.queue_thumb_if_needed(&entry.video.id, &entry.video.best_thumbnail_url());
        }

        let total_w = list_inner.width as usize;
        let uploader_w = (total_w / 4).clamp(10, 18);
        let prefix_w = 14usize; // " ▸ " (3) + "HH:MM " (6) + "[OK] " (5)
        let title_w = total_w.saturating_sub(prefix_w + uploader_w + 2).max(12);

        let mut lines = Vec::new();
        for (i, entry) in active_day_entries
            .iter()
            .enumerate()
            .skip(start)
            .take(max_vis)
        {
            let sel = i == cur;
            let bg = if sel { Color::Rgb(34, 42, 58) } else { Color::Reset };
            let badge = if entry.is_watched {
                Span::styled("[OK] ", Style::default().fg(theme.accent).bg(bg))
            } else if entry.is_watch_later {
                Span::styled("[WL] ", Style::default().fg(Color::Rgb(251, 191, 36)).bg(bg))
            } else {
                Span::styled("  ·  ", Style::default().fg(theme.text_dim).bg(bg))
            };
            let title_text = if sel {
                marquee_window(&entry.video.title, title_w, state.ticker_step)
            } else {
                entry.video.title.chars().take(title_w).collect::<String>()
            };
            let uploader_text: String = entry
                .video
                .display_uploader()
                .chars()
                .take(uploader_w)
                .collect();

            lines.push(Line::from(vec![
                Span::styled(
                    if sel { " ▸ " } else { "   " },
                    Style::default().fg(theme.primary).bg(bg),
                ),
                Span::styled(
                    format!("{} ", entry.local_hhmm),
                    Style::default().fg(theme.secondary).bg(bg),
                ),
                badge,
                Span::styled(
                    format!("{:<width$} ", title_text, width = title_w),
                    Style::default()
                        .fg(if sel { theme.primary } else { theme.text })
                        .bg(bg)
                        .add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(
                    format!("{:<width$}", uploader_text, width = uploader_w),
                    Style::default().fg(theme.text_dim).bg(bg),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), list_inner);

        if let Some(selected) = active_day_entries.get(cur) {
            let mut badges: Vec<(&str, Color)> = Vec::new();
            if selected.is_watched {
                badges.push(("WATCHED", theme.accent));
            }
            if selected.is_watch_later {
                badges.push(("WL", Color::Rgb(251, 191, 36)));
            }
            let day_label = cur_meta
                .as_ref()
                .map(|m| m.short_name)
                .unwrap_or("Day");
            let rel_extra = format!("Upload {} {}", selected.local_hhmm, day_label);
            let now_unix = unix_now_secs();
            render_video_preview_inspector(
                frame,
                panes[1],
                state,
                &selected.video,
                now_unix,
                &badges,
                Some(&rel_extra),
                "[Space] Play · [Enter] Detail · [a] +CW · [w] WL",
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[h/l] Day ", Style::default().fg(theme.primary)),
            Span::styled("· [[/]] Week ", Style::default().fg(theme.secondary)),
            Span::styled("· [f] Filter ", Style::default().fg(theme.accent)),
            Span::styled("· [Space] Play ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled("· [a] +Continue Watching ", Style::default().fg(theme.secondary)),
            Span::styled("· [w] WL ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("· [Enter] Detail · [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::cache::VideoItem;
    use crate::services::calendar::compute_week_bounds;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_calendar_split_pane_renders_cleanly_without_emoji() {
        let now = unix_now_secs();
        let mut state = AppState::new(Config::default());
        state.calendar_bounds = compute_week_bounds(now, 0, 0);
        if let Some(d) = state.calendar_bounds.today_day_idx {
            state.calendar_day_cursor = d;
        }
        let vid = VideoItem {
            id: "caltest001".to_string(),
            title: "Long Engineering Video Title With Extra Detail For Marquee Scroll".to_string(),
            uploader: Some("Rust Foundry".to_string()),
            duration: Some(930),
            view_count: Some(125_000),
            timestamp: Some(now),
            description: Some("Chronological release radar test description.".to_string()),
            ..Default::default()
        };
        if let Some((_, list)) = state.categories.first_mut() {
            list.push(vid);
        }

        let backend = TestBackend::new(130, 36);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_calendar_modal(f, &mut state)).unwrap();

        let buf_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buf_text.contains("UPLOAD RADAR"));
        assert!(buf_text.contains("LIVE PREVIEW"));
    }
}
