//! Spotify-style YouTube Music workspace UI:
//! - 4-Zone Desktop Layout:
//!   1. Left Sidebar (`Home`, `Search`, `Charts`, `Liked`, `Library`, `Music Settings`, & quick Library list)
//!   2. Main Stage (`1:1` square cover card shelves grid, Search hero + filter bar, or compact Track table)
//!   3. Collapsible Right Drawer (`d` key / bottom button / Music Settings default: large `1:1` art, `Up Next` queue & live synced `Lyrics`)
//!   4. Full-width Bottom Player Bar (mini `1:1` cover, clickable album/artist -> opens Album Popup, centered transport + seek bar, volume & drawer/video toggles)
//! - Centered Spotify Album / Playlist / Artist Detail Popup (`ActiveOverlay::MusicAlbum`)
//! - Dedicated YouTube Music Audio & UI Settings Modal (`ActiveOverlay::MusicSettings`)

use crate::api::ytmusic::{YtmItemKind, YtmPage, YtmSearchFilter, YtmShelfItem};
use crate::config::MusicSettingsConfig;
use crate::db::cache::format_duration_secs;
use crate::state::{
    AppState, MusicHit, MusicSettingsHit, ThumbPriority, YtmAlbumPopupHit, YtmRepeatMode,
    YtmRightPaneTab, YtmSection,
};
use crate::ui::components::cover::render_thumbnail_cover;
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

fn rect_intersects_optional(r: Rect, modal: Option<Rect>) -> bool {
    modal.is_some_and(|m| {
        r.x < m.x + m.width && m.x < r.x + r.width && r.y < m.y + m.height && m.y < r.y + r.height
    })
}

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

/// Queue a square cover image (`ytm:<key>`) if missing from cache.
fn queue_music_cover(state: &mut AppState, key: &str, url: &str, priority: ThumbPriority) {
    if key.is_empty() || url.is_empty() {
        return;
    }
    let full_key = format!("ytm:{key}");
    state.queue_thumb_with_priority(&full_key, url, priority);
}

/// Compute number of cover-grid columns fitting cleanly inside the Spotify main stage.
pub fn spotify_grid_cols(stage_width: u16) -> usize {
    ((stage_width.saturating_sub(2)) / 21).clamp(2, 5) as usize
}

/// Render the full 4-zone Spotify-style YouTube Music workspace below Phaenna's top tab bar.
pub fn render_music_workspace(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    state.music.hitboxes.clear();
    state.music.progress_bar_rect = Rect::default();

    if area.width < 40 || area.height < 12 {
        return;
    }

    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(4)])
        .split(area);

    let body_area = root_chunks[0];
    let bar_area = root_chunks[1];

    let show_sidebar = body_area.width >= 68;
    let show_drawer = state.music.right_drawer_open && body_area.width >= 104;
    let sidebar_w = if body_area.width >= 120 { 24 } else { 19 };
    let drawer_w = if body_area.width >= 136 { 35 } else { 29 };

    match (show_sidebar, show_drawer) {
        (true, true) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(sidebar_w),
                    Constraint::Min(32),
                    Constraint::Length(drawer_w),
                ])
                .split(body_area);
            render_spotify_sidebar(frame, cols[0], state);
            render_main_stage(frame, cols[1], state, modal_rect);
            render_right_drawer(frame, cols[2], state, modal_rect);
        }
        (true, false) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sidebar_w), Constraint::Min(32)])
                .split(body_area);
            render_spotify_sidebar(frame, cols[0], state);
            render_main_stage(frame, cols[1], state, modal_rect);
        }
        (false, true) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(28), Constraint::Length(drawer_w)])
                .split(body_area);
            render_main_stage(frame, cols[0], state, modal_rect);
            render_right_drawer(frame, cols[1], state, modal_rect);
        }
        (false, false) => {
            render_main_stage(frame, body_area, state, modal_rect);
        }
    }

    render_bottom_player_bar(frame, bar_area, state, modal_rect);
}

fn render_spotify_sidebar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme().clone();
    let spotify_green = Color::Rgb(30, 215, 96);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle))
        .style(Style::default().bg(Color::Rgb(14, 16, 22)));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 10 || inner.height < 6 {
        return;
    }

    let nav_items = [
        (YtmSection::Home, "Home"),
        (YtmSection::Search, "Search"),
        (YtmSection::Charts, "Charts"),
        (YtmSection::Liked, "Liked Music"),
        (YtmSection::Library, "Your Library"),
    ];

    let mut y = inner.y;
    for (sec, label) in nav_items {
        if y >= inner.y + inner.height {
            return;
        }
        let active = state.music.section == sec;
        let icon = match sec {
            YtmSection::Home => "⌂",
            YtmSection::Search => "⌕",
            YtmSection::Charts => "★",
            YtmSection::Liked => "♥",
            YtmSection::Library => "▤",
            YtmSection::Browse => "▸",
        };
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        state
            .music
            .hitboxes
            .push((row_rect, MusicHit::SelectSection(sec)));
        let st = if active {
            Style::default()
                .fg(Color::Black)
                .bg(spotify_green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(if sec == YtmSection::Liked {
                    Color::Rgb(255, 110, 150)
                } else {
                    theme.text
                })
                .bg(Color::Rgb(14, 16, 22))
        };
        let text = truncate_str(&format!(" {icon} {label}"), inner.width as usize);
        frame.render_widget(Paragraph::new(text).style(st), row_rect);
        y += 1;
    }

    // Dedicated Music Settings button inside sidebar
    if y < inner.y + inner.height {
        let s_rect = Rect::new(inner.x, y, inner.width, 1);
        state
            .music
            .hitboxes
            .push((s_rect, MusicHit::OpenMusicSettings));
        let st = Style::default()
            .fg(theme.accent)
            .bg(Color::Rgb(14, 16, 22))
            .add_modifier(Modifier::BOLD);
        let txt = truncate_str(" ⚙ Music Settings", inner.width as usize);
        frame.render_widget(Paragraph::new(txt).style(st), s_rect);
        y += 1;
    }

    if y + 1 < inner.y + inner.height {
        y += 1;
        let hdr_rect = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(
            Paragraph::new(" PLAYLISTS & ALBUMS").style(
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::BOLD),
            ),
            hdr_rect,
        );
        y += 1;
    }

    // Populate Library playlists / albums clickable shortcut rail
    let lib_items: Vec<YtmShelfItem> = state
        .music
        .library_page
        .as_ref()
        .map(|p| {
            p.shelves
                .iter()
                .flat_map(|s| s.items.iter().cloned())
                .collect()
        })
        .unwrap_or_default();

    if lib_items.is_empty() && y < inner.y + inner.height {
        let msg = if state.auth_vault.active_account().is_some() {
            " (Loading mixes…)"
        } else {
            " Sign in [L] to sync"
        };
        frame.render_widget(
            Paragraph::new(truncate_str(msg, inner.width as usize))
                .style(Style::default().fg(theme.text_dim)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        return;
    }

    for (idx, item) in lib_items.iter().enumerate() {
        if y >= inner.y + inner.height {
            break;
        }
        let r = Rect::new(inner.x, y, inner.width, 1);
        state
            .music
            .hitboxes
            .push((r, MusicHit::OpenSidebarPlaylist(idx)));
        let label = format!(" ◈ {}", item.title);
        frame.render_widget(
            Paragraph::new(truncate_str(&label, inner.width as usize))
                .style(Style::default().fg(theme.text_dim)),
            r,
        );
        y += 1;
    }
}

fn render_main_stage(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let theme = state.base_theme().clone();
    let show_search = state.music.section == YtmSection::Search;
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.music.search_editing {
            theme.border_idle
        } else {
            theme.primary
        }))
        .style(Style::default().bg(Color::Rgb(18, 20, 28)));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 16 || inner.height < 6 {
        return;
    }

    let hdr_h: u16 = if show_search { 3 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(hdr_h), Constraint::Min(4)])
        .split(inner);

    render_stage_header(frame, chunks[0], state, show_search);

    let stage = chunks[1];
    if let Some(ref err) = state.music.error {
        frame.render_widget(
            Paragraph::new(format!("Unable to load YouTube Music:\n{err}\n\nPress [r] to retry."))
                .style(Style::default().fg(Color::Rgb(245, 110, 110)))
                .wrap(Wrap { trim: true }),
            stage,
        );
        return;
    }

    let page_opt = state.music.current_page().cloned();
    let Some(page) = page_opt else {
        let msg = if state.music.loading {
            "Loading YouTube Music catalog…"
        } else if show_search {
            "Press [/] or click Search above to look up any song, album, artist, or playlist."
        } else {
            "Press [r] to refresh."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            Rect::new(stage.x, stage.y + stage.height / 3, stage.width, 2),
        );
        return;
    };

    // If the page is primarily a dense flat tracklist (Liked Music / direct track view) with no cover shelves,
    // render a crisp Spotify Track Table; otherwise render the Spotify Cover Cards Grid!
    if page.shelves.is_empty() && !page.tracks.is_empty() {
        render_track_table(frame, stage, state, &page, modal_rect);
    } else {
        render_cover_shelves_grid(frame, stage, state, &page, modal_rect);
    }
}

fn render_stage_header(frame: &mut Frame, area: Rect, state: &mut AppState, show_search: bool) {
    let theme = state.base_theme().clone();
    let top_row = Rect::new(area.x, area.y, area.width, 1);

    let mut spans = Vec::new();
    let mut x = top_row.x;

    if !state.music.browse_stack.is_empty() || state.music.section == YtmSection::Browse {
        let back_lbl = " <- Back ";
        let w = back_lbl.chars().count() as u16;
        state
            .music
            .hitboxes
            .push((Rect::new(x, top_row.y, w, 1), MusicHit::GoBack));
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

    let sec_title = match state.music.section {
        YtmSection::Browse if !state.music.browse_title.is_empty() => {
            format!(" {} ", state.music.browse_title)
        }
        YtmSection::Home => " Spotify-Style Home ".to_string(),
        YtmSection::Liked => " ♥ Liked Music ".to_string(),
        s => format!(" {} ", s.label()),
    };
    let t_len = sec_title.chars().count() as u16;
    spans.push(Span::styled(
        sec_title,
        Style::default()
            .fg(Color::Rgb(30, 215, 96))
            .add_modifier(Modifier::BOLD),
    ));
    x += t_len;

    if state.music.loading {
        spans.push(Span::styled(
            " Loading… ",
            Style::default().fg(theme.accent),
        ));
    } else {
        spans.push(Span::styled(
            " · Click Cover: Open Album / Play Song · [s] Audio Settings · [d] Drawer",
            Style::default().fg(theme.text_dim),
        ));
    }
    let _ = x;
    frame.render_widget(Paragraph::new(Line::from(spans)), top_row);

    if show_search && area.height >= 3 {
        let q_rect = Rect::new(area.x, area.y + 1, area.width, 1);
        state
            .music
            .hitboxes
            .push((q_rect, MusicHit::FocusSearch));
        let editing = state.music.search_editing;
        let cur_glyph = if editing { "▏" } else { "" };
        let q_txt = if state.music.search_query.is_empty() && !editing {
            "What do you want to listen to? (press / to type)".to_string()
        } else {
            format!("{}{cur_glyph}", state.music.search_query)
        };
        let box_st = if editing {
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(42, 48, 66))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.text_dim)
                .bg(Color::Rgb(28, 32, 44))
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ⌕ Search: ",
                    Style::default()
                        .fg(Color::Rgb(30, 215, 96))
                        .bg(Color::Rgb(28, 32, 44))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(q_txt, box_st),
            ]))
            .style(box_st),
            q_rect,
        );

        let chips_rect = Rect::new(area.x, area.y + 2, area.width, 1);
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
                    .bg(Color::Rgb(30, 215, 96))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.text_dim)
                    .bg(Color::Rgb(28, 32, 44))
            };
            chip_spans.push(Span::styled(lbl, st));
            chip_spans.push(Span::raw(" "));
            cx += w + 1;
        }
        frame.render_widget(Paragraph::new(Line::from(chip_spans)), chips_rect);
    }
}

fn render_cover_shelves_grid(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    page: &YtmPage,
    modal_rect: Option<Rect>,
) {
    let theme = state.base_theme().clone();
    let spotify_green = Color::Rgb(30, 215, 96);
    let cols = spotify_grid_cols(area.width);
    let card_gap: u16 = 1;
    let total_gaps = card_gap * (cols.saturating_sub(1) as u16);
    let card_w = (area.width.saturating_sub(total_gaps) / (cols as u16)).max(14);

    // Tall crisp 1:1 card height: 1px top pad + square art + 1 title + 1 sub + 1 border/pad
    let cover_h: u16 = if area.height >= 24 {
        7
    } else if area.height >= 16 {
        5
    } else {
        4
    };
    let card_h: u16 = cover_h + 4;

    struct ShelfRowBlock {
        shelf_title: Option<String>,
        items: Vec<(usize, YtmShelfItem)>,
    }

    let mut row_blocks: Vec<ShelfRowBlock> = Vec::new();
    let mut flat_idx = 0usize;
    for shelf in &page.shelves {
        if shelf.items.is_empty() {
            continue;
        }
        let mut first = true;
        for chunk in shelf.items.chunks(cols) {
            let mut row_items = Vec::with_capacity(chunk.len());
            for item in chunk {
                row_items.push((flat_idx, item.clone()));
                flat_idx += 1;
            }
            row_blocks.push(ShelfRowBlock {
                shelf_title: if first {
                    first = false;
                    Some(shelf.title.clone())
                } else {
                    None
                },
                items: row_items,
            });
        }
    }

    if row_blocks.is_empty() {
        return;
    }

    if state.music.cursor >= flat_idx {
        state.music.cursor = flat_idx.saturating_sub(1);
    }

    // Keep row containing cursor inside viewport
    let cursor_block_idx = row_blocks
        .iter()
        .position(|rb| rb.items.iter().any(|(idx, _)| *idx == state.music.cursor))
        .unwrap_or(0);
    if cursor_block_idx < state.music.scroll {
        state.music.scroll = cursor_block_idx;
    } else {
        // Measure cumulative height downward from current scroll
        let mut h_sum: u16 = 0;
        let mut last_fit = state.music.scroll;
        for (bi, rb) in row_blocks.iter().enumerate().skip(state.music.scroll) {
            let bh = (if rb.shelf_title.is_some() { 1 } else { 0 }) + card_h;
            if h_sum + bh <= area.height || bi == state.music.scroll {
                h_sum += bh;
                last_fit = bi;
            } else {
                break;
            }
        }
        if cursor_block_idx > last_fit {
            state.music.scroll = cursor_block_idx;
        }
    }

    let active_vid = state.active_playback_video_id.clone();
    let mut cur_y = area.y;

    for rb in row_blocks.iter().skip(state.music.scroll) {
        if let Some(ref stitle) = rb.shelf_title {
            if cur_y >= area.y + area.height {
                break;
            }
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    format!(" {stitle}"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )])),
                Rect::new(area.x, cur_y, area.width, 1),
            );
            cur_y += 1;
        }

        if cur_y + card_h > area.y + area.height {
            break;
        }

        for (c_i, (s_idx, item)) in rb.items.iter().enumerate() {
            let cx = area.x + (c_i as u16) * (card_w + card_gap);
            let card_rect = Rect::new(cx, cur_y, card_w, card_h);
            let is_sel = *s_idx == state.music.cursor;
            let is_playing = item
                .video_id
                .as_deref()
                .is_some_and(|v| active_vid.as_deref() == Some(v));

            let key = item.thumb_cache_key();
            if !key.is_empty() {
                let prio = if is_sel {
                    ThumbPriority::Focused
                } else {
                    ThumbPriority::Visible
                };
                queue_music_cover(state, &key, &item.thumb_url, prio);
            }

            state
                .music
                .hitboxes
                .push((card_rect, MusicHit::SelectItem(*s_idx)));

            let border_col = if is_playing {
                spotify_green
            } else if is_sel {
                theme.primary
            } else {
                Color::Rgb(42, 46, 58)
            };
            let card_bg = if is_sel {
                Color::Rgb(32, 37, 50)
            } else {
                Color::Rgb(22, 25, 34)
            };

            let card_blk = Block::default()
                .borders(Borders::ALL)
                .border_type(if is_sel || is_playing {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
                .border_style(Style::default().fg(border_col))
                .style(Style::default().bg(card_bg));
            let c_inner = card_blk.inner(card_rect);
            frame.render_widget(card_blk, card_rect);

            if c_inner.width < 6 || c_inner.height < 3 {
                continue;
            }

            let art_h = c_inner.height.saturating_sub(2);
            let art_rect = Rect::new(c_inner.x, c_inner.y, c_inner.width, art_h);
            let title_rect = Rect::new(c_inner.x, c_inner.y + art_h, c_inner.width, 1);
            let sub_rect = Rect::new(c_inner.x, c_inner.y + art_h + 1, c_inner.width, 1);

            let full_key = format!("ytm:{key}");
            if !key.is_empty() && !rect_intersects_optional(art_rect, modal_rect) {
                render_thumbnail_cover(
                    frame,
                    art_rect,
                    &full_key,
                    state,
                    item.kind.badge(),
                );
            } else {
                frame.render_widget(
                    Paragraph::new(format!("\n♪ {}", item.kind.badge()))
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(border_col).bg(card_bg)),
                    art_rect,
                );
            }

            let play_pfx = if is_playing { "▶ " } else { "" };
            let title_line = truncate_str(
                &format!("{play_pfx}{}", item.title),
                title_rect.width as usize,
            );
            frame.render_widget(
                Paragraph::new(title_line).style(
                    Style::default()
                        .fg(if is_playing { spotify_green } else { Color::White })
                        .bg(card_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                title_rect,
            );

            let kind_tag = match item.kind {
                YtmItemKind::Album => "Album",
                YtmItemKind::Playlist => "Playlist",
                YtmItemKind::Artist => "Artist",
                YtmItemKind::Radio => "Mix",
                YtmItemKind::Song | YtmItemKind::Video => "",
            };
            let sub_txt = if !item.subtitle.is_empty() {
                item.subtitle.clone()
            } else {
                kind_tag.to_string()
            };
            frame.render_widget(
                Paragraph::new(truncate_str(&sub_txt, sub_rect.width as usize))
                    .style(Style::default().fg(theme.text_dim).bg(card_bg)),
                sub_rect,
            );
        }

        cur_y += card_h;
    }
}

fn render_track_table(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    page: &YtmPage,
    modal_rect: Option<Rect>,
) {
    let theme = state.base_theme().clone();
    let spotify_green = Color::Rgb(30, 215, 96);
    let _ = modal_rect;

    if area.height < 3 {
        return;
    }

    let hdr = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        Paragraph::new("  #   TITLE                                 ARTIST / ALBUM              TIME")
            .style(
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::BOLD),
            ),
        hdr,
    );

    let list_area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    let total = page.tracks.len();
    if total == 0 {
        return;
    }
    if state.music.cursor >= total {
        state.music.cursor = total - 1;
    }

    let vis = list_area.height as usize;
    if state.music.cursor < state.music.scroll {
        state.music.scroll = state.music.cursor;
    } else if state.music.cursor >= state.music.scroll + vis {
        state.music.scroll = state.music.cursor + 1 - vis;
    }

    let active_vid = state.active_playback_video_id.clone();
    for (vi, (idx, tr)) in page
        .tracks
        .iter()
        .enumerate()
        .skip(state.music.scroll)
        .take(vis)
        .enumerate()
    {
        let r = Rect::new(list_area.x, list_area.y + vi as u16, list_area.width, 1);
        state.music.hitboxes.push((r, MusicHit::SelectItem(idx)));

        let is_sel = idx == state.music.cursor;
        let is_playing = active_vid.as_deref() == Some(tr.video_id.as_str());
        let bg = if is_sel {
            Color::Rgb(36, 44, 62)
        } else if is_playing {
            Color::Rgb(20, 42, 30)
        } else {
            Color::Reset
        };
        let lead = if is_playing {
            "▶ ".to_string()
        } else {
            format!("{:>2}.", idx + 1)
        };
        let liked = state.music.liked_ids.contains(&tr.video_id);
        let heart = if liked { "♥ " } else { "  " };
        let dur = tr
            .duration_text
            .clone()
            .or_else(|| tr.duration_secs.map(format_duration_secs))
            .unwrap_or_default();

        let w = list_area.width as usize;
        let t_budget = ((w.saturating_sub(14)) * 52) / 100;
        let a_budget = (w.saturating_sub(14)).saturating_sub(t_budget);
        let t_str = truncate_str(&tr.title, t_budget.max(8));
        let meta = if let Some(ref alb) = tr.album {
            format!("{} · {alb}", tr.artist)
        } else {
            tr.artist.clone()
        };
        let a_str = truncate_str(&meta, a_budget.max(6));

        let spans = vec![
            Span::styled(
                format!(" {lead:<3} "),
                Style::default()
                    .fg(if is_playing { spotify_green } else { theme.text_dim })
                    .bg(bg),
            ),
            Span::styled(
                format!("{t_str:<width$}", width = t_budget.max(8)),
                Style::default()
                    .fg(if is_playing { spotify_green } else { Color::White })
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{a_str:<width$}", width = a_budget.max(6)),
                Style::default().fg(theme.text_dim).bg(bg),
            ),
            Span::styled(
                heart,
                Style::default().fg(Color::Rgb(255, 90, 130)).bg(bg),
            ),
            Span::styled(dur, Style::default().fg(theme.text_dim).bg(bg)),
        ];
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
            r,
        );
    }
}

fn render_right_drawer(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let cur_track = state.music.current_track().cloned();
    let items = state.music.selectable_items();
    let preview = items.get(state.music.cursor).cloned();

    let (art_id, art_url, title, artist, album_opt) = if let Some(ref tr) = cur_track {
        (
            format!("ytm:{}", tr.thumb_cache_key()),
            tr.thumb_url.clone(),
            tr.title.clone(),
            tr.artist.clone(),
            tr.album.clone(),
        )
    } else if let Some(ref p) = preview {
        (
            format!("ytm:{}", p.thumb_cache_key()),
            p.thumb_url.clone(),
            p.title.clone(),
            p.subtitle.clone(),
            None,
        )
    } else {
        (
            String::new(),
            String::new(),
            "Nothing Playing".to_string(),
            "Click any song or album cover".to_string(),
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

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary))
        .title(Span::styled(
            " Now Playing ",
            Style::default()
                .fg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(15, 18, 26)));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 12 || inner.height < 10 {
        return;
    }

    let art_h = (inner.height / 2).clamp(6, 13);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(art_h),
            Constraint::Length(3),
            Constraint::Min(4),
        ])
        .split(inner);

    if !art_id.is_empty() && !rect_intersects_optional(parts[0], modal_rect) {
        render_thumbnail_cover(frame, parts[0], &art_id, state, "♪");
    } else {
        frame.render_widget(
            Paragraph::new("\n♪\nYTM")
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.primary).bg(Color::Rgb(20, 24, 34))),
            parts[0],
        );
    }

    let w = parts[1].width as usize;
    let marq = marquee_window(&title, w.max(6), state.ticker_step);
    let sub_line = if let Some(ref alb) = album_opt {
        format!("{artist} · {alb}")
    } else {
        artist
    };
    let album_btn_rect = Rect::new(parts[1].x, parts[1].y + 2, parts[1].width, 1);
    if cur_track.as_ref().is_some_and(|t| t.album_browse_id.is_some()) {
        state
            .music
            .hitboxes
            .push((album_btn_rect, MusicHit::OpenCurrentAlbum));
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                marq,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate_str(&sub_line, w),
                Style::default().fg(pal.text_dim),
            )),
            Line::from(Span::styled(
                if cur_track.as_ref().is_some_and(|t| t.album_browse_id.is_some()) {
                    "[Open Album Popup]"
                } else {
                    ""
                },
                Style::default().fg(pal.accent),
            )),
        ]),
        parts[1],
    );

    render_drawer_queue_or_lyrics(frame, parts[2], state, &pal);
}

fn render_drawer_queue_or_lyrics(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &crate::ui::theme::ThemePalette,
) {
    if area.height < 3 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(area);

    let tab_bar = chunks[0];
    let up_lbl = format!(" Up Next ({}) ", state.music.queue.len());
    let lyr_lbl = if state.music.lyrics.as_ref().is_some_and(|l| l.is_timed) {
        " Lyrics (Sync) ".to_string()
    } else {
        " Lyrics ".to_string()
    };
    let up_w = up_lbl.chars().count() as u16;
    let lyr_w = lyr_lbl.chars().count() as u16;

    state.music.hitboxes.push((
        Rect::new(tab_bar.x, tab_bar.y, up_w, 1),
        MusicHit::SelectRightTab(YtmRightPaneTab::UpNext),
    ));
    state.music.hitboxes.push((
        Rect::new(tab_bar.x + up_w + 1, tab_bar.y, lyr_w, 1),
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
            .bg(Color::Rgb(30, 215, 96))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.text_dim)
            .bg(Color::Rgb(24, 28, 38))
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(up_lbl, up_st),
            Span::raw(" "),
            Span::styled(lyr_lbl, lyr_st),
        ])),
        tab_bar,
    );

    let body = chunks[1];
    match state.music.right_tab {
        YtmRightPaneTab::UpNext => render_up_next_list(frame, body, state, theme),
        YtmRightPaneTab::Lyrics => render_lyrics_view(frame, body, state, theme),
    }
}

fn render_bottom_player_bar(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let spotify_green = Color::Rgb(30, 215, 96);
    let theme = state.base_theme().clone();
    let bar_bg = Color::Rgb(14, 16, 24);
    let blk = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::Rgb(38, 42, 56)))
        .style(Style::default().bg(bar_bg));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    if inner.width < 36 || inner.height < 2 {
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(32),
            Constraint::Percentage(42),
            Constraint::Percentage(26),
        ])
        .split(inner);

    // 1. Left section: Mini 1:1 cover + Title + Artist + Heart (Click opens Album Popup!)
    let cur = state.music.current_track().cloned();
    let left_area = cols[0];
    let cover_w: u16 = if left_area.width >= 20 { 6 } else { 0 };
    let (mini_cover_rect, meta_rect) = if cover_w > 0 {
        let lsplit = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(cover_w), Constraint::Min(8)])
            .split(left_area);
        (Some(lsplit[0]), lsplit[1])
    } else {
        (None, left_area)
    };

    if let (Some(c_rect), Some(ref tr)) = (mini_cover_rect, &cur) {
        let key = format!("ytm:{}", tr.thumb_cache_key());
        if !tr.thumb_url.is_empty() {
            state.queue_thumb_with_priority(&key, &tr.thumb_url, ThumbPriority::Focused);
        }
        if !rect_intersects_optional(c_rect, modal_rect) {
            render_thumbnail_cover(frame, c_rect, &key, state, "♪");
        }
        state
            .music
            .hitboxes
            .push((c_rect, MusicHit::OpenCurrentAlbum));
    }

    if let Some(ref tr) = cur {
        let liked = state.music.liked_ids.contains(&tr.video_id);
        let heart_lbl = if liked { " ♥" } else { " ♡" };
        let w = meta_rect.width.saturating_sub(4) as usize;
        let t_str = marquee_window(&tr.title, w.max(6), state.ticker_step);
        let sub_str = tr
            .album
            .as_deref()
            .map(|alb| format!("{} · {alb}", tr.artist))
            .unwrap_or_else(|| tr.artist.clone());

        let heart_x = meta_rect.x + (t_str.chars().count() as u16 + 1).min(meta_rect.width.saturating_sub(2));
        state
            .music
            .hitboxes
            .push((Rect::new(heart_x, meta_rect.y, 3, 1), MusicHit::ToggleLike));
        if tr.album_browse_id.is_some() {
            state.music.hitboxes.push((
                Rect::new(meta_rect.x, meta_rect.y + 1, meta_rect.width, 1),
                MusicHit::OpenCurrentAlbum,
            ));
        }

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {t_str}"),
                        Style::default()
                            .fg(Color::White)
                            .bg(bar_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        heart_lbl,
                        Style::default()
                            .fg(if liked {
                                spotify_green
                            } else {
                                theme.text_dim
                            })
                            .bg(bar_bg),
                    ),
                ]),
                Line::from(Span::styled(
                    format!(" {}", truncate_str(&sub_str, w)),
                    Style::default().fg(theme.text_dim).bg(bar_bg),
                )),
            ])
            .style(Style::default().bg(bar_bg)),
            meta_rect,
        );
    } else {
        frame.render_widget(
            Paragraph::new(" ♪ Pick a track or album cover to start listening")
                .style(Style::default().fg(theme.text_dim).bg(bar_bg)),
            meta_rect,
        );
    }

    // 2. Center section: Transport row + Spotify progress bar
    let center = cols[1];
    let c_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(center);

    let play_lbl = if cur.is_some() && !state.live_playback.paused && state.has_live_ipc() {
        " ⏸ "
    } else {
        " ▶ "
    };
    let btns: Vec<(&str, MusicHit, Color, Color)> = vec![
        (
            if state.music.shuffle { " Shuf " } else { " shuf " },
            MusicHit::ToggleShuffle,
            if state.music.shuffle { spotify_green } else { theme.text_dim },
            bar_bg,
        ),
        (" |< ", MusicHit::PrevTrack, Color::White, bar_bg),
        (play_lbl, MusicHit::TogglePlayPause, Color::Black, Color::White),
        (" >| ", MusicHit::NextTrack, Color::White, bar_bg),
        (
            match state.music.repeat {
                YtmRepeatMode::Off => " rep ",
                YtmRepeatMode::All => " Rep ",
                YtmRepeatMode::One => " Rep1 ",
            },
            MusicHit::CycleRepeat,
            if state.music.repeat != YtmRepeatMode::Off {
                spotify_green
            } else {
                theme.text_dim
            },
            bar_bg,
        ),
    ];
    let total_btn_w: u16 = btns.iter().map(|b| b.0.chars().count() as u16 + 1).sum();
    let mut bx = c_rows[0].x + c_rows[0].width.saturating_sub(total_btn_w) / 2;
    let mut bspans = Vec::new();
    if bx > c_rows[0].x {
        bspans.push(Span::raw(" ".repeat((bx - c_rows[0].x) as usize)));
    }
    for (lbl, hit, fg, bg) in btns {
        let w = lbl.chars().count() as u16;
        state
            .music
            .hitboxes
            .push((Rect::new(bx, c_rows[0].y, w, 1), hit));
        bspans.push(Span::styled(
            lbl,
            Style::default()
                .fg(fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ));
        bspans.push(Span::raw(" "));
        bx += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(bspans)), c_rows[0]);

    // Progress bar row
    let dur_secs = if state.active_playback_dur > 0.0 {
        state.active_playback_dur
    } else {
        cur.as_ref()
            .and_then(|t| t.duration_secs)
            .map(|d| d as f64)
            .unwrap_or(0.0)
    };
    let pos_secs = if cur.is_some() {
        state.active_playback_pos.clamp(0.0, dur_secs.max(state.active_playback_pos))
    } else {
        0.0
    };
    let t_left = format_duration_secs(pos_secs as u64);
    let t_right = format_duration_secs(dur_secs as u64);
    let bar_space = (c_rows[1].width as usize).saturating_sub(t_left.len() + t_right.len() + 4);
    if bar_space >= 6 {
        let ratio = if dur_secs > 0.0 {
            (pos_secs / dur_secs).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled = ((bar_space - 1) as f64 * ratio).round() as usize;
        let rem = bar_space.saturating_sub(filled + 1);
        let seek_rect = Rect::new(
            c_rows[1].x + t_left.len() as u16 + 2,
            c_rows[1].y,
            bar_space as u16,
            1,
        );
        state.music.progress_bar_rect = seek_rect;
        state.music.hitboxes.push((seek_rect, MusicHit::SeekBar));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {t_left} "), Style::default().fg(theme.text_dim)),
                Span::styled("━".repeat(filled), Style::default().fg(spotify_green)),
                Span::styled("●", Style::default().fg(Color::White)),
                Span::styled("─".repeat(rem), Style::default().fg(Color::Rgb(70, 76, 92))),
                Span::styled(format!(" {t_right}"), Style::default().fg(theme.text_dim)),
            ])),
            c_rows[1],
        );
    }

    // 3. Right section: Lyrics/Queue drawer toggle, Video/Audio switch, Volume, Settings
    let right_area = cols[2];
    let vol_lbl = if state.live_playback.muted {
        "MUTE".to_string()
    } else {
        format!("{}%", state.live_playback.volume.clamp(0, 100))
    };
    let r_btns: Vec<(&str, MusicHit, Color)> = vec![
        (
            if state.music.right_drawer_open {
                " [Drawer] "
            } else {
                " [Drawer] "
            },
            MusicHit::ToggleRightDrawer,
            if state.music.right_drawer_open {
                spotify_green
            } else {
                theme.text_dim
            },
        ),
        (
            if state.music.video_mode { " Vid " } else { " Aud " },
            MusicHit::ToggleVideoMode,
            if state.music.video_mode {
                Color::Rgb(210, 120, 255)
            } else {
                spotify_green
            },
        ),
        (" - ", MusicHit::VolumeDelta(-5), theme.text_dim),
        (" Vol ", MusicHit::ToggleMute, Color::White),
        (" + ", MusicHit::VolumeDelta(5), theme.text_dim),
        (" ⚙ ", MusicHit::OpenMusicSettings, theme.accent),
    ];
    let mut rx = right_area.x;
    let mut rspans = Vec::new();
    for (lbl, hit, col) in r_btns {
        let w = lbl.chars().count() as u16;
        if rx + w <= right_area.x + right_area.width {
            state
                .music
                .hitboxes
                .push((Rect::new(rx, right_area.y, w, 1), hit));
            rspans.push(Span::styled(
                lbl,
                Style::default()
                    .fg(col)
                    .bg(bar_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            rx += w;
        }
    }
    frame.render_widget(Paragraph::new(Line::from(rspans)), right_area);
    if right_area.height >= 2 {
        let info = format!(
            " Vol: {vol_lbl} · EQ:{} · {}",
            state.config.music.eq_preset.short_label(),
            if state.config.music.anti_clip_limiter {
                "Hi-Fi Safe"
            } else {
                "Raw"
            }
        );
        frame.render_widget(
            Paragraph::new(truncate_str(&info, right_area.width as usize))
                .style(Style::default().fg(theme.text_dim).bg(bar_bg)),
            Rect::new(right_area.x, right_area.y + 1, right_area.width, 1),
        );
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
            Paragraph::new("\nQueue empty — click any track to start radio mix.")
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

    let spotify_green = Color::Rgb(30, 215, 96);
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
        let bg = if is_cur {
            Color::Rgb(22, 42, 30)
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
        let avail = (area.width as usize).saturating_sub(dur.chars().count() + 6);
        let t_w = (avail * 58) / 100;
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
                    format!("{lead:<3}"),
                    Style::default()
                        .fg(if is_cur { spotify_green } else { theme.text_dim })
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    title_str,
                    Style::default()
                        .fg(if is_cur { spotify_green } else { theme.text })
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
        let msg = if state.music.lyrics_browse_id.is_none() && state.music.current_track().is_some()
        {
            "\nLyrics unavailable for this track."
        } else {
            "\nPlay a song to view synced lyrics."
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
            Paragraph::new("\nInstrumental / no lyrics.")
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
                .fg(Color::Rgb(30, 215, 96))
                .add_modifier(Modifier::BOLD)
        } else if is_past {
            Style::default().fg(theme.text_dim)
        } else {
            Style::default().fg(Color::White)
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
        Paragraph::new(rendered_lines).wrap(Wrap { trim: true }),
        area,
    );
}

/// Centered Spotify-style Album / Playlist / Artist Detail Popup Modal.
pub fn render_album_popup_modal(frame: &mut Frame, state: &mut AppState) {
    let Some(modal_area) =
        crate::state::active_modal_rect(frame.area(), crate::state::ActiveOverlay::MusicAlbum)
    else {
        return;
    };
    state.music.album_popup.hitboxes.clear();

    frame.render_widget(Clear, modal_area);

    let art_key = format!("ytm:b:{}", state.music.album_popup.browse_id);
    if let Some(url) = state
        .music
        .album_popup
        .page
        .as_ref()
        .and_then(|p| p.header_thumb.clone())
    {
        if !url.is_empty() {
            state.queue_thumb_with_priority(&art_key, &url, ThumbPriority::Focused);
        }
    }
    let pal = state.active_palette_for(Some(&art_key)).clone();
    let spotify_green = Color::Rgb(30, 215, 96);

    let title_hdr = if state.music.album_popup.title.is_empty() {
        " Album / Playlist ".to_string()
    } else {
        format!(" {} ", state.music.album_popup.title)
    };
    let outer_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary))
        .title(Span::styled(
            title_hdr,
            Style::default()
                .fg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(16, 19, 28)));
    let inner = outer_blk.inner(modal_area);
    frame.render_widget(outer_blk, modal_area);

    if inner.width < 32 || inner.height < 12 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    // 1. Top Nav bar ([<- Back] / [Esc Close])
    let nav = chunks[0];
    let mut nx = nav.x + 1;
    let mut nav_spans = vec![Span::raw(" ")];
    if !state.music.album_popup.history.is_empty() {
        let b_lbl = " <- Back ";
        let bw = b_lbl.chars().count() as u16;
        state
            .music
            .album_popup
            .hitboxes
            .push((Rect::new(nx, nav.y, bw, 1), YtmAlbumPopupHit::Back));
        nav_spans.push(Span::styled(
            b_lbl,
            Style::default()
                .fg(Color::Black)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ));
        nav_spans.push(Span::raw(" "));
        nx += bw + 1;
    }
    let close_lbl = " [✕ Close] ";
    let cw = close_lbl.chars().count() as u16;
    let cx = nav.x + nav.width.saturating_sub(cw + 1);
    state
        .music
        .album_popup
        .hitboxes
        .push((Rect::new(cx, nav.y, cw, 1), YtmAlbumPopupHit::Close));
    if cx > nx {
        nav_spans.push(Span::raw(" ".repeat((cx - nx) as usize)));
    }
    nav_spans.push(Span::styled(
        close_lbl,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(190, 80, 95))
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(Line::from(nav_spans)), nav);

    // 2. Hero Header (1:1 Cover Art + Rich Metadata + Play / Shuffle / Artist buttons)
    let hero = chunks[1];
    let cover_w = 20u16.min(hero.width / 3);
    let hsplit = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(cover_w), Constraint::Min(18)])
        .split(hero);
    let cov_rect = Rect::new(
        hsplit[0].x + 1,
        hsplit[0].y,
        hsplit[0].width.saturating_sub(2),
        hsplit[0].height,
    );
    if state
        .music
        .album_popup
        .page
        .as_ref()
        .and_then(|p| p.header_thumb.as_ref())
        .is_some()
    {
        render_thumbnail_cover(frame, cov_rect, &art_key, state, "♪ Album");
    } else {
        frame.render_widget(
            Paragraph::new("\n\n♪ COVER")
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.primary).bg(Color::Rgb(24, 28, 40))),
            cov_rect,
        );
    }

    let info_rect = hsplit[1];
    let page_opt = state.music.album_popup.page.clone();
    if let Some(ref p) = page_opt {
        let title = p
            .header_title
            .as_deref()
            .unwrap_or(&state.music.album_popup.title);
        let artist_line = p
            .header_artist
            .as_deref()
            .or(p.header_subtitle.as_deref())
            .unwrap_or("");
        let summary = p.header_track_summary.as_deref().unwrap_or("");
        let year = p.header_year.as_deref().unwrap_or("");
        let mut meta_parts = Vec::new();
        if !artist_line.is_empty() {
            meta_parts.push(artist_line.to_string());
        }
        if !year.is_empty() && !artist_line.contains(year) {
            meta_parts.push(year.to_string());
        }
        if !summary.is_empty() {
            meta_parts.push(summary.to_string());
        }
        let info_w = info_rect.width.saturating_sub(2) as usize;
        let btn_y = info_rect.y + 6.min(info_rect.height.saturating_sub(2));

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " ALBUM / COLLECTION",
                    Style::default()
                        .fg(pal.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", truncate_str(title, info_w)),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", truncate_str(&meta_parts.join(" · "), info_w)),
                    Style::default().fg(pal.text_dim),
                )),
                Line::from(Span::styled(
                    format!(" {} tracks loaded", p.tracks.len()),
                    Style::default().fg(spotify_green),
                )),
            ]),
            Rect::new(info_rect.x, info_rect.y + 1, info_rect.width, 5),
        );

        // Action Pills Row
        let mut bx = info_rect.x + 1;
        let mut bspans = vec![Span::raw(" ")];
        let mut actions: Vec<(&str, YtmAlbumPopupHit, Color, Color)> = vec![
            (
                " ▶ Play All ",
                YtmAlbumPopupHit::PlayAll,
                Color::Black,
                spotify_green,
            ),
            (
                " 🔀 Shuffle ",
                YtmAlbumPopupHit::ShuffleAll,
                Color::White,
                Color::Rgb(42, 48, 64),
            ),
        ];
        if p.header_artist_browse_id.is_some() {
            actions.push((
                " View Artist > ",
                YtmAlbumPopupHit::OpenArtist,
                Color::Black,
                pal.accent,
            ));
        }
        for (lbl, hit, fg, bg) in actions {
            let w = lbl.chars().count() as u16;
            if bx + w <= info_rect.x + info_rect.width {
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((Rect::new(bx, btn_y, w, 1), hit));
                bspans.push(Span::styled(
                    lbl,
                    Style::default()
                        .fg(fg)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ));
                bspans.push(Span::raw(" "));
                bx += w + 1;
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(bspans)),
            Rect::new(info_rect.x, btn_y, info_rect.width, 1),
        );
    }

    // 3. Body: Tracklist OR Artist Albums/Shelves
    let body = chunks[2];
    if state.music.album_popup.loading {
        frame.render_widget(
            Paragraph::new("\nLoading album & tracklist from YouTube Music…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.accent)),
            body,
        );
    } else if let Some(ref err) = state.music.album_popup.error {
        frame.render_widget(
            Paragraph::new(format!("\nFailed to load: {err}"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Rgb(245, 110, 110))),
            body,
        );
    } else if let Some(ref p) = page_opt {
        if !p.tracks.is_empty() {
            let total = p.tracks.len();
            if state.music.album_popup.cursor >= total {
                state.music.album_popup.cursor = total - 1;
            }
            let vis = body.height.saturating_sub(2) as usize;
            if state.music.album_popup.cursor < state.music.album_popup.scroll {
                state.music.album_popup.scroll = state.music.album_popup.cursor;
            } else if vis > 0 && state.music.album_popup.cursor >= state.music.album_popup.scroll + vis
            {
                state.music.album_popup.scroll = state.music.album_popup.cursor + 1 - vis;
            }

            frame.render_widget(
                Paragraph::new("   #   TRACK                                  ARTIST / PLAYS        TIME")
                    .style(
                        Style::default()
                            .fg(pal.text_dim)
                            .add_modifier(Modifier::BOLD),
                    ),
                Rect::new(body.x + 1, body.y, body.width.saturating_sub(2), 1),
            );
            let active_vid = state.active_playback_video_id.clone();

            for (vi, (t_idx, tr)) in p
                .tracks
                .iter()
                .enumerate()
                .skip(state.music.album_popup.scroll)
                .take(vis)
                .enumerate()
            {
                let row_y = body.y + 1 + vi as u16;
                let r = Rect::new(body.x + 1, row_y, body.width.saturating_sub(2), 1);
                let is_sel = t_idx == state.music.album_popup.cursor;
                let is_playing = active_vid.as_deref() == Some(tr.video_id.as_str());
                let bg = if is_sel {
                    Color::Rgb(38, 46, 66)
                } else if is_playing {
                    Color::Rgb(20, 42, 30)
                } else {
                    Color::Reset
                };
                let liked = state.music.liked_ids.contains(&tr.video_id);
                let lead = if is_playing {
                    "▶ ".to_string()
                } else {
                    format!("{:>2}.", t_idx + 1)
                };
                let dur = tr
                    .duration_text
                    .clone()
                    .or_else(|| tr.duration_secs.map(format_duration_secs))
                    .unwrap_or_default();
                let w = r.width as usize;
                let t_w = (w.saturating_sub(26) * 58) / 100;
                let a_w = w.saturating_sub(26 + t_w);
                let artist_plays = tr
                    .plays_text
                    .as_deref()
                    .map(|pl| format!("{} · {pl}", tr.artist))
                    .unwrap_or_else(|| tr.artist.clone());

                // Action pills on right: [+Q] [♥]
                let pill_q = " +Q ";
                let pill_h = if liked { " ♥ " } else { " ♡ " };
                let hx = r.x + r.width.saturating_sub(14);
                let qx = r.x + r.width.saturating_sub(10);
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((Rect::new(hx, row_y, 3, 1), YtmAlbumPopupHit::LikeTrack(t_idx)));
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((Rect::new(qx, row_y, 4, 1), YtmAlbumPopupHit::QueueTrack(t_idx)));
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((r, YtmAlbumPopupHit::PlayTrack(t_idx)));

                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!(" {lead:<3} "),
                            Style::default()
                                .fg(if is_playing { spotify_green } else { pal.text_dim })
                                .bg(bg),
                        ),
                        Span::styled(
                            format!(
                                "{:<w$}",
                                truncate_str(&tr.title, t_w.max(6)),
                                w = t_w.max(6)
                            ),
                            Style::default()
                                .fg(if is_playing { spotify_green } else { Color::White })
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!(
                                "{:<w$}",
                                truncate_str(&artist_plays, a_w.max(6)),
                                w = a_w.max(6)
                            ),
                            Style::default().fg(pal.text_dim).bg(bg),
                        ),
                        Span::styled(
                            pill_h,
                            Style::default()
                                .fg(if liked {
                                    spotify_green
                                } else {
                                    pal.text_dim
                                })
                                .bg(bg),
                        ),
                        Span::styled(pill_q, Style::default().fg(pal.accent).bg(bg)),
                        Span::styled(format!(" {dur:>5}"), Style::default().fg(pal.text_dim).bg(bg)),
                    ]))
                    .style(Style::default().bg(bg)),
                    r,
                );
            }
        } else {
            // Artist / Hub discography view inside popup
            let flat_items: Vec<YtmShelfItem> = p
                .shelves
                .iter()
                .flat_map(|s| s.items.iter().cloned())
                .collect();
            let vis = body.height as usize;
            if !flat_items.is_empty() && state.music.album_popup.cursor >= flat_items.len() {
                state.music.album_popup.cursor = flat_items.len() - 1;
            }
            for (vi, (i_idx, item)) in flat_items
                .iter()
                .enumerate()
                .skip(state.music.album_popup.scroll)
                .take(vis)
                .enumerate()
            {
                let row_y = body.y + vi as u16;
                let r = Rect::new(body.x + 1, row_y, body.width.saturating_sub(2), 1);
                let is_sel = i_idx == state.music.album_popup.cursor;
                let bg = if is_sel {
                    Color::Rgb(38, 46, 66)
                } else {
                    Color::Reset
                };
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((r, YtmAlbumPopupHit::OpenItem(i_idx)));
                let line = format!(" [{}] {} · {}", item.kind.badge(), item.title, item.subtitle);
                frame.render_widget(
                    Paragraph::new(truncate_str(&line, r.width as usize))
                        .style(Style::default().fg(Color::White).bg(bg)),
                    r,
                );
            }
        }
    }

    frame.render_widget(
        Paragraph::new("[Click / Enter] Play Song in Album · [+Q / a] Queue Next · [♥ / l] Like · [Esc] Close Popup")
            .alignment(Alignment::Center)
            .style(Style::default().fg(pal.text_dim)),
        chunks[3],
    );
}

/// Separate YouTube Music Settings Overlay (decoupled from Video/Shader/SponsorBlock Settings).
pub fn render_music_settings_modal(frame: &mut Frame, state: &mut AppState) {
    let Some(modal_area) =
        crate::state::active_modal_rect(frame.area(), crate::state::ActiveOverlay::MusicSettings)
    else {
        return;
    };
    state.music.settings_hitboxes.clear();
    frame.render_widget(Clear, modal_area);

    let theme = state.base_theme().clone();
    let spotify_green = Color::Rgb(30, 215, 96);

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(spotify_green))
        .title(Span::styled(
            " ♪ YouTube Music & Hi-Fi Audio Settings ",
            Style::default()
                .fg(spotify_green)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(16, 19, 28)));
    let inner = blk.inner(modal_area);
    frame.render_widget(blk, modal_area);
    if inner.width < 24 || inner.height < 8 {
        return;
    }

    let rows = music_settings_rows(&state.config.music);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(" Separate Spotify / Audio Engine preferences (saved under [music] in config.toml)")
            .style(Style::default().fg(theme.text_dim)),
        chunks[0],
    );

    let list_area = chunks[1];
    for (idx, (label, val, hint)) in rows.iter().enumerate() {
        let ry = list_area.y + (idx as u16) * 2;
        if ry + 1 >= list_area.y + list_area.height {
            break;
        }
        let r = Rect::new(list_area.x + 1, ry, list_area.width.saturating_sub(2), 2);
        let sel = idx == state.music.settings_cursor;
        let bg = if sel {
            Color::Rgb(32, 42, 56)
        } else {
            Color::Rgb(20, 24, 34)
        };
        state
            .music
            .settings_hitboxes
            .push((r, MusicSettingsHit::ActivateRow(idx)));

        let w = r.width as usize;
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(
                        if sel { " ▸ " } else { "   " },
                        Style::default().fg(spotify_green).bg(bg),
                    ),
                    Span::styled(
                        format!("{label:<32}"),
                        Style::default()
                            .fg(Color::White)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        truncate_str(val, w.saturating_sub(36)),
                        Style::default()
                            .fg(spotify_green)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("   {}", truncate_str(hint, w.saturating_sub(4))),
                    Style::default().fg(theme.text_dim).bg(bg),
                )),
            ])
            .style(Style::default().bg(bg)),
            r,
        );
    }

    let close_rect = Rect::new(
        chunks[2].x + chunks[2].width.saturating_sub(14),
        chunks[2].y,
        12,
        1,
    );
    state
        .music
        .settings_hitboxes
        .push((close_rect, MusicSettingsHit::Close));
    frame.render_widget(
        Paragraph::new("[Up/Down] Navigate · [Enter / Click / Space] Cycle & Apply Live · [Esc] Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[2],
    );
}

pub const MUSIC_SETTINGS_ROW_COUNT: usize = 10;

pub fn music_settings_rows(cfg: &MusicSettingsConfig) -> [(&'static str, String, &'static str); MUSIC_SETTINGS_ROW_COUNT] {
    [
        (
            "Audio Stream Quality",
            cfg.audio_quality.label().to_string(),
            "Hi-Fi Opus 48 kHz recommended for clearest fidelity",
        ),
        (
            "Volume Normalization",
            cfg.normalization.label().to_string(),
            "Prevents loud mastered tracks from slamming into clipping",
        ),
        (
            "Anti-Clipping Limiter",
            if cfg.anti_clip_limiter {
                "ON (-2 dB Headroom + Peak Limiter)".to_string()
            } else {
                "OFF (Raw Gain)".to_string()
            },
            "Eliminates crackle & inter-sample digital distortion",
        ),
        (
            "Music Equalizer Preset",
            cfg.eq_preset.label().to_string(),
            "Dedicated Music 10-band EQ with automatic headroom",
        ),
        (
            "Manual Preamp Trim",
            format!("{:+} dB", cfg.preamp_db),
            "Adjust extra gain reduction if using hot DAC/headphones",
        ),
        (
            "Spatial Channel Stage",
            cfg.channel_mode.label().to_string(),
            "Stereo, natural headphone Crossfeed, Wide Stage, or Mono",
        ),
        (
            "Right Now Playing Drawer",
            if cfg.show_right_drawer {
                "Expanded by default ([d] toggles)".to_string()
            } else {
                "Collapsed by default ([d] toggles)".to_string()
            },
            "Right-hand Now Playing art + Up Next queue + Synced Lyrics",
        ),
        (
            "Default Playback Mode",
            if cfg.default_video_mode {
                "Music Video Window".to_string()
            } else {
                "Windowless Background Audio".to_string()
            },
            "Windowless Audio runs silent in PipeWire/ALSA cleanly",
        ),
        (
            "Gapless Single-Player",
            if cfg.gapless_audio {
                "ON (Instant IPC Switch)".to_string()
            } else {
                "OFF (Respawn per Track)".to_string()
            },
            "Reuses idle mpv audio engine for instant zero-gap transitions",
        ),
        (
            "Autoplay Mix Radio",
            if cfg.autoplay_mix {
                "ON (Infinite Up Next)".to_string()
            } else {
                "OFF (Play Selected Only)".to_string()
            },
            "Builds similar tracks automatically via YouTube Music Up Next",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ytmusic::{YtmLyricLine, YtmLyrics, YtmShelf, YtmTrack};
    use crate::config::Config;
    use crate::state::{ActiveOverlay, Category};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_music_workspace_renders_spotify_grid_and_album_popup_and_settings() {
        let mut st = AppState::new(Config::default());
        st.active_category = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Music)
            .unwrap();
        st.music.home_page = Some(YtmPage {
            shelves: vec![YtmShelf {
                title: "Listen again".to_string(),
                items: vec![
                    YtmShelfItem {
                        kind: YtmItemKind::Song,
                        title: "Starlight".to_string(),
                        subtitle: "Muse".to_string(),
                        video_id: Some("Pgum6OT_VH8".to_string()),
                        browse_id: None,
                        playlist_id: Some("RDAMVMPgum6OT_VH8".to_string()),
                        params: None,
                        thumb_url: String::new(),
                        duration_secs: Some(240),
                    },
                    YtmShelfItem {
                        kind: YtmItemKind::Album,
                        title: "Black Holes and Revelations".to_string(),
                        subtitle: "Album • Muse".to_string(),
                        video_id: None,
                        browse_id: Some("MPREb_123".to_string()),
                        playlist_id: None,
                        params: None,
                        thumb_url: String::new(),
                        duration_secs: None,
                    },
                ],
            }],
            ..Default::default()
        });
        st.music.queue = vec![YtmTrack {
            video_id: "Pgum6OT_VH8".to_string(),
            title: "Starlight".to_string(),
            artist: "Muse".to_string(),
            artist_browse_id: None,
            album: Some("Black Holes".to_string()),
            album_browse_id: Some("MPREb_123".to_string()),
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

        let backend = TestBackend::new(140, 42);
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
        assert!(text.contains("Black Holes"));
        assert!(text.contains("Lyrics (Sync)"));
        assert!(text.contains("Hold you in my arms"));
        assert!(text.contains("Your Library"));
        assert!(text.contains("Music Settings"));

        // Album popup modal renders cleanly with tracks
        st.overlay = ActiveOverlay::MusicAlbum;
        st.music.album_popup.title = "Black Holes and Revelations".to_string();
        st.music.album_popup.browse_id = "MPREb_123".to_string();
        st.music.album_popup.page = Some(YtmPage {
            header_title: Some("Black Holes and Revelations".to_string()),
            header_artist: Some("Muse".to_string()),
            header_year: Some("2006".to_string()),
            tracks: st.music.queue.clone(),
            ..Default::default()
        });
        term.draw(|f| crate::ui::render_app(f, &mut st)).unwrap();
        let pop_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(pop_text.contains("Play All"));
        assert!(pop_text.contains("2006"));

        // Dedicated YouTube Music Settings modal renders separately from video Settings
        st.overlay = ActiveOverlay::MusicSettings;
        term.draw(|f| crate::ui::render_app(f, &mut st)).unwrap();
        let set_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(set_text.contains("Anti-Clipping Limiter"));
        assert!(set_text.contains("Volume Normalization"));
    }
}
