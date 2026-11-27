//! Native YouTube Music tab UI:
//! - Sub-navigation (`Home`, `Library`, `Liked`, `Charts`, `Search`) + Back breadcrumb
//! - Search bar & category filter chips (`All`, `Songs`, `Videos`, `Albums`, `Playlists`, `Artists`)
//! - Left browse pane (carousels/shelves & numbered album/playlist tracklists)
//! - Right Now Playing card (`1:1` Ambilight square album art, seek bar, transport & Like/Video toggles)
//! - Bottom-right live `Up Next` queue & auto-scrolling time-synced `Lyrics` pane

use crate::api::ytmusic::{YtmItemKind, YtmSearchFilter, YtmShelfItem, YtmTrack};
use crate::db::cache::format_duration_secs;
use crate::state::{
    AppState, MusicHit, ThumbPriority, YtmBrowseRow, YtmRepeatMode, YtmRightPaneTab, YtmSection,
};
use crate::ui::components::cover::render_thumbnail_cover;
use crate::ui::home::marquee_window;

fn rect_intersects_optional(r: Rect, modal: Option<Rect>) -> bool {
    modal.is_some_and(|m| {
        r.x < m.x + m.width && m.x < r.x + r.width && r.y < m.y + m.height && m.y < r.y + r.height
    })
}
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let prefix: String = s.chars().take(max_chars - 1).collect();
    format!("{prefix}…")
}

/// Queue square album cover (`ytm:<id>`) for `thumb_url` when missing from cache.
fn queue_music_cover(state: &mut AppState, key: &str, url: &str, priority: ThumbPriority) {
    if key.is_empty() || url.is_empty() {
        return;
    }
    let full_key = format!("ytm:{key}");
    state.queue_thumb_with_priority(&full_key, url, priority);
}

/// Render the full YouTube Music workspace below Phaenna's top tab bar.
pub fn render_music_workspace(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    state.music.hitboxes.clear();
    state.music.progress_bar_rect = Rect::default();

    if area.width < 32 || area.height < 10 {
        return;
    }

    let show_search_strip = state.music.section == YtmSection::Search;
    let show_browse_banner = !show_search_strip
        && state
            .music
            .current_page()
            .and_then(|p| p.header_title.as_ref())
            .is_some();

    let header_h = if show_search_strip {
        3
    } else if show_browse_banner {
        2
    } else {
        1
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_h), Constraint::Min(6)])
        .split(area);

    render_music_subnav(frame, chunks[0], state, show_search_strip, show_browse_banner);

    let body_area = chunks[1];
    if body_area.width >= 76 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(body_area);
        render_browse_pane(frame, cols[0], state);
        render_player_pane(frame, cols[1], state, modal_rect);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(body_area);
        render_browse_pane(frame, rows[0], state);
        render_player_pane(frame, rows[1], state, modal_rect);
    }
}

fn render_music_subnav(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    show_search_strip: bool,
    show_browse_banner: bool,
) {
    let theme = state.base_theme();
    let nav_rect = Rect::new(area.x, area.y, area.width, 1);

    let mut x = nav_rect.x + 1;
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    if !state.music.browse_stack.is_empty() || state.music.section == YtmSection::Browse {
        let back_lbl = " <- Back ";
        let w = back_lbl.chars().count() as u16;
        state
            .music
            .hitboxes
            .push((Rect::new(x, nav_rect.y, w, 1), MusicHit::GoBack));
        spans.push(Span::styled(
            back_lbl,
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        x += w + 1;
    }

    for sec in YtmSection::NAV_TABS {
        let active = state.music.section == sec;
        let lbl = format!(" {} ", sec.label());
        let w = lbl.chars().count() as u16;
        state
            .music
            .hitboxes
            .push((Rect::new(x, nav_rect.y, w, 1), MusicHit::SelectSection(sec)));
        let st = if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.text)
                .bg(Color::Rgb(28, 32, 44))
        };
        spans.push(Span::styled(lbl, st));
        spans.push(Span::raw(" "));
        x += w + 1;
    }

    if state.music.loading {
        let msg = if state.music.loading_label.is_empty() {
            " Loading… ".to_string()
        } else {
            format!(" {}… ", state.music.loading_label)
        };
        spans.push(Span::styled(
            msg,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    } else if state.auth_vault.active_account().is_none() {
        spans.push(Span::styled(
            " Guest mode ([a] Login for personalized mixes & Likes) ",
            Style::default().fg(theme.text_dim),
        ));
    } else {
        spans.push(Span::styled(
            " [/] Search · [Enter] Play · [Space] Pause · [v] Song/Video · [y] Lyrics ",
            Style::default().fg(theme.text_dim),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), nav_rect);

    if show_search_strip && area.height >= 3 {
        let query_rect = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1);
        state
            .music
            .hitboxes
            .push((query_rect, MusicHit::FocusSearch));
        let editing = state.music.search_editing;
        let cursor_glyph = if editing { "▏" } else { "" };
        let q_text = if state.music.search_query.is_empty() && !editing {
            "Search songs, albums, artists, playlists… (press / to type)".to_string()
        } else {
            format!("{}{cursor_glyph}", state.music.search_query)
        };
        let box_style = if editing {
            Style::default()
                .fg(theme.text)
                .bg(Color::Rgb(36, 44, 62))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(if state.music.search_query.is_empty() {
                    theme.text_dim
                } else {
                    theme.text
                })
                .bg(Color::Rgb(24, 28, 38))
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Search: ",
                    Style::default()
                        .fg(theme.primary)
                        .bg(Color::Rgb(24, 28, 38))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {q_text}"), box_style),
            ]))
            .style(box_style),
            query_rect,
        );

        let chips_rect = Rect::new(area.x + 1, area.y + 2, area.width.saturating_sub(2), 1);
        let mut cx = chips_rect.x;
        let mut chip_spans = Vec::new();
        for filter in YtmSearchFilter::ALL {
            let active = state.music.search_filter == filter;
            let lbl = format!(" {} ", filter.label());
            let w = lbl.chars().count() as u16;
            state.music.hitboxes.push((
                Rect::new(cx, chips_rect.y, w, 1),
                MusicHit::SelectSearchFilter(filter),
            ));
            let st = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.secondary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.text_dim)
                    .bg(Color::Rgb(22, 26, 36))
            };
            chip_spans.push(Span::styled(lbl, st));
            chip_spans.push(Span::raw(" "));
            cx += w + 1;
        }
        chip_spans.push(Span::styled(
            "  [Tab] Switch Filter  [Enter] Submit",
            Style::default().fg(theme.text_dim),
        ));
        frame.render_widget(Paragraph::new(Line::from(chip_spans)), chips_rect);
    } else if show_browse_banner && area.height >= 2 {
        if let Some(page) = state.music.current_page() {
            let banner_rect = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1);
            let title = page
                .header_title
                .as_deref()
                .unwrap_or(&state.music.browse_title);
            let sub = page.header_subtitle.as_deref().unwrap_or("");
            let line = if sub.is_empty() {
                Line::from(vec![Span::styled(
                    format!(" {title} "),
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                )])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(" {title} "),
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" · {sub}"), Style::default().fg(theme.text_dim)),
                ])
            };
            frame.render_widget(Paragraph::new(line), banner_rect);
        }
    }
}

fn render_browse_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme().clone();
    let title = match state.music.section {
        YtmSection::Browse if !state.music.browse_title.is_empty() => {
            format!(" {} ", state.music.browse_title)
        }
        s => format!(" YouTube Music · {} ", s.label()),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.music.search_editing {
            theme.border_idle
        } else {
            theme.primary
        }))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 12 || inner.height < 3 {
        return;
    }

    if let Some(ref err) = state.music.error {
        let msg = Paragraph::new(format!("Unable to load YouTube Music:\n{err}\n\nPress [r] to retry."))
            .style(Style::default().fg(Color::Rgb(245, 110, 110)))
            .wrap(Wrap { trim: true });
        frame.render_widget(msg, inner);
        return;
    }

    let (rows, items) = state.music.build_rows();
    if rows.is_empty() {
        let empty_msg = if state.music.loading {
            "Fetching YouTube Music catalog…".to_string()
        } else if state.music.section == YtmSection::Search {
            "Type a query and press Enter to search songs, albums, artists & playlists.".to_string()
        } else {
            "Nothing loaded yet — press [r] to refresh.".to_string()
        };
        frame.render_widget(
            Paragraph::new(empty_msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            Rect::new(
                inner.x,
                inner.y + inner.height / 3,
                inner.width,
                inner.height.saturating_sub(inner.height / 3),
            ),
        );
        return;
    }

    if !items.is_empty() && state.music.cursor >= items.len() {
        state.music.cursor = items.len() - 1;
    }

    // Locate display row corresponding to cursor so viewport always keeps cursor visible.
    let cursor_row_idx = rows
        .iter()
        .position(|r| matches!(r, YtmBrowseRow::Item { selectable_idx, .. } if *selectable_idx == state.music.cursor))
        .unwrap_or(0);

    let max_vis = inner.height as usize;
    if cursor_row_idx < state.music.scroll {
        state.music.scroll = cursor_row_idx.saturating_sub(1);
    } else if cursor_row_idx >= state.music.scroll + max_vis {
        state.music.scroll = cursor_row_idx + 1 - max_vis;
    }
    let start = state.music.scroll.min(rows.len().saturating_sub(1));

    // Prefetch square thumbnails near the viewport.
    for row in rows.iter().skip(start).take(max_vis + 4) {
        if let YtmBrowseRow::Item { item, selectable_idx, .. } = row {
            let key = item.thumb_cache_key();
            if !key.is_empty() {
                let prio = if *selectable_idx == state.music.cursor {
                    ThumbPriority::Focused
                } else {
                    ThumbPriority::Visible
                };
                queue_music_cover(state, &key, &item.thumb_url, prio);
            }
        }
    }

    let active_vid = state.active_playback_video_id.clone();
    let liked_ids = state.music.liked_ids.clone();

    for (vis_i, row) in rows.iter().skip(start).take(max_vis).enumerate() {
        let row_rect = Rect::new(inner.x, inner.y + vis_i as u16, inner.width, 1);
        match row {
            YtmBrowseRow::ShelfHeader { title } => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!(" ▸ {title} "),
                            Style::default()
                                .fg(theme.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]))
                    .style(Style::default().bg(Color::Rgb(16, 20, 30))),
                    row_rect,
                );
            }
            YtmBrowseRow::Item {
                item,
                selectable_idx,
                track_num,
            } => {
                let is_sel = *selectable_idx == state.music.cursor;
                let is_playing = item
                    .video_id
                    .as_deref()
                    .is_some_and(|vid| active_vid.as_deref() == Some(vid));
                let is_liked = item
                    .video_id
                    .as_deref()
                    .is_some_and(|vid| liked_ids.contains(vid));

                state
                    .music
                    .hitboxes
                    .push((row_rect, MusicHit::SelectItem(*selectable_idx)));

                let bg = if is_sel {
                    Color::Rgb(36, 46, 68)
                } else if is_playing {
                    Color::Rgb(20, 36, 30)
                } else {
                    Color::Reset
                };

                let lead = if is_playing {
                    "▶ ".to_string()
                } else if let Some(n) = track_num {
                    format!("{n:>2}.")
                } else if is_sel {
                    "› ".to_string()
                } else {
                    "  ".to_string()
                };

                let badge_col = match item.kind {
                    YtmItemKind::Song => theme.secondary,
                    YtmItemKind::Video => Color::Rgb(220, 130, 255),
                    YtmItemKind::Album => theme.primary,
                    YtmItemKind::Playlist | YtmItemKind::Radio => Color::Rgb(100, 195, 255),
                    YtmItemKind::Artist => Color::Rgb(255, 190, 90),
                };

                let like_tag = if is_liked { " ♥" } else { "" };
                let dur_tag = item
                    .duration_secs
                    .map(|d| format!(" {}", format_duration_secs(d)))
                    .unwrap_or_default();

                let reserved = 10 + like_tag.chars().count() + dur_tag.chars().count();
                let avail = (inner.width as usize).saturating_sub(reserved);
                let title_budget = (avail * 54) / 100;
                let sub_budget = avail.saturating_sub(title_budget + 3);

                let t_str = truncate_str(&item.title, title_budget.max(8));
                let s_str = if sub_budget >= 6 && !item.subtitle.is_empty() {
                    format!(" · {}", truncate_str(&item.subtitle, sub_budget))
                } else {
                    String::new()
                };

                let spans = vec![
                    Span::styled(
                        format!("{lead:<3}"),
                        Style::default()
                            .fg(if is_playing {
                                theme.secondary
                            } else {
                                theme.text_dim
                            })
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("[{}] ", item.kind.badge()),
                        Style::default()
                            .fg(badge_col)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        t_str,
                        Style::default()
                            .fg(if is_playing {
                                theme.secondary
                            } else if is_sel {
                                Color::White
                            } else {
                                theme.text
                            })
                            .bg(bg)
                            .add_modifier(if is_sel || is_playing {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(s_str, Style::default().fg(theme.text_dim).bg(bg)),
                    Span::styled(
                        like_tag,
                        Style::default().fg(Color::Rgb(255, 100, 140)).bg(bg),
                    ),
                    Span::styled(dur_tag, Style::default().fg(theme.text_dim).bg(bg)),
                ];

                frame.render_widget(
                    Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
                    row_rect,
                );
            }
        }
    }
}

fn render_player_pane(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let now_h = if area.height >= 24 {
        14
    } else {
        (area.height / 2).max(10).min(area.height)
    };
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(now_h), Constraint::Min(6)])
        .split(area);

    render_now_playing_card(frame, panes[0], state, modal_rect);
    render_queue_or_lyrics_pane(frame, panes[1], state);
}

fn render_now_playing_card(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let items = state.music.selectable_items();
    let preview_item: Option<YtmShelfItem> = items.get(state.music.cursor).cloned();
    let cur_track: Option<YtmTrack> = state.music.current_track().cloned();

    // Determine art key for dynamic Ambilight border + album cover.
    let (art_id, art_url, title, artist, album_opt, active_vid) = if let Some(ref tr) = cur_track {
        (
            format!("ytm:{}", tr.thumb_cache_key()),
            tr.thumb_url.clone(),
            tr.title.clone(),
            tr.artist.clone(),
            tr.album.clone(),
            Some(tr.video_id.clone()),
        )
    } else if let Some(ref p) = preview_item {
        let raw_k = p.thumb_cache_key();
        let k = if raw_k.is_empty() {
            "preview".to_string()
        } else {
            raw_k
        };
        (
            format!("ytm:{k}"),
            p.thumb_url.clone(),
            p.title.clone(),
            p.subtitle.clone(),
            None,
            p.video_id.clone(),
        )
    } else {
        (
            String::new(),
            String::new(),
            "No track selected".to_string(),
            "Pick any song or playlist to start".to_string(),
            None,
            None,
        )
    };

    if !art_id.is_empty() && !art_url.is_empty() {
        state.queue_thumb_with_priority(&art_id, &art_url, ThumbPriority::Focused);
    }

    let pal = if !art_id.is_empty() {
        state.active_palette_for(Some(&art_id)).clone()
    } else {
        state.base_theme().clone()
    };

    let mode_label = if state.music.video_mode {
        " VIDEO "
    } else {
        " AUDIO "
    };
    let status_tag = if cur_track.is_some() && state.has_live_ipc() {
        if state.live_playback.paused {
            "PAUSED"
        } else {
            "PLAYING"
        }
    } else if cur_track.is_some() {
        "LOADING"
    } else {
        "PREVIEW"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary))
        .title(Line::from(vec![
            Span::styled(
                format!(" Now Playing · {status_tag} "),
                Style::default()
                    .fg(pal.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                mode_label,
                Style::default()
                    .fg(Color::Black)
                    .bg(if state.music.video_mode {
                        Color::Rgb(210, 120, 255)
                    } else {
                        pal.secondary
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 16 || inner.height < 6 {
        return;
    }

    // Reserve square-looking aspect area on left (1:1 visual ≈ cols = rows * 2).
    let cover_w = ((inner.height.saturating_sub(2)) * 2)
        .clamp(14, inner.width.saturating_sub(24).max(14));
    let hsplit = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(cover_w), Constraint::Min(16)])
        .split(inner);

    let cover_rect = Rect::new(
        hsplit[0].x,
        hsplit[0].y,
        hsplit[0].width.saturating_sub(1),
        hsplit[0].height,
    );
    if !art_id.is_empty() && !rect_intersects_optional(cover_rect, modal_rect) {
        render_thumbnail_cover(frame, cover_rect, &art_id, state, "♪ Cover");
    } else {
        frame.render_widget(
            Paragraph::new("\n♪\nYTM")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(pal.primary)
                        .bg(Color::Rgb(20, 24, 34))
                        .add_modifier(Modifier::BOLD),
                ),
            cover_rect,
        );
    }

    let info_rect = hsplit[1];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Title
            Constraint::Length(1), // Artist / Album
            Constraint::Length(1), // Progress text
            Constraint::Length(1), // Progress bar
            Constraint::Length(1), // Transport buttons
            Constraint::Min(0),    // Extra toggles
        ])
        .split(info_rect);

    let w = info_rect.width as usize;
    let marq_title = marquee_window(&title, w.max(8), state.ticker_step);
    frame.render_widget(
        Paragraph::new(marq_title)
            .style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    let meta = if let Some(alb) = album_opt {
        format!("{artist} — {alb}")
    } else {
        artist
    };
    frame.render_widget(
        Paragraph::new(truncate_str(&meta, w.max(8))).style(Style::default().fg(pal.text_dim)),
        rows[1],
    );

    let dur_secs = if state.active_playback_dur > 0.0 {
        state.active_playback_dur
    } else {
        cur_track
            .as_ref()
            .and_then(|t| t.duration_secs)
            .map(|d| d as f64)
            .unwrap_or(0.0)
    };
    let pos_secs = if cur_track.is_some() {
        state.active_playback_pos.clamp(0.0, dur_secs.max(state.active_playback_pos))
    } else {
        0.0
    };

    let vol_text = if state.live_playback.muted {
        "MUTE".to_string()
    } else {
        format!("Vol:{}%", state.live_playback.volume)
    };
    let liked = active_vid
        .as_deref()
        .is_some_and(|v| state.music.liked_ids.contains(v));
    let time_line = format!(
        "{} / {}   ·   {}",
        format_duration_secs(pos_secs as u64),
        format_duration_secs(dur_secs as u64),
        vol_text
    );
    frame.render_widget(
        Paragraph::new(truncate_str(&time_line, w)).style(Style::default().fg(pal.accent)),
        rows[2],
    );

    // Interactive seek progress bar
    let bar_w = rows[3].width as usize;
    if bar_w >= 6 {
        let ratio = if dur_secs > 0.0 {
            (pos_secs / dur_secs).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled = ((bar_w.saturating_sub(1)) as f64 * ratio).round() as usize;
        let rem = bar_w.saturating_sub(filled + 1);
        let bar_str = format!("{}{}{}", "━".repeat(filled), "●", "─".repeat(rem));
        state.music.progress_bar_rect = rows[3];
        state.music.hitboxes.push((rows[3], MusicHit::SeekBar));
        frame.render_widget(
            Paragraph::new(bar_str).style(Style::default().fg(pal.primary)),
            rows[3],
        );
    }

    // Transport buttons row
    render_transport_buttons(frame, rows[4], state, liked, &pal);

    // Extra info row
    if rows[5].height > 0 {
        let shuf_lbl = if state.music.shuffle { "ON" } else { "OFF" };
        let rep_lbl = state.music.repeat.label();
        let mode_hint = if state.music.video_mode {
            "Video window"
        } else {
            "Windowless Audio"
        };
        let hint = format!(
            "[>] Next  [<] Prev  [z] Shuf:{shuf_lbl}  [R] Rep:{rep_lbl}  [v] {mode_hint}"
        );
        frame.render_widget(
            Paragraph::new(truncate_str(&hint, w)).style(Style::default().fg(pal.text_dim)),
            rows[5],
        );
    }
}

fn render_transport_buttons(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    liked: bool,
    pal: &crate::ui::theme::ThemePalette,
) {
    let mut bx = area.x;
    let buttons: Vec<(&str, MusicHit, Color, Color)> = vec![
        (" |< ", MusicHit::PrevTrack, pal.text, Color::Rgb(30, 34, 48)),
        (
            if state.live_playback.paused {
                " ▶ "
            } else {
                " ⏸ "
            },
            MusicHit::TogglePlayPause,
            Color::Black,
            pal.primary,
        ),
        (" >| ", MusicHit::NextTrack, pal.text, Color::Rgb(30, 34, 48)),
        (
            if state.music.shuffle {
                " Shuf:ON "
            } else {
                " Shuf "
            },
            MusicHit::ToggleShuffle,
            if state.music.shuffle {
                Color::Black
            } else {
                pal.text_dim
            },
            if state.music.shuffle {
                pal.secondary
            } else {
                Color::Rgb(26, 30, 40)
            },
        ),
        (
            match state.music.repeat {
                YtmRepeatMode::Off => " Rep:Off ",
                YtmRepeatMode::All => " Rep:All ",
                YtmRepeatMode::One => " Rep:1 ",
            },
            MusicHit::CycleRepeat,
            if state.music.repeat != YtmRepeatMode::Off {
                Color::Black
            } else {
                pal.text_dim
            },
            if state.music.repeat != YtmRepeatMode::Off {
                pal.accent
            } else {
                Color::Rgb(26, 30, 40)
            },
        ),
        (
            if liked { " ♥ " } else { " ♡ " },
            MusicHit::ToggleLike,
            if liked {
                Color::Rgb(255, 90, 130)
            } else {
                pal.text_dim
            },
            Color::Rgb(26, 30, 40),
        ),
        (
            if state.music.video_mode {
                " Vid "
            } else {
                " Aud "
            },
            MusicHit::ToggleVideoMode,
            Color::Black,
            if state.music.video_mode {
                Color::Rgb(210, 120, 255)
            } else {
                pal.secondary
            },
        ),
    ];

    let mut spans = Vec::new();
    for (lbl, hit, fg, bg) in buttons {
        let w = lbl.chars().count() as u16;
        if bx + w <= area.x + area.width {
            state
                .music
                .hitboxes
                .push((Rect::new(bx, area.y, w, 1), hit));
            spans.push(Span::styled(
                lbl,
                Style::default()
                    .fg(fg)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            bx += w + 1;
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_queue_or_lyrics_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme().clone();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 12 || inner.height < 3 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(inner);

    // Tab switcher header
    let tab_bar = chunks[0];
    let up_lbl = format!(" Up Next ({}) ", state.music.queue.len());
    let lyr_lbl = if state
        .music
        .lyrics
        .as_ref()
        .is_some_and(|l| l.is_timed)
    {
        " Lyrics (Synced) ".to_string()
    } else {
        " Lyrics ".to_string()
    };

    let up_w = up_lbl.chars().count() as u16;
    let lyr_x = tab_bar.x + 1 + up_w + 1;
    let lyr_w = lyr_lbl.chars().count() as u16;

    state.music.hitboxes.push((
        Rect::new(tab_bar.x + 1, tab_bar.y, up_w, 1),
        MusicHit::SelectRightTab(YtmRightPaneTab::UpNext),
    ));
    state.music.hitboxes.push((
        Rect::new(lyr_x, tab_bar.y, lyr_w, 1),
        MusicHit::SelectRightTab(YtmRightPaneTab::Lyrics),
    ));

    let up_st = if state.music.right_tab == YtmRightPaneTab::UpNext {
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.text_dim)
            .bg(Color::Rgb(24, 28, 38))
    };
    let lyr_st = if state.music.right_tab == YtmRightPaneTab::Lyrics {
        Style::default()
            .fg(Color::Black)
            .bg(theme.secondary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.text_dim)
            .bg(Color::Rgb(24, 28, 38))
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(up_lbl, up_st),
            Span::raw(" "),
            Span::styled(lyr_lbl, lyr_st),
            Span::styled("   [y] Toggle", Style::default().fg(theme.text_dim)),
        ])),
        tab_bar,
    );

    let body = chunks[1];
    match state.music.right_tab {
        YtmRightPaneTab::UpNext => render_up_next_list(frame, body, state, &theme),
        YtmRightPaneTab::Lyrics => render_lyrics_view(frame, body, state, &theme),
    }
}

fn render_up_next_list(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &crate::ui::theme::ThemePalette,
) {
    if state.music.queue.is_empty() {
        frame.render_widget(
            Paragraph::new("\nQueue is empty — play any track to build an automatic mix.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            area,
        );
        return;
    }

    let max_vis = area.height as usize;
    let total = state.music.queue.len();
    let cur = state.music.queue_index.min(total - 1);
    let scroll = if cur >= max_vis / 2 && total > max_vis {
        (cur - max_vis / 2).min(total - max_vis)
    } else {
        0
    };

    for (vi, (q_idx, tr)) in state
        .music
        .queue
        .iter()
        .enumerate()
        .skip(scroll)
        .take(max_vis)
        .enumerate()
    {
        let r = Rect::new(area.x, area.y + vi as u16, area.width, 1);
        state
            .music
            .hitboxes
            .push((r, MusicHit::SelectQueueTrack(q_idx)));

        let is_cur = q_idx == cur;
        let bg = if is_sel_q(is_cur) {
            Color::Rgb(24, 42, 34)
        } else {
            Color::Reset
        };
        let lead = if is_cur {
            "▶ ".to_string()
        } else {
            format!("{:>2}.", q_idx + 1)
        };
        let dur = tr
            .duration_text
            .clone()
            .or_else(|| tr.duration_secs.map(format_duration_secs))
            .unwrap_or_default();
        let avail = (area.width as usize).saturating_sub(dur.chars().count() + 7);
        let t_w = (avail * 56) / 100;
        let a_w = avail.saturating_sub(t_w + 2);
        let title_str = truncate_str(&tr.title, t_w.max(6));
        let art_str = if a_w >= 4 && !tr.artist.is_empty() {
            format!(" · {}", truncate_str(&tr.artist, a_w))
        } else {
            String::new()
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {lead:<3} "),
                    Style::default()
                        .fg(if is_cur {
                            theme.secondary
                        } else {
                            theme.text_dim
                        })
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    title_str,
                    Style::default()
                        .fg(if is_cur { theme.secondary } else { theme.text })
                        .bg(bg)
                        .add_modifier(if is_cur {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(art_str, Style::default().fg(theme.text_dim).bg(bg)),
                Span::styled(
                    if dur.is_empty() {
                        String::new()
                    } else {
                        format!(" {dur}")
                    },
                    Style::default().fg(theme.text_dim).bg(bg),
                ),
            ]))
            .style(Style::default().bg(bg)),
            r,
        );
    }
}

fn is_sel_q(active: bool) -> bool {
    active
}

fn render_lyrics_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &crate::ui::theme::ThemePalette,
) {
    if state.music.lyrics_loading {
        frame.render_widget(
            Paragraph::new("\nLoading lyrics…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accent)),
            area,
        );
        return;
    }

    let Some(ref lyrics) = state.music.lyrics else {
        let msg = if state.music.lyrics_browse_id.is_none() && state.music.current_track().is_some() {
            "\nLyrics are not available for this track."
        } else {
            "\nPlay a song to view live synced lyrics."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            area,
        );
        return;
    };

    if lyrics.lines.is_empty() {
        frame.render_widget(
            Paragraph::new("\nInstrumental / no lyrics returned.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            area,
        );
        return;
    }

    let max_vis = area.height.saturating_sub(1) as usize;
    let active_idx = state.music.active_lyric_index(state.active_playback_pos);
    let scroll = if let Some(act) = active_idx {
        if act >= max_vis / 2 && lyrics.lines.len() > max_vis {
            (act - max_vis / 2).min(lyrics.lines.len() - max_vis)
        } else {
            0
        }
    } else {
        0
    };

    let mut rendered_lines: Vec<Line> = Vec::new();
    for (idx, line) in lyrics.lines.iter().enumerate().skip(scroll).take(max_vis) {
        let is_active = active_idx == Some(idx);
        let is_past = active_idx.is_some_and(|a| idx < a);
        let st = if is_active {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if is_past {
            Style::default().fg(theme.text_dim)
        } else {
            Style::default().fg(theme.text)
        };
        let prefix = if is_active { "♪ " } else { "  " };
        rendered_lines.push(Line::from(Span::styled(
            format!("{prefix}{}", line.text),
            st,
        )));
    }

    if let Some(ref src) = lyrics.source {
        rendered_lines.push(Line::from(Span::styled(
            format!("  {src}"),
            Style::default().fg(theme.text_dim),
        )));
    }

    frame.render_widget(
        Paragraph::new(rendered_lines)
            .wrap(Wrap { trim: true }),
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), area.height),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ytmusic::{YtmLyricLine, YtmLyrics, YtmPage, YtmShelf};
    use crate::config::Config;
    use crate::state::Category;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_music_workspace_renders_shelves_player_and_synced_lyrics() {
        let mut st = AppState::new(Config::default());
        st.active_category = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Music)
            .unwrap();
        st.music.home_page = Some(YtmPage {
            header_title: None,
            header_subtitle: None,
            header_thumb: None,
            header_playlist_id: None,
            shelves: vec![YtmShelf {
                title: "Listen again".to_string(),
                items: vec![YtmShelfItem {
                    kind: YtmItemKind::Song,
                    title: "Starlight".to_string(),
                    subtitle: "Muse • Black Holes and Revelations".to_string(),
                    video_id: Some("Pgum6OT_VH8".to_string()),
                    browse_id: None,
                    playlist_id: Some("RDAMVMPgum6OT_VH8".to_string()),
                    params: None,
                    thumb_url: String::new(),
                    duration_secs: Some(240),
                }],
            }],
            tracks: Vec::new(),
            continuation: None,
        });
        st.music.queue = vec![YtmTrack {
            video_id: "Pgum6OT_VH8".to_string(),
            title: "Starlight".to_string(),
            artist: "Muse".to_string(),
            artist_browse_id: None,
            album: Some("Black Holes".to_string()),
            album_browse_id: None,
            duration_text: Some("4:00".to_string()),
            duration_secs: Some(240),
            plays_text: None,
            thumb_url: String::new(),
            playlist_id: None,
            kind: YtmItemKind::Song,
        }];
        st.music.right_tab = YtmRightPaneTab::Lyrics;
        st.music.lyrics = Some(YtmLyrics {
            lines: vec![
                YtmLyricLine {
                    text: "Far away".to_string(),
                    start_ms: Some(1_000),
                    end_ms: Some(5_000),
                },
                YtmLyricLine {
                    text: "Hold you in my arms".to_string(),
                    start_ms: Some(10_000),
                    end_ms: Some(15_000),
                },
            ],
            source: Some("Source: Musixmatch".to_string()),
            is_timed: true,
        });
        st.active_playback_pos = 12.0;
        st.active_playback_dur = 240.0;

        let backend = TestBackend::new(130, 38);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| crate::ui::home::render_home_screen(f, &mut st, None))
            .unwrap();

        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Listen again"));
        assert!(text.contains("Starlight"));
        assert!(text.contains("Lyrics (Synced)"));
        assert!(text.contains("Hold you in my arms"));
        assert!(!st.music.hitboxes.is_empty());
        assert_eq!(st.music.active_lyric_index(12.0), Some(1));
    }
}
