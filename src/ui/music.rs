//! Spotify-style YouTube Music workspace UI:
//! - 4-Zone Desktop Layout:
//!   1. Left Sidebar (`Home`, `Search`, `Charts`, `Liked`, `Library`, `Music Settings`, & quick Library list)
//!   2. Main Stage (`1:1` square cover card shelves grid, Search hero + filter bar, or compact Track table)
//!   3. Collapsible Right Drawer (`d` key / bottom button / Music Settings default: large `1:1` art, `Up Next` queue & live synced `Lyrics`)
//!   4. Full-width Bottom Player Bar (mini `1:1` cover, clickable album/artist -> opens Album Popup, centered transport + seek bar, volume & drawer/video toggles)
//! - Centered Spotify Album / Playlist / Artist Detail Popup (`ActiveOverlay::MusicAlbum`)
//! - Dedicated YouTube Music Audio & UI Settings Modal (`ActiveOverlay::MusicSettings`)

use crate::api::ytmusic::{YtmItemKind, YtmPage, YtmSearchFilter, YtmShelfItem, YtmTrack};
use crate::config::{AudioEqPreset, MusicSettingsConfig, EQ_BAND_LABELS};
use crate::db::cache::format_duration_secs;
use crate::state::{
    AppState, MusicEqHit, MusicHit, MusicSettingsHit, ThumbPriority, YtmAlbumPopupHit,
    YtmArtistSectionTab, YtmRepeatMode, YtmRightPaneTab, YtmSection,
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

/// Queue a square cover image (`ytm:...`) if missing from cache, accepting canonical or bare keys cleanly.
fn queue_music_cover(state: &mut AppState, key: &str, url: &str, priority: ThumbPriority) {
    if key.is_empty() || url.is_empty() {
        return;
    }
    if key.starts_with("ytm:") {
        state.queue_thumb_with_priority(key, url, priority);
    } else {
        let full_key = format!("ytm:{key}");
        state.queue_thumb_with_priority(&full_key, url, priority);
    }
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

    let bar_h: u16 = if area.height >= 26 { 6 } else { 5 };
    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(bar_h)])
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
        .style(Style::default().bg(Color::Reset));
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
                .bg(Color::Reset)
        };
        let text = truncate_str(&format!(" {icon} {label}"), inner.width as usize);
        frame.render_widget(Paragraph::new(text).style(st), row_rect);
        y += 1;
    }

    // Dedicated Music EQ button inside sidebar
    if y < inner.y + inner.height {
        let eq_rect = Rect::new(inner.x, y, inner.width, 1);
        state
            .music
            .hitboxes
            .push((eq_rect, MusicHit::OpenMusicEq));
        let st = Style::default()
            .fg(spotify_green)
            .bg(Color::Reset)
            .add_modifier(Modifier::BOLD);
        let txt = truncate_str(
            &format!(" ≡ Equalizer ({})", state.config.music.eq_badge_label()),
            inner.width as usize,
        );
        frame.render_widget(Paragraph::new(txt).style(st), eq_rect);
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
            .bg(Color::Reset)
            .add_modifier(Modifier::BOLD);
        let txt = truncate_str(" [#] Music Settings", inner.width as usize);
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
        .style(Style::default().bg(Color::Reset));
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
            " · Double-Click Album/Artist: Open · Click Song: Play · [e] EQ · [s] Settings · [d] Drawer",
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
                .bg(Color::Reset)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(theme.text_dim)
                .bg(Color::Reset)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ⌕ Search: ",
                    Style::default()
                        .fg(Color::Rgb(30, 215, 96))
                        .bg(Color::Reset)
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
                    .bg(Color::Reset)
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
                theme.border_idle
            };
            let card_bg = Color::Reset;

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

            if !key.is_empty() && !rect_intersects_optional(art_rect, modal_rect) {
                render_thumbnail_cover(
                    frame,
                    art_rect,
                    &key,
                    state,
                    item.kind.badge(),
                );
            } else {
                frame.render_widget(
                    Paragraph::new(format!("\n◈ {}", item.kind.badge()))
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
        let bg = Color::Reset;
        let lead = if is_playing {
            "▶ ".to_string()
        } else if is_sel {
            "▸ ".to_string()
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
                    .fg(if is_playing {
                        spotify_green
                    } else if is_sel {
                        theme.accent
                    } else {
                        theme.text_dim
                    })
                    .bg(bg),
            ),
            Span::styled(
                format!("{t_str:<width$}", width = t_budget.max(8)),
                Style::default()
                    .fg(if is_playing {
                        spotify_green
                    } else if is_sel {
                        theme.accent
                    } else {
                        Color::White
                    })
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
    let preview = state.music.preview_item.clone();
    let is_preview = preview.is_some();

    let (art_id, art_url, title, artist, album_opt, preview_kind) = if let Some(ref p) = preview {
        (
            p.thumb_cache_key(),
            p.thumb_url.clone(),
            p.title.clone(),
            if p.subtitle.is_empty() {
                p.kind.badge().to_string()
            } else {
                p.subtitle.clone()
            },
            p.duration_secs.map(|d| format!("Duration: {}", format_duration_secs(d))),
            Some(p.kind),
        )
    } else if let Some(ref tr) = cur_track {
        (
            tr.thumb_cache_key(),
            tr.effective_thumb_url(),
            tr.title.clone(),
            tr.artist.clone(),
            tr.album.clone(),
            Some(tr.kind),
        )
    } else {
        let fallback = state.music.selectable_items().get(state.music.cursor).cloned();
        if let Some(ref p) = fallback {
            (
                p.thumb_cache_key(),
                p.thumb_url.clone(),
                p.title.clone(),
                p.subtitle.clone(),
                p.duration_secs.map(|d| format!("Duration: {}", format_duration_secs(d))),
                Some(p.kind),
            )
        } else {
            (
                String::new(),
                String::new(),
                "Nothing Playing".to_string(),
                "Click to inspect · 2x-click to play".to_string(),
                None,
                None,
            )
        }
    };

    if !art_id.is_empty() && !art_url.is_empty() {
        state.queue_thumb_with_priority(&art_id, &art_url, ThumbPriority::Focused);
    }
    let pal = if !art_id.is_empty() {
        state.active_palette_for(Some(&art_id)).clone()
    } else {
        state.base_theme().clone()
    };

    let drawer_title = if is_preview {
        " Preview Inspector "
    } else {
        " Now Playing "
    };
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary))
        .title(Span::styled(
            drawer_title,
            Style::default()
                .fg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Reset));
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
            Constraint::Length(if is_preview { 4 } else { 3 }),
            Constraint::Min(4),
        ])
        .split(inner);

    if !art_id.is_empty() && !rect_intersects_optional(parts[0], modal_rect) {
        render_thumbnail_cover(frame, parts[0], &art_id, state, "ART");
    } else {
        frame.render_widget(
            Paragraph::new("\n◈\nYTM")
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.primary).bg(Color::Reset)),
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
    if is_preview {
        let kind = preview_kind.unwrap_or(YtmItemKind::Song);
        let play_lbl = match kind {
            YtmItemKind::Video => " [▸ Watch Video] ",
            YtmItemKind::Album | YtmItemKind::Playlist | YtmItemKind::Artist => " [◈ Open] ",
            _ => " [▸ Play Now] ",
        };
        let pw = play_lbl.chars().count() as u16;
        let btn_y = parts[1].y + 3;
        let mut bx = parts[1].x;
        let mut btn_spans = Vec::new();
        if bx + pw <= parts[1].x + parts[1].width {
            state
                .music
                .hitboxes
                .push((Rect::new(bx, btn_y, pw, 1), MusicHit::PreviewPlay));
            btn_spans.push(Span::styled(
                play_lbl,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(30, 215, 96))
                    .add_modifier(Modifier::BOLD),
            ));
            bx += pw + 1;
        }
        if cur_track.is_some() {
            let np_lbl = " [Live] ";
            let nw = np_lbl.chars().count() as u16;
            if bx + nw <= parts[1].x + parts[1].width {
                state
                    .music
                    .hitboxes
                    .push((Rect::new(bx, btn_y, nw, 1), MusicHit::ShowNowPlaying));
                btn_spans.push(Span::raw(" "));
                btn_spans.push(Span::styled(
                    np_lbl,
                    Style::default()
                        .fg(pal.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("[{}] 2x-click to run", kind.badge()),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
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
                Line::from(btn_spans),
            ]),
            parts[1],
        );
    } else {
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
    }

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
            .bg(Color::Reset)
    };
    let lyr_st = if state.music.right_tab == YtmRightPaneTab::Lyrics {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(30, 215, 96))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.text_dim)
            .bg(Color::Reset)
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
    let bar_bg = Color::Reset;
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle))
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
            Constraint::Percentage(36),
            Constraint::Percentage(32),
        ])
        .split(inner);

    // 1. Left section: Enlarged 1:1 cover + Title + Artist + Album + Live Status
    let cur = state.music.current_track().cloned();
    let left_area = cols[0];
    let cover_w: u16 = if left_area.width >= 24 {
        (inner.height * 2 + 2).clamp(8, 12)
    } else if left_area.width >= 18 {
        8
    } else {
        0
    };
    let (mini_cover_rect, meta_rect) = if cover_w > 0 {
        let lsplit = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(cover_w), Constraint::Min(8)])
            .split(left_area);
        let c = Rect::new(
            lsplit[0].x + 1,
            lsplit[0].y,
            lsplit[0].width.saturating_sub(2),
            lsplit[0].height,
        );
        (Some(c), lsplit[1])
    } else {
        (None, left_area)
    };

    if let (Some(c_rect), Some(ref tr)) = (mini_cover_rect, &cur) {
        let key = tr.thumb_cache_key();
        let eff_url = tr.effective_thumb_url();
        if !eff_url.is_empty() {
            state.queue_thumb_with_priority(&key, &eff_url, ThumbPriority::Focused);
        }
        if !rect_intersects_optional(c_rect, modal_rect) {
            render_thumbnail_cover(frame, c_rect, &key, state, "◈");
        }
        state
            .music
            .hitboxes
            .push((c_rect, MusicHit::OpenCurrentAlbum));
    } else if let Some(c_rect) = mini_cover_rect {
        frame.render_widget(
            Paragraph::new(if c_rect.height >= 3 { "\n◈ YTM" } else { "◈" })
                .alignment(Alignment::Center)
                .style(Style::default().fg(spotify_green).bg(bar_bg)),
            c_rect,
        );
    }

    if let Some(ref tr) = cur {
        let liked = state.music.liked_ids.contains(&tr.video_id);
        let heart_lbl = if liked { " ♥" } else { " ♡" };
        let w = meta_rect.width.saturating_sub(4) as usize;
        let t_str = marquee_window(&tr.title, w.max(6), state.ticker_step);
        let artist_str = if tr.artist.is_empty() {
            "YouTube Music".to_string()
        } else {
            tr.artist.clone()
        };
        let album_str = tr
            .album
            .as_deref()
            .map(|a| format!("{a} [Open]"))
            .unwrap_or_else(|| "Single / Mix".to_string());

        let heart_x = meta_rect.x + (t_str.chars().count() as u16 + 1).min(meta_rect.width.saturating_sub(2));
        state
            .music
            .hitboxes
            .push((Rect::new(heart_x, meta_rect.y, 3, 1), MusicHit::ToggleLike));
        if (tr.album_browse_id.is_some() || state.music.last_opened_collection.is_some())
            && meta_rect.height >= 2
        {
            let album_row = if meta_rect.height >= 3 {
                meta_rect.y + 2
            } else {
                meta_rect.y + 1
            };
            state.music.hitboxes.push((
                Rect::new(meta_rect.x, album_row, meta_rect.width, 1),
                MusicHit::OpenCurrentAlbum,
            ));
        }

        let mut meta_lines = vec![
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
                format!(" {}", truncate_str(&artist_str, w)),
                Style::default().fg(theme.text_dim).bg(bar_bg),
            )),
        ];
        if meta_rect.height >= 3 {
            meta_lines.push(Line::from(Span::styled(
                format!(" {}", truncate_str(&album_str, w)),
                Style::default().fg(theme.accent).bg(bar_bg),
            )));
        }
        if meta_rect.height >= 4 {
            let state_tag = if !state.live_playback.paused && state.has_live_ipc() {
                "▸ PLAYING"
            } else if state.has_live_ipc() {
                "|| PAUSED"
            } else {
                "◆ READY"
            };
            let q_info = format!(
                " {state_tag} · Track {}/{}",
                state.music.queue_index + 1,
                state.music.queue.len().max(1)
            );
            meta_lines.push(Line::from(Span::styled(
                truncate_str(&q_info, w),
                Style::default().fg(spotify_green).bg(bar_bg),
            )));
        }
        frame.render_widget(
            Paragraph::new(meta_lines).style(Style::default().bg(bar_bg)),
            meta_rect,
        );
    } else {
        let mut idle_lines = vec![Line::from(Span::styled(
            " ◈ Ready to Listen",
            Style::default()
                .fg(Color::White)
                .bg(bar_bg)
                .add_modifier(Modifier::BOLD),
        ))];
        if meta_rect.height >= 2 {
            idle_lines.push(Line::from(Span::styled(
                " 1x Click: Inspect · 2x Click: Play",
                Style::default().fg(theme.text_dim).bg(bar_bg),
            )));
        }
        if meta_rect.height >= 3 {
            idle_lines.push(Line::from(Span::styled(
                " Videos open in mpv Window",
                Style::default().fg(theme.accent).bg(bar_bg),
            )));
        }
        frame.render_widget(Paragraph::new(idle_lines), meta_rect);
    }

    // 2. Center section: Transport row + Seek bar + Live Audio Spec / Hint rows (Zero Dead Space)
    let center = cols[1];
    let c_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(center);

    let play_lbl = if cur.is_some() && !state.live_playback.paused && state.has_live_ipc() {
        " || "
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
    if c_rows[2].height >= 1 {
        let eq_tag = state.config.music.eq_badge_label();
        let spec_line = if let Some(ref tr) = cur {
            let mode_s = if tr.kind == YtmItemKind::Video || state.music.video_mode {
                "Video+Audio"
            } else {
                "Opus 48kHz 32f"
            };
            format!(
                "{mode_s} · {} · EQ:{eq_tag}",
                state.config.music.channel_mode.label()
            )
        } else {
            format!(
                "PipeWire 32f Float · EQ:{eq_tag} · {}",
                state.config.music.audio_quality.label()
            )
        };
        frame.render_widget(
            Paragraph::new(truncate_str(&spec_line, center.width as usize))
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accent).bg(bar_bg)),
            c_rows[2],
        );
    }
    if c_rows[3].height >= 1 {
        frame.render_widget(
            Paragraph::new(truncate_str(
                "[Space] Pause  [[/]] Seek ±10s  [</>] Prev/Next",
                center.width as usize,
            ))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim).bg(bar_bg)),
            c_rows[3],
        );
    }

    // 3. Right section: Balanced 4-row Deck (Controls, Volume Bar, Audio Telemetry, Hotkeys)
    let right_area = cols[2];
    let vol_pct = state.live_playback.volume.clamp(0, 100);
    let vol_lbl = if state.live_playback.muted {
        "MUTE".to_string()
    } else {
        format!("{vol_pct}%")
    };
    let eq_badge = state.config.music.eq_badge_label();
    let eq_pill_lbl = format!(" [EQ:{eq_badge}] ");
    let eq_pill_col = if !state.config.music.eq_bypass
        && (state.config.music.eq_preset != AudioEqPreset::Flat
            || state.config.music.active_saved_eq.is_some())
    {
        spotify_green
    } else {
        theme.accent
    };
    let r_btns: Vec<(String, MusicHit, Color)> = vec![
        (
            " [Drawer] ".to_string(),
            MusicHit::ToggleRightDrawer,
            if state.music.right_drawer_open {
                spotify_green
            } else {
                theme.text_dim
            },
        ),
        (
            if state.music.video_mode {
                " [Vid] ".to_string()
            } else {
                " [Aud] ".to_string()
            },
            MusicHit::ToggleVideoMode,
            if state.music.video_mode {
                Color::Rgb(210, 120, 255)
            } else {
                spotify_green
            },
        ),
        (" -".to_string(), MusicHit::VolumeDelta(-5), theme.text_dim),
        (" Vol".to_string(), MusicHit::ToggleMute, Color::White),
        ("+ ".to_string(), MusicHit::VolumeDelta(5), theme.text_dim),
        (eq_pill_lbl, MusicHit::OpenMusicEq, eq_pill_col),
        (" [#] ".to_string(), MusicHit::OpenMusicSettings, theme.accent),
    ];
    let mut kept: Vec<(String, MusicHit, Color, u16)> = Vec::new();
    let max_w = right_area.width.saturating_sub(1);
    let mut actual_w: u16 = 0;
    for (lbl, hit, col) in r_btns {
        let w = lbl.chars().count() as u16;
        if actual_w + w <= max_w {
            actual_w += w;
            kept.push((lbl, hit, col, w));
        }
    }
    let lead_pad = max_w.saturating_sub(actual_w);
    let mut rx = right_area.x + lead_pad;
    let mut rspans = if lead_pad > 0 {
        vec![Span::raw(" ".repeat(lead_pad as usize))]
    } else {
        Vec::new()
    };
    for (lbl, hit, col, w) in kept {
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
    frame.render_widget(
        Paragraph::new(Line::from(rspans)),
        Rect::new(right_area.x, right_area.y, right_area.width.saturating_sub(1), 1),
    );
    if right_area.height >= 2 {
        let eff_preamp = crate::api::player::effective_music_preamp_db(&state.config.music);
        let info = format!(
            "Vol: {vol_lbl} · EQ:{eq_badge} ({:+}dB) · {}",
            eff_preamp,
            if state.config.music.anti_clip_limiter {
                "Lossless Safe"
            } else {
                "Direct"
            }
        );
        frame.render_widget(
            Paragraph::new(truncate_str(&info, right_area.width.saturating_sub(1) as usize))
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme.text_dim).bg(bar_bg)),
            Rect::new(right_area.x, right_area.y + 1, right_area.width.saturating_sub(1), 1),
        );
    }
    if right_area.height >= 3 {
        let bar_w = 10usize;
        let filled_v = if state.live_playback.muted {
            0
        } else {
            ((vol_pct as usize * bar_w) / 100).min(bar_w)
        };
        let vol_bar = format!(
            "Output [{}{}] {:>4}",
            "█".repeat(filled_v),
            "░".repeat(bar_w - filled_v),
            vol_lbl
        );
        frame.render_widget(
            Paragraph::new(truncate_str(&vol_bar, right_area.width.saturating_sub(1) as usize))
                .alignment(Alignment::Right)
                .style(Style::default().fg(spotify_green).bg(bar_bg)),
            Rect::new(right_area.x, right_area.y + 2, right_area.width.saturating_sub(1), 1),
        );
    }
    if right_area.height >= 4 {
        frame.render_widget(
            Paragraph::new(truncate_str(
                "[e] EQ Studio · [s] Audio Settings · [d] Drawer",
                right_area.width.saturating_sub(1) as usize,
            ))
            .alignment(Alignment::Right)
            .style(Style::default().fg(theme.text_dim).bg(bar_bg)),
            Rect::new(right_area.x, right_area.y + 3, right_area.width.saturating_sub(1), 1),
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
            Paragraph::new("\nQueue empty — double-click album or play song.")
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
        let bg = Color::Reset;
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
        let prefix = if is_active { "▸ " } else { "  " };
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

/// Interactive row inside the unified Album / Playlist / Artist popup body.
#[derive(Debug, Clone)]
pub enum PopupNavRow {
    SectionHeader {
        title: String,
        more_shelf_idx: Option<usize>,
    },
    Track {
        sel_idx: usize,
        track_idx: usize,
    },
    ShelfItem {
        sel_idx: usize,
        flat_item_idx: usize,
        item: YtmShelfItem,
    },
    AboutLine {
        text: String,
    },
}

fn shelf_matches_artist_tab(title: &str, tab: YtmArtistSectionTab) -> bool {
    let low = title.to_ascii_lowercase();
    match tab {
        YtmArtistSectionTab::All => true,
        YtmArtistSectionTab::TopSongs => low.contains("song") || low.contains("popular"),
        YtmArtistSectionTab::Albums => low.contains("album"),
        YtmArtistSectionTab::Singles => low.contains("single") || low.contains("ep"),
        YtmArtistSectionTab::Videos => low.contains("video"),
        YtmArtistSectionTab::About => false,
    }
}

fn wrap_plain_text(text: &str, width: usize) -> Vec<String> {
    let w = width.max(20);
    let mut lines = Vec::new();
    for para in text.lines() {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in trimmed.split_whitespace() {
            if cur.is_empty() {
                cur.push_str(word);
            } else if cur.chars().count() + 1 + word.chars().count() <= w {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(cur);
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

/// Build the unified, scrollable row list for an Album / Playlist / Artist popup so rendering,
/// mouse hit-testing, and keyboard navigation stay 100% synchronized.
pub fn popup_nav_rows(
    page: &YtmPage,
    tab: YtmArtistSectionTab,
    about_width: usize,
) -> (Vec<PopupNavRow>, usize) {
    let mut rows = Vec::new();
    let mut sel_count = 0usize;

    let track_vids: std::collections::HashSet<&str> =
        page.tracks.iter().map(|t| t.video_id.as_str()).collect();

    if page.is_artist_profile || !page.shelves.is_empty() {
        // 1. Top Songs / Tracks table (when visible for current tab)
        if !page.tracks.is_empty() && matches!(tab, YtmArtistSectionTab::All | YtmArtistSectionTab::TopSongs)
        {
            if page.is_artist_profile {
                let more_idx = page.shelves.iter().position(|s| {
                    s.more_browse_id.is_some()
                        && (s.title.to_ascii_lowercase().contains("song")
                            || s.items
                                .iter()
                                .all(|it| it.video_id.as_deref().is_some_and(|v| track_vids.contains(v))))
                });
                rows.push(PopupNavRow::SectionHeader {
                    title: "TOP SONGS".to_string(),
                    more_shelf_idx: more_idx,
                });
            }
            for (t_idx, _) in page.tracks.iter().enumerate() {
                let sel_idx = sel_count;
                sel_count += 1;
                rows.push(PopupNavRow::Track { sel_idx, track_idx: t_idx });
            }
        }

        // 2. Collection & Discography Shelves (Albums, Singles, Videos, Playlists, Fans might also like)
        let mut flat_item_idx = 0usize;
        for (s_idx, shelf) in page.shelves.iter().enumerate() {
            let count = shelf.items.len();
            let is_duplicate_tracks_shelf = !page.tracks.is_empty()
                && !shelf.items.is_empty()
                && shelf.items.iter().all(|it| {
                    it.video_id.as_deref().is_some_and(|v| track_vids.contains(v))
                })
                && !shelf.title.to_ascii_lowercase().contains("video");

            if is_duplicate_tracks_shelf || !shelf_matches_artist_tab(&shelf.title, tab) {
                flat_item_idx += count;
                continue;
            }
            if shelf.items.is_empty() {
                continue;
            }

            rows.push(PopupNavRow::SectionHeader {
                title: shelf.title.to_ascii_uppercase(),
                more_shelf_idx: shelf.more_browse_id.is_some().then_some(s_idx),
            });
            for it in &shelf.items {
                let sel_idx = sel_count;
                sel_count += 1;
                rows.push(PopupNavRow::ShelfItem {
                    sel_idx,
                    flat_item_idx,
                    item: it.clone(),
                });
                flat_item_idx += 1;
            }
        }

        // 3. About Artist Bio section
        if page.is_artist_profile && matches!(tab, YtmArtistSectionTab::All | YtmArtistSectionTab::About)
        {
            if let Some(ref desc) = page.header_description {
                rows.push(PopupNavRow::SectionHeader {
                    title: "ABOUT ARTIST".to_string(),
                    more_shelf_idx: None,
                });
                for line in wrap_plain_text(desc, about_width) {
                    rows.push(PopupNavRow::AboutLine { text: line });
                }
            }
        }
    } else {
        for (t_idx, _) in page.tracks.iter().enumerate() {
            let sel_idx = sel_count;
            sel_count += 1;
            rows.push(PopupNavRow::Track { sel_idx, track_idx: t_idx });
        }
    }

    (rows, sel_count)
}

/// Resolve primary activation / queue / like target for a popup row cursor.
pub fn popup_target_for_cursor(
    page: &YtmPage,
    tab: YtmArtistSectionTab,
    sel_cursor: usize,
) -> Option<(YtmAlbumPopupHit, Option<usize>)> {
    let (rows, _) = popup_nav_rows(page, tab, 80);
    for row in rows {
        match row {
            PopupNavRow::Track { sel_idx, track_idx } if sel_idx == sel_cursor => {
                return Some((YtmAlbumPopupHit::PlayTrack(track_idx), Some(track_idx)));
            }
            PopupNavRow::ShelfItem {
                sel_idx,
                flat_item_idx,
                ..
            } if sel_idx == sel_cursor => {
                return Some((YtmAlbumPopupHit::OpenItem(flat_item_idx), None));
            }
            _ => {}
        }
    }
    None
}

/// Centered Transparent Spotify-style Album / Playlist / Artist Detail Popup Modal.
pub fn render_album_popup_modal(frame: &mut Frame, state: &mut AppState) {
    let Some(modal_area) =
        crate::state::active_modal_rect(frame.area(), crate::state::ActiveOverlay::MusicAlbum)
    else {
        return;
    };
    state.music.album_popup.hitboxes.clear();

    frame.render_widget(Clear, modal_area);

    let page_opt = state.music.album_popup.page.clone();
    let active_vid = state.active_playback_video_id.clone();

    // Dynamically pick hero artwork:
    // For Albums/Playlists, show the active playing song's cover (updating live as tracks change),
    // or preview the highlighted track's cover, falling back cleanly to collection cover art.
    let playing_track_in_collection = page_opt.as_ref().and_then(|p| {
        active_vid.as_deref().and_then(|vid| {
            p.tracks
                .iter()
                .enumerate()
                .find(|(_, t)| t.video_id == vid)
                .map(|(i, t)| (i, t.clone()))
        })
    });
    let highlighted_track = page_opt.as_ref().and_then(|p| {
        popup_target_for_cursor(p, state.music.album_popup.section_filter, state.music.album_popup.cursor)
            .and_then(|(_, opt_tidx)| opt_tidx.and_then(|ti| p.tracks.get(ti).cloned()))
    });

    let (art_key, art_url, dynamic_badge) = if let Some(ref p) = page_opt {
        if !p.is_artist_profile {
            if let Some((p_idx, ref tr)) = playing_track_in_collection {
                let k = tr.thumb_cache_key();
                let u = tr.effective_thumb_url();
                let fallback_u = p.header_thumb.clone().unwrap_or_default();
                let final_u = if u.is_empty() { fallback_u } else { u };
                (k, final_u, Some((p_idx, tr.clone())))
            } else if let Some(ref tr) = highlighted_track {
                let k = tr.thumb_cache_key();
                let u = tr.effective_thumb_url();
                if !u.is_empty() {
                    (k, u, None)
                } else {
                    (
                        format!("ytm:b:{}", state.music.album_popup.browse_id),
                        p.header_thumb.clone().unwrap_or_default(),
                        None,
                    )
                }
            } else {
                (
                    format!("ytm:b:{}", state.music.album_popup.browse_id),
                    p.header_thumb.clone().unwrap_or_default(),
                    None,
                )
            }
        } else {
            (
                format!("ytm:b:{}", state.music.album_popup.browse_id),
                p.header_thumb.clone().unwrap_or_default(),
                None,
            )
        }
    } else {
        (
            format!("ytm:b:{}", state.music.album_popup.browse_id),
            String::new(),
            None,
        )
    };

    if !art_key.is_empty() && !art_url.is_empty() {
        state.queue_thumb_with_priority(&art_key, &art_url, ThumbPriority::Focused);
    }
    let pal = state.active_palette_for(Some(&art_key)).clone();
    let spotify_green = Color::Rgb(30, 215, 96);

    let title_hdr = if state.music.album_popup.title.is_empty() {
        " Collection ".to_string()
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
        .style(Style::default().bg(Color::Reset));
    let inner = outer_blk.inner(modal_area);
    frame.render_widget(outer_blk, modal_area);

    if inner.width < 32 || inner.height < 12 {
        return;
    }

    let is_artist = page_opt.as_ref().is_some_and(|p| p.is_artist_profile);
    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(10),
    ];
    if is_artist {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(4));
    constraints.push(Constraint::Length(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
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
    let dr_lbl = if state.music.album_popup.drawer_open {
        " [d Drawer:ON] "
    } else {
        " [d Drawer:OFF] "
    };
    let close_lbl = " [x Close] ";
    let dw = dr_lbl.chars().count() as u16;
    let cw = close_lbl.chars().count() as u16;
    let cx = nav.x + nav.width.saturating_sub(cw + 1);
    let dx = cx.saturating_sub(dw + 1);
    state
        .music
        .album_popup
        .hitboxes
        .push((Rect::new(dx, nav.y, dw, 1), YtmAlbumPopupHit::ToggleDrawer));
    state
        .music
        .album_popup
        .hitboxes
        .push((Rect::new(cx, nav.y, cw, 1), YtmAlbumPopupHit::Close));
    if dx > nx {
        nav_spans.push(Span::raw(" ".repeat((dx - nx) as usize)));
    }
    nav_spans.push(Span::styled(
        dr_lbl,
        Style::default()
            .fg(if state.music.album_popup.drawer_open {
                Color::Black
            } else {
                pal.text_dim
            })
            .bg(if state.music.album_popup.drawer_open {
                spotify_green
            } else {
                Color::Reset
            })
            .add_modifier(Modifier::BOLD),
    ));
    nav_spans.push(Span::raw(" "));
    nav_spans.push(Span::styled(
        close_lbl,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(190, 80, 95))
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(Line::from(nav_spans)), nav);

    // 2. Hero Header (Dynamic 1:1 Cover Art + Rich Metadata + Actions)
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
    if !art_url.is_empty() {
        render_thumbnail_cover(frame, cov_rect, &art_key, state, if is_artist { "Artist" } else { "Cover" });
    } else {
        frame.render_widget(
            Paragraph::new(if is_artist { "\n\n◈ ARTIST" } else { "\n\n◈ COVER" })
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.primary).bg(Color::Reset)),
            cov_rect,
        );
    }

    let info_rect = hsplit[1];
    if let Some(ref p) = page_opt {
        let title = p
            .header_title
            .as_deref()
            .unwrap_or(&state.music.album_popup.title);
        let info_w = info_rect.width.saturating_sub(2) as usize;
        let btn_y = info_rect.y + 6.min(info_rect.height.saturating_sub(2));

        let (kicker, line3, line4) = if p.is_artist_profile {
            let subs = p
                .header_subscribers
                .as_deref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default();
            let bio_preview = p
                .header_description
                .as_deref()
                .or(p.header_subtitle.as_deref())
                .unwrap_or("");
            let stats = format!(
                "{} Top Songs · {} Sections",
                p.tracks.len(),
                p.shelves.len()
            );
            (
                format!(" ARTIST PROFILE{subs}"),
                truncate_str(bio_preview, info_w),
                stats,
            )
        } else {
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
            let status_line = if let Some((idx, ref playing_tr)) = dynamic_badge {
                format!(
                    "▶ Playing #{}: {} — {}",
                    idx + 1,
                    playing_tr.title,
                    playing_tr.artist
                )
            } else {
                format!("{} tracks loaded", p.tracks.len())
            };
            (
                " ALBUM / PLAYLIST · DYNAMIC SONG COVER".to_string(),
                meta_parts.join(" · "),
                status_line,
            )
        };

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    truncate_str(&kicker, info_w),
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
                    format!(" {}", truncate_str(&line3, info_w)),
                    Style::default().fg(pal.text_dim),
                )),
                Line::from(Span::styled(
                    format!(" {}", truncate_str(&line4, info_w)),
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
                if p.is_artist_profile {
                    " ▶ Play Top Songs "
                } else {
                    " ▶ Play All "
                },
                YtmAlbumPopupHit::PlayAll,
                Color::Black,
                spotify_green,
            ),
            (
                " [~] Shuffle ",
                YtmAlbumPopupHit::ShuffleAll,
                Color::White,
                Color::Rgb(50, 58, 76),
            ),
        ];
        if p.header_radio_playlist_id.is_some() {
            actions.push((
                " ((·)) Start Radio ",
                YtmAlbumPopupHit::StartRadio,
                Color::Black,
                pal.accent,
            ));
        } else if p.header_artist_browse_id.is_some() && !p.is_artist_profile {
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

    // Artist section tabs row if artist profile
    let mut body_idx = 2usize;
    if is_artist {
        let tab_bar = chunks[body_idx];
        body_idx += 1;
        let mut tx = tab_bar.x + 1;
        let mut tspans = vec![Span::raw(" ")];
        for tab in YtmArtistSectionTab::ALL {
            let active = state.music.album_popup.section_filter == tab;
            let lbl = format!(" {} ", tab.label());
            let w = lbl.chars().count() as u16;
            if tx + w <= tab_bar.x + tab_bar.width {
                state
                    .music
                    .album_popup
                    .hitboxes
                    .push((Rect::new(tx, tab_bar.y, w, 1), YtmAlbumPopupHit::SelectArtistTab(tab)));
                let st = if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(spotify_green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(pal.text_dim).bg(Color::Reset)
                };
                tspans.push(Span::styled(lbl, st));
                tspans.push(Span::raw(" "));
                tx += w + 1;
            }
        }
        frame.render_widget(Paragraph::new(Line::from(tspans)), tab_bar);
    }

    // 3. Body: Left Unified Tracklist/Shelves/Bio + Collapsible Right Preview Inspector Drawer
    let body_row = chunks[body_idx];
    let footer = chunks[body_idx + 1];
    let show_inspector = state.music.album_popup.drawer_open && body_row.width >= 72 && page_opt.is_some();
    let (body, inspector_opt) = if show_inspector {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body_row);
        (cols[0], Some(cols[1]))
    } else {
        (body_row, None)
    };

    if state.music.album_popup.loading {
        frame.render_widget(
            Paragraph::new("\nLoading details from YouTube Music…")
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
        let (rows, sel_count) = popup_nav_rows(
            p,
            state.music.album_popup.section_filter,
            body.width.saturating_sub(4) as usize,
        );
        if sel_count > 0 && state.music.album_popup.cursor >= sel_count {
            state.music.album_popup.cursor = sel_count - 1;
        } else if sel_count == 0 {
            state.music.album_popup.cursor = 0;
        }

        let vis = body.height as usize;
        // Locate line offset of active selectable cursor
        let cursor_row_pos = rows
            .iter()
            .position(|row| match row {
                PopupNavRow::Track { sel_idx, .. } | PopupNavRow::ShelfItem { sel_idx, .. } => {
                    *sel_idx == state.music.album_popup.cursor
                }
                _ => false,
            })
            .unwrap_or(0);

        if sel_count > 0 {
            if cursor_row_pos < state.music.album_popup.scroll {
                state.music.album_popup.scroll = cursor_row_pos;
            } else if vis > 0 && cursor_row_pos >= state.music.album_popup.scroll + vis {
                state.music.album_popup.scroll = cursor_row_pos + 1 - vis;
            }
        } else if rows.len() > vis {
            state.music.album_popup.scroll =
                state.music.album_popup.scroll.min(rows.len() - vis);
        } else {
            state.music.album_popup.scroll = 0;
        }

        let mut inspected_track: Option<YtmTrack> = None;
        let mut inspected_item: Option<YtmShelfItem> = None;
        for row in &rows {
            match row {
                PopupNavRow::Track { sel_idx, track_idx }
                    if *sel_idx == state.music.album_popup.cursor =>
                {
                    inspected_track = p.tracks.get(*track_idx).cloned();
                    break;
                }
                PopupNavRow::ShelfItem { sel_idx, item, .. }
                    if *sel_idx == state.music.album_popup.cursor =>
                {
                    inspected_item = Some(item.clone());
                    break;
                }
                _ => {}
            }
        }

        for (vi, row) in rows
            .iter()
            .skip(state.music.album_popup.scroll)
            .take(vis)
            .enumerate()
        {
            let row_y = body.y + vi as u16;
            let r = Rect::new(body.x + 1, row_y, body.width.saturating_sub(2), 1);

            match row {
                PopupNavRow::SectionHeader {
                    title,
                    more_shelf_idx,
                } => {
                    let mut spans = vec![Span::styled(
                        format!(" {title}"),
                        Style::default()
                            .fg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    )];
                    if let Some(s_idx) = more_shelf_idx {
                        let m_lbl = " [More >] ";
                        let mw = m_lbl.chars().count() as u16;
                        let mx = r.x + r.width.saturating_sub(mw + 1);
                        state
                            .music
                            .album_popup
                            .hitboxes
                            .push((Rect::new(mx, row_y, mw, 1), YtmAlbumPopupHit::ExpandShelf(*s_idx)));
                        let gap = (mx.saturating_sub(r.x + 1 + title.chars().count() as u16)) as usize;
                        if gap > 0 {
                            spans.push(Span::raw(" ".repeat(gap)));
                        }
                        spans.push(Span::styled(
                            m_lbl,
                            Style::default()
                                .fg(Color::Black)
                                .bg(pal.accent)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    frame.render_widget(Paragraph::new(Line::from(spans)), r);
                }
                PopupNavRow::Track { sel_idx, track_idx } => {
                    let Some(tr) = p.tracks.get(*track_idx) else {
                        continue;
                    };
                    let is_sel = *sel_idx == state.music.album_popup.cursor;
                    let is_playing = active_vid.as_deref() == Some(tr.video_id.as_str());
                    let bg = Color::Reset;
                    let liked = state.music.liked_ids.contains(&tr.video_id);
                    let lead = if is_playing {
                        "▶ ".to_string()
                    } else if is_sel {
                        "› ".to_string()
                    } else {
                        format!("{:>2}.", track_idx + 1)
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

                    let pill_q = " +Q ";
                    let pill_h = if liked { " ♥ " } else { " ♡ " };
                    let hx = r.x + r.width.saturating_sub(14);
                    let qx = r.x + r.width.saturating_sub(10);
                    // Push row SelectRow first, then specific pill hitboxes so .rev() prioritizes pills!
                    state
                        .music
                        .album_popup
                        .hitboxes
                        .push((r, YtmAlbumPopupHit::SelectRow(*sel_idx)));
                    state
                        .music
                        .album_popup
                        .hitboxes
                        .push((Rect::new(hx, row_y, 3, 1), YtmAlbumPopupHit::LikeTrack(*track_idx)));
                    state
                        .music
                        .album_popup
                        .hitboxes
                        .push((Rect::new(qx, row_y, 4, 1), YtmAlbumPopupHit::QueueTrack(*track_idx)));

                    let title_col = if is_playing {
                        spotify_green
                    } else if is_sel {
                        pal.accent
                    } else {
                        Color::White
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                format!(" {lead:<3} "),
                                Style::default()
                                    .fg(if is_playing {
                                        spotify_green
                                    } else if is_sel {
                                        pal.accent
                                    } else {
                                        pal.text_dim
                                    })
                                    .bg(bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    "{:<w$}",
                                    truncate_str(&tr.title, t_w.max(6)),
                                    w = t_w.max(6)
                                ),
                                Style::default()
                                    .fg(title_col)
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
                PopupNavRow::ShelfItem {
                    sel_idx,
                    flat_item_idx: _,
                    item,
                } => {
                    let is_sel = *sel_idx == state.music.album_popup.cursor;
                    state
                        .music
                        .album_popup
                        .hitboxes
                        .push((r, YtmAlbumPopupHit::SelectRow(*sel_idx)));
                    let marker = if is_sel { " ›" } else { "  " };
                    let badge = format!("[{}]", item.kind.badge());
                    let dur_s = item
                        .duration_secs
                        .map(|d| format!(" · {}", format_duration_secs(d)))
                        .unwrap_or_default();
                    let title_str = truncate_str(&item.title, (r.width as usize).saturating_sub(26));
                    let sub_str = truncate_str(&item.subtitle, 24);
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                format!("{marker} "),
                                Style::default()
                                    .fg(if is_sel { spotify_green } else { pal.text_dim })
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("{badge:<10} "),
                                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                title_str,
                                Style::default()
                                    .fg(if is_sel { spotify_green } else { Color::White })
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                if sub_str.is_empty() && dur_s.is_empty() {
                                    String::new()
                                } else if sub_str.is_empty() {
                                    dur_s
                                } else {
                                    format!(" · {sub_str}{dur_s}")
                                },
                                Style::default().fg(pal.text_dim),
                            ),
                        ]))
                        .style(Style::default().bg(Color::Reset)),
                        r,
                    );
                }
                PopupNavRow::AboutLine { text } => {
                    frame.render_widget(
                        Paragraph::new(format!(" {text}"))
                            .style(Style::default().fg(pal.text_dim).bg(Color::Reset)),
                        r,
                    );
                }
            }
        }

        // Collapsible Right Preview Inspector Drawer inside Popup
        if let Some(insp_area) = inspector_opt {
            let insp_blk = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(pal.accent))
                .title(Span::styled(
                    " PREVIEW INSPECTOR ",
                    Style::default()
                        .fg(pal.accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(Color::Reset));
            let i_inner = insp_blk.inner(insp_area);
            frame.render_widget(insp_blk, insp_area);

            if i_inner.width >= 14 && i_inner.height >= 6 {
                let (sel_k, sel_u, kind_tag, s_title, s_sub, s_meta, can_open_alb, is_playable, vid_opt) =
                    if let Some(ref tr) = inspected_track {
                        let k = tr.thumb_cache_key();
                        let u = tr.effective_thumb_url();
                        let kind = if tr.kind == YtmItemKind::Video {
                            "[VID] mpv Video"
                        } else {
                            "[TRK] Lossless Audio"
                        };
                        let dur = tr
                            .duration_text
                            .as_deref()
                            .map(ToString::to_string)
                            .or_else(|| tr.duration_secs.map(format_duration_secs))
                            .unwrap_or_default();
                        let plays = tr.plays_text.as_deref().unwrap_or("");
                        let meta = [tr.album.as_deref().unwrap_or(""), &dur, plays]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");
                        (
                            k,
                            u,
                            kind,
                            tr.title.clone(),
                            tr.artist.clone(),
                            meta,
                            tr.album_browse_id.is_some(),
                            true,
                            Some(tr.video_id.clone()),
                        )
                    } else if let Some(ref it) = inspected_item {
                        let k = it.thumb_cache_key();
                        let u = it.thumb_url.clone();
                        let kind = match it.kind {
                            YtmItemKind::Video => "[VID] mpv Video",
                            YtmItemKind::Song => "[TRK] Song Preview",
                            YtmItemKind::Album => "[ALB] Album",
                            YtmItemKind::Playlist => "[PLS] Playlist",
                            YtmItemKind::Artist => "[ART] Artist",
                            YtmItemKind::Radio => "[MIX] Mix",
                        };
                        let dur = it
                            .duration_secs
                            .map(format_duration_secs)
                            .unwrap_or_default();
                        (
                            k,
                            u,
                            kind,
                            it.title.clone(),
                            it.subtitle.clone(),
                            dur,
                            false,
                            it.video_id.is_some(),
                            it.video_id.clone(),
                        )
                    } else {
                        (
                            art_key.clone(),
                            art_url.clone(),
                            "[◈] Selection",
                            p.header_title
                                .clone()
                                .unwrap_or_else(|| state.music.album_popup.title.clone()),
                            p.header_artist
                                .clone()
                                .or_else(|| p.header_subscribers.clone())
                                .unwrap_or_default(),
                            p.header_track_summary.clone().unwrap_or_default(),
                            false,
                            false,
                            None,
                        )
                    };

                if !sel_k.is_empty() && !sel_u.is_empty() {
                    state.queue_thumb_with_priority(&sel_k, &sel_u, ThumbPriority::Focused);
                }

                let cov_h = (i_inner.height / 2).clamp(4, 10);
                let i_parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(cov_h),
                        Constraint::Min(3),
                        Constraint::Length(2),
                    ])
                    .split(i_inner);

                let cov_w = (cov_h * 2 + 2).min(i_parts[0].width);
                let cov_x = i_parts[0].x + i_parts[0].width.saturating_sub(cov_w) / 2;
                let preview_cov_rect = Rect::new(cov_x, i_parts[0].y, cov_w, i_parts[0].height);
                if !sel_u.is_empty() {
                    render_thumbnail_cover(frame, preview_cov_rect, &sel_k, state, "Preview");
                } else {
                    frame.render_widget(
                        Paragraph::new("\n◈ PREVIEW")
                            .alignment(Alignment::Center)
                            .style(Style::default().fg(pal.accent)),
                        preview_cov_rect,
                    );
                }

                let iw = i_parts[1].width.saturating_sub(2) as usize;
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(
                            truncate_str(kind_tag, iw),
                            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            truncate_str(&s_title, iw),
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            truncate_str(&s_sub, iw),
                            Style::default().fg(pal.text_dim),
                        )),
                        Line::from(Span::styled(
                            truncate_str(&s_meta, iw),
                            Style::default().fg(spotify_green),
                        )),
                    ])
                    .wrap(Wrap { trim: true }),
                    Rect::new(i_parts[1].x + 1, i_parts[1].y, i_parts[1].width.saturating_sub(2), i_parts[1].height),
                );

                // Action pills in Inspector Footer
                let act_r = i_parts[2];
                let mut ax = act_r.x + 1;
                let mut r0_spans = Vec::new();
                let prim_lbl = if kind_tag.contains("VID") {
                    " [▸ Watch] "
                } else if is_playable {
                    " [▸ Play] "
                } else {
                    " [◈ Open] "
                };
                let pw = prim_lbl.chars().count() as u16;
                if ax + pw <= act_r.x + act_r.width {
                    state
                        .music
                        .album_popup
                        .hitboxes
                        .push((Rect::new(ax, act_r.y, pw, 1), YtmAlbumPopupHit::PreviewPlay));
                    r0_spans.push(Span::styled(
                        prim_lbl,
                        Style::default()
                            .fg(Color::Black)
                            .bg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    ));
                    ax += pw + 1;
                }
                if is_playable {
                    let qlbl = " +Q ";
                    let qw = qlbl.chars().count() as u16;
                    if ax + qw <= act_r.x + act_r.width {
                        state
                            .music
                            .album_popup
                            .hitboxes
                            .push((Rect::new(ax, act_r.y, qw, 1), YtmAlbumPopupHit::PreviewQueue));
                        r0_spans.push(Span::raw(" "));
                        r0_spans.push(Span::styled(
                            qlbl,
                            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                        ));
                        ax += qw + 1;
                    }
                    let liked = vid_opt
                        .as_deref()
                        .is_some_and(|v| state.music.liked_ids.contains(v));
                    let hlbl = if liked { " ♥ " } else { " ♡ " };
                    let hw = hlbl.chars().count() as u16;
                    if ax + hw <= act_r.x + act_r.width {
                        state
                            .music
                            .album_popup
                            .hitboxes
                            .push((Rect::new(ax, act_r.y, hw, 1), YtmAlbumPopupHit::PreviewLike));
                        r0_spans.push(Span::raw(" "));
                        r0_spans.push(Span::styled(
                            hlbl,
                            Style::default().fg(if liked {
                                spotify_green
                            } else {
                                pal.text_dim
                            }),
                        ));
                    }
                }
                frame.render_widget(
                    Paragraph::new(Line::from(r0_spans)),
                    Rect::new(act_r.x, act_r.y, act_r.width, 1),
                );
                if act_r.height >= 2 && can_open_alb {
                    let alb_lbl = " [Open Album >] ";
                    let aw = alb_lbl.chars().count() as u16;
                    state.music.album_popup.hitboxes.push((
                        Rect::new(act_r.x + 1, act_r.y + 1, aw.min(act_r.width), 1),
                        YtmAlbumPopupHit::PreviewOpenAlbum,
                    ));
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            alb_lbl,
                            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                        )),
                        Rect::new(act_r.x + 1, act_r.y + 1, aw.min(act_r.width), 1),
                    );
                }
            }
        }
    }

    frame.render_widget(
        Paragraph::new(if is_artist {
            "[1x Click] Preview Cover · [2x Click/Enter] Play, Watch Video or Open · [d] Drawer · [<- Back] · [Esc] Close"
        } else {
            "[1x Click] Inspect Song · [2x Click/Enter] Play or Watch Video · [+Q/a] Queue · [♥/l] Like · [d] Drawer · [Esc] Close"
        })
        .alignment(Alignment::Center)
        .style(Style::default().fg(pal.text_dim)),
        footer,
    );
}

/// Transparent Separate YouTube Music Settings Overlay (decoupled from Video Settings).
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
            " [=] YouTube Music & Lossless Audio Settings ",
            Style::default()
                .fg(spotify_green)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Reset));
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

    // Header row with quick jump button to the 10-Band EQ Studio
    let eq_btn_lbl = " [≡ Open 10-Band EQ Studio (e)] ";
    let eq_bw = eq_btn_lbl.chars().count() as u16;
    let eq_bx = chunks[0].x + chunks[0].width.saturating_sub(eq_bw + 1);
    state
        .music
        .settings_hitboxes
        .push((Rect::new(eq_bx, chunks[0].y, eq_bw, 1), MusicSettingsHit::OpenEqStudio));
    let desc_w = eq_bx.saturating_sub(chunks[0].x + 1) as usize;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                truncate_str(
                    " Separate Spotify / PipeWire Float preferences ([music] in config.toml)",
                    desc_w,
                ),
                Style::default().fg(theme.text_dim),
            ),
            Span::raw(" "),
            Span::styled(
                eq_btn_lbl,
                Style::default()
                    .fg(Color::Black)
                    .bg(spotify_green)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
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
        let bg = Color::Reset;
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
                            .fg(if sel { theme.accent } else { Color::White })
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
        Paragraph::new("[Up/Down] Navigate · [Enter/Click/Space] Cycle & Apply Live · [e] 10-Band EQ · [Esc] Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[2],
    );
}

/// Dedicated Transparent 10-Band Interactive Music Equalizer Studio Modal (`ActiveOverlay::MusicEq`).
pub fn render_music_eq_modal(frame: &mut Frame, state: &mut AppState) {
    let Some(modal_area) =
        crate::state::active_modal_rect(frame.area(), crate::state::ActiveOverlay::MusicEq)
    else {
        return;
    };
    state.music.eq.hitboxes.clear();
    frame.render_widget(Clear, modal_area);

    let theme = state.base_theme().clone();
    let spotify_green = Color::Rgb(30, 215, 96);
    let bypass = state.config.music.eq_bypass;
    let badge = state.config.music.eq_badge_label();
    let title = format!(" ≡ YouTube Music 10-Band EQ Studio ({badge}) ");

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(spotify_green))
        .title(Span::styled(
            title,
            Style::default()
                .fg(spotify_green)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Reset));
    let inner = blk.inner(modal_area);
    frame.render_widget(blk, modal_area);
    if inner.width < 46 || inner.height < 12 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(inner);

    let hits = &mut state.music.eq.hitboxes;
    let cur_preset = state.config.music.eq_preset;
    let active_saved = state.config.music.active_saved_eq.clone();
    let preamp = state.config.music.preamp_db;
    let bands = state.config.music.active_eq_bands();
    let cursor = state.music.eq.band_cursor.min(9);
    let dropdown_open = state.music.eq.dropdown_open;

    let render_pill_strip = |frame: &mut Frame,
                             row: Rect,
                             prefix: &str,
                             prefix_col: Color,
                             pills: &[(String, Style, Option<MusicEqHit>)],
                             hits: &mut Vec<(Rect, MusicEqHit)>| {
        let mut x = row.x + 1;
        let mut spans = vec![Span::styled(
            prefix.to_string(),
            Style::default().fg(prefix_col).add_modifier(Modifier::BOLD),
        )];
        x += prefix.chars().count() as u16;
        for (lbl, st, hit_opt) in pills {
            let w = lbl.chars().count() as u16;
            if x + w > row.x + row.width {
                break;
            }
            if let Some(hit) = hit_opt {
                hits.push((Rect::new(x, row.y, w, 1), *hit));
            }
            spans.push(Span::styled(lbl.clone(), *st));
            spans.push(Span::raw(" "));
            x += w + 1;
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    };

    let pill_st = |on: bool, col: Color| -> Style {
        if on {
            Style::default()
                .fg(Color::Black)
                .bg(col)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim).bg(Color::Reset)
        }
    };

    if state.music.eq.save_editing {
        let cancel_lbl = " [Esc Cancel] ";
        let cw = cancel_lbl.chars().count() as u16;
        let cx = chunks[0].x + chunks[0].width.saturating_sub(cw + 1);
        hits.push((Rect::new(cx, chunks[0].y, cw, 1), MusicEqHit::CancelSavePrompt));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Save EQ Name: ",
                    Style::default().fg(spotify_green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}▏  (Enter to save)", state.music.eq.save_input),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ),
                Span::styled(
                    cancel_lbl,
                    Style::default().fg(Color::Rgb(245, 110, 110)),
                ),
            ])),
            chunks[0],
        );
    } else {
        let active_dd_name = active_saved
            .as_deref()
            .map(|s| format!("◈ {s}"))
            .unwrap_or_else(|| cur_preset.label().to_string());
        let dd_arrow = if dropdown_open { "▴" } else { "▾" };
        let top_pills = vec![
            (
                format!("[{dd_arrow} {active_dd_name}]"),
                if dropdown_open {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Black)
                        .bg(spotify_green)
                        .add_modifier(Modifier::BOLD)
                },
                Some(MusicEqHit::ToggleDropdown),
            ),
            (
                "[+] Save As".to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                Some(MusicEqHit::StartSavePrompt),
            ),
            (
                "[Save Custom]".to_string(),
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
                Some(MusicEqHit::OverwriteCustom),
            ),
            (
                if bypass {
                    "[b Bypass:ON]"
                } else {
                    "[b Bypass:OFF]"
                }
                .to_string(),
                pill_st(bypass, Color::Rgb(251, 191, 36)),
                Some(MusicEqHit::ToggleBypass),
            ),
            (
                "[0 Flat]".to_string(),
                Style::default().fg(theme.text_dim),
                Some(MusicEqHit::ResetFlat),
            ),
        ];
        render_pill_strip(frame, chunks[0], " Preset : ", theme.accent, &top_pills, hits);
    }

    let eff_preamp = crate::api::player::effective_music_preamp_db(&state.config.music);
    let headroom_txt = format!(
        "Net:{eff_preamp:+}dB{}",
        if state.config.music.anti_clip_limiter {
            "+Lim"
        } else {
            ""
        }
    );
    let curve_sig = if active_saved.is_some() || cur_preset == AudioEqPreset::Custom {
        let sub = (bands[0] + bands[1]) / 2;
        let mid = (bands[4] + bands[5]) / 2;
        let air = (bands[8] + bands[9]) / 2;
        format!("Custom Curve (Sub:{sub:+} / Mid:{mid:+} / Air:{air:+})")
    } else {
        cur_preset.signature_summary()
    };
    let row1_tools = vec![
        ("Preamp:".to_string(), Style::default().fg(theme.text_dim), None),
        (
            "[-]".to_string(),
            Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
            Some(MusicEqHit::PreampDelta(-1)),
        ),
        (
            format!("{preamp:+}dB"),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            Some(MusicEqHit::PreampDelta(0)),
        ),
        (
            "[+]".to_string(),
            Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
            Some(MusicEqHit::PreampDelta(1)),
        ),
        (
            headroom_txt,
            Style::default().fg(spotify_green).add_modifier(Modifier::BOLD),
            None,
        ),
        (
            format!("· {curve_sig}"),
            Style::default().fg(theme.text_dim),
            None,
        ),
    ];
    render_pill_strip(frame, chunks[1], " Stage  : ", theme.text_dim, &row1_tools, hits);

    let eq_area = chunks[2];
    let show_steppers = eq_area.height >= 6;
    let bar_h = eq_area
        .height
        .saturating_sub(if show_steppers { 4 } else { 2 })
        .max(2);
    let band_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 10); 10])
        .split(eq_area);

    for (idx, &col_rect) in band_cols.iter().enumerate() {
        let gain = bands[idx].clamp(-12, 12);
        let is_sel = idx == cursor;
        let fill_col = if bypass {
            theme.text_dim
        } else if gain >= 5 {
            Color::Rgb(251, 191, 36)
        } else if gain > 0 {
            spotify_green
        } else if gain < 0 {
            Color::Rgb(56, 189, 248)
        } else {
            theme.secondary
        };

        let mut fc = Vec::new();
        if show_steppers {
            fc.push(Constraint::Length(1));
        }
        fc.push(Constraint::Length(1));
        fc.push(Constraint::Length(bar_h));
        if show_steppers {
            fc.push(Constraint::Length(1));
        }
        fc.push(Constraint::Min(1));

        let b_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(fc)
            .split(col_rect);

        let mut bi = 0usize;
        if show_steppers {
            frame.render_widget(
                Paragraph::new(Span::styled("[+]", Style::default().fg(theme.secondary)))
                    .alignment(Alignment::Center),
                b_rows[bi],
            );
            hits.push((b_rows[bi], MusicEqHit::BandDelta(idx, 1)));
            bi += 1;
        }

        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("{gain:+}dB"),
                Style::default()
                    .fg(if is_sel { Color::White } else { fill_col })
                    .add_modifier(if is_sel || gain != 0 {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ))
            .alignment(Alignment::Center),
            b_rows[bi],
        );
        hits.push((b_rows[bi], MusicEqHit::SelectBand(idx)));
        bi += 1;

        let meter_area = b_rows[bi];
        bi += 1;
        let bh = meter_area.height as usize;
        let zero_row = bh / 2;
        let pos_steps = zero_row.max(1);
        let neg_steps = bh.saturating_sub(zero_row + 1).max(1);
        let bw = (meter_area.width as usize).saturating_sub(2).clamp(2, 4);

        for r in 0..bh {
            let row_rect = Rect::new(meter_area.x, meter_area.y + r as u16, meter_area.width, 1);
            let mapped_db: i8 = if bh <= 2 {
                if r == 0 { 6 } else { -6 }
            } else if r == zero_row {
                0
            } else if r < zero_row {
                let frac = (zero_row - r) as f64 / pos_steps as f64;
                ((frac * 12.0).round() as i8).clamp(1, 12)
            } else {
                let frac = (r - zero_row) as f64 / neg_steps as f64;
                (-(frac * 12.0).round() as i8).clamp(-12, -1)
            };
            hits.push((row_rect, MusicEqHit::SetBandGain(idx, mapped_db)));

            let filled = if bh <= 2 {
                (r == 0 && gain > 0) || (r > 0 && gain < 0)
            } else if gain == 0 {
                r == zero_row
            } else if gain > 0 {
                let target = (gain as f64 / 12.0 * pos_steps as f64).ceil() as usize;
                let top_row = zero_row.saturating_sub(target);
                r >= top_row && r <= zero_row
            } else {
                let target = ((-gain) as f64 / 12.0 * neg_steps as f64).ceil() as usize;
                let bot_row = (zero_row + target).min(bh - 1);
                r >= zero_row && r <= bot_row
            };

            let (glyph, col) = if filled {
                ("█".repeat(bw), fill_col)
            } else if r == zero_row {
                ("─".repeat(bw), theme.border_idle)
            } else {
                ("·".repeat(bw), theme.text_dim)
            };
            frame.render_widget(
                Paragraph::new(Span::styled(glyph, Style::default().fg(col)))
                    .alignment(Alignment::Center),
                row_rect,
            );
        }

        if show_steppers {
            frame.render_widget(
                Paragraph::new(Span::styled("[-]", Style::default().fg(theme.secondary)))
                    .alignment(Alignment::Center),
                b_rows[bi],
            );
            hits.push((b_rows[bi], MusicEqHit::BandDelta(idx, -1)));
            bi += 1;
        }

        let flbl = EQ_BAND_LABELS[idx];
        frame.render_widget(
            Paragraph::new(Span::styled(
                if is_sel {
                    format!("[{flbl}]")
                } else {
                    flbl.to_string()
                },
                Style::default()
                    .fg(if is_sel { spotify_green } else { theme.text_dim })
                    .add_modifier(if is_sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ))
            .alignment(Alignment::Center),
            b_rows[bi],
        );
        hits.push((b_rows[bi], MusicEqHit::SelectBand(idx)));
    }

    // Floating Preset Dropdown Menu Overlay when dropdown_open == true
    if dropdown_open && inner.width >= 36 && inner.height >= 8 {
        #[derive(Clone)]
        enum DdRow {
            Header(&'static str),
            Builtin(AudioEqPreset),
            Saved(usize, String),
            SavePrompt,
        }
        let mut dd_rows: Vec<DdRow> = vec![DdRow::Header("AUDIOPHILE CALIBRATED PRESETS (18)")];
        for &p in &AudioEqPreset::AUDIOPHILE_PRESETS {
            dd_rows.push(DdRow::Builtin(p));
        }
        if !state.config.music.saved_eq_presets.is_empty() {
            dd_rows.push(DdRow::Header("SAVED CUSTOM PROFILES"));
            for (i, s) in state.config.music.saved_eq_presets.iter().enumerate() {
                dd_rows.push(DdRow::Saved(i, s.name.clone()));
            }
        }
        dd_rows.push(DdRow::SavePrompt);

        let total_dd = dd_rows.len();
        let dd_w = 58u16.min(inner.width.saturating_sub(4));
        let dd_x = (inner.x + 10).min(inner.x + inner.width.saturating_sub(dd_w + 1));
        let dd_h = ((total_dd + 2) as u16).min(chunks[2].height.saturating_add(1)).max(6);
        let dd_rect = Rect::new(dd_x, chunks[1].y, dd_w, dd_h);

        frame.render_widget(Clear, dd_rect);
        let dd_blk = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(spotify_green))
            .title(Span::styled(
                " Audiophile EQ Presets ",
                Style::default()
                    .fg(spotify_green)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(16, 19, 27)));
        let dd_inner = dd_blk.inner(dd_rect);
        frame.render_widget(dd_blk, dd_rect);

        // Push dd_rect first so individual rows pushed afterward override it, while background closes dropdown cleanly
        hits.push((dd_rect, MusicEqHit::ToggleDropdown));

        let vis_dd = dd_inner.height as usize;
        let dd_cur = state.music.eq.dropdown_cursor.min(total_dd.saturating_sub(1));
        let scroll_dd = if dd_cur >= vis_dd {
            dd_cur + 1 - vis_dd
        } else {
            0
        };

        for (vi, item) in dd_rows.iter().enumerate().skip(scroll_dd).take(vis_dd) {
            let row_rect = Rect::new(dd_inner.x, dd_inner.y + vi as u16 - scroll_dd as u16, dd_inner.width, 1);
            let is_hl = vi == dd_cur;
            match item {
                DdRow::Header(hdr) => {
                    frame.render_widget(
                        Paragraph::new(format!(" {hdr}")).style(
                            Style::default()
                                .fg(theme.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        row_rect,
                    );
                }
                DdRow::Builtin(p) => {
                    let is_act = !bypass && active_saved.is_none() && cur_preset == *p;
                    let sig = p.signature_summary();
                    let name_s = p.label();
                    let w = row_rect.width as usize;
                    let mark = if is_act { "● " } else if is_hl { "› " } else { "  " };
                    let left = format!(" {mark}{name_s}");
                    let gap = w.saturating_sub(left.chars().count() + sig.chars().count() + 1);
                    let line = Line::from(vec![
                        Span::styled(
                            left,
                            Style::default()
                                .fg(if is_act {
                                    spotify_green
                                } else if is_hl {
                                    Color::White
                                } else {
                                    theme.text
                                })
                                .add_modifier(if is_act || is_hl {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::raw(" ".repeat(gap)),
                        Span::styled(sig, Style::default().fg(theme.text_dim)),
                    ]);
                    frame.render_widget(
                        Paragraph::new(line).style(Style::default().bg(if is_hl {
                            Color::Rgb(34, 42, 58)
                        } else {
                            Color::Rgb(16, 19, 27)
                        })),
                        row_rect,
                    );
                    hits.push((row_rect, MusicEqHit::SelectDropdownPreset(*p)));
                }
                DdRow::Saved(idx, name) => {
                    let is_act = !bypass
                        && active_saved
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(name));
                    let del_x = row_rect.x + row_rect.width.saturating_sub(4);
                    let main_w = row_rect.width.saturating_sub(4);
                    hits.push((
                        Rect::new(row_rect.x, row_rect.y, main_w, 1),
                        MusicEqHit::SelectSavedPreset(*idx),
                    ));
                    hits.push((
                        Rect::new(del_x, row_rect.y, 4, 1),
                        MusicEqHit::DeleteSavedPreset(*idx),
                    ));
                    let mark = if is_act { "◈ " } else if is_hl { "› " } else { "  " };
                    let txt = truncate_str(&format!(" {mark}{name} (Saved)"), main_w as usize);
                    let pad = (main_w as usize).saturating_sub(txt.chars().count());
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                txt,
                                Style::default()
                                    .fg(if is_act { spotify_green } else { Color::White })
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" ".repeat(pad)),
                            Span::styled("[x]", Style::default().fg(Color::Rgb(245, 110, 110))),
                        ]))
                        .style(Style::default().bg(if is_hl {
                            Color::Rgb(34, 42, 58)
                        } else {
                            Color::Rgb(16, 19, 27)
                        })),
                        row_rect,
                    );
                }
                DdRow::SavePrompt => {
                    hits.push((row_rect, MusicEqHit::StartSavePrompt));
                    frame.render_widget(
                        Paragraph::new("  [+] Save Current Curve as Named Profile…")
                            .style(
                                Style::default()
                                    .fg(theme.accent)
                                    .bg(Color::Rgb(16, 19, 27))
                                    .add_modifier(Modifier::BOLD),
                            ),
                        row_rect,
                    );
                }
            }
        }
    }

    let close_rect = Rect::new(
        chunks[3].x + chunks[3].width.saturating_sub(12),
        chunks[3].y,
        11,
        1,
    );
    state
        .music
        .eq
        .hitboxes
        .push((close_rect, MusicEqHit::Close));
    frame.render_widget(
        Paragraph::new("[p] Presets · [←/→] Band · [↑/↓/Click/Scroll] Gain · [w] Save · [b] Bypass · [Esc] Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[3],
    );
}

pub const MUSIC_SETTINGS_ROW_COUNT: usize = 10;

pub fn music_settings_rows(cfg: &MusicSettingsConfig) -> [(&'static str, String, &'static str); MUSIC_SETTINGS_ROW_COUNT] {
    [
        (
            "Audio Stream Quality",
            cfg.audio_quality.label().to_string(),
            "Best ABR Opus 48 kHz + format-sort=abr for lossless-clean fidelity",
        ),
        (
            "Volume Normalization",
            cfg.normalization.label().to_string(),
            "Clip-safe native matching (Off = Pure Studio Direct path)",
        ),
        (
            "Anti-Clipping Limiter",
            if cfg.anti_clip_limiter {
                "ON (64-bit Float True-Peak Safe)".to_string()
            } else {
                "OFF (Pure Direct Pass-Through)".to_string()
            },
            "Transparent 64-bit lookahead guard preventing DAC clipping",
        ),
        (
            "Music Equalizer Preset",
            cfg.eq_badge_label(),
            "Press [e] or click [EQ] pill to edit 10-band curves & saved presets",
        ),
        (
            "Manual Preamp Trim",
            format!("{:+} dB", cfg.preamp_db),
            "Clean headroom trim without dulling flat tracks",
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
                "Windowless PipeWire Float Audio".to_string()
            },
            "Windowless Audio streams 32-bit float natively cleanly",
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
                        thumb_url: "https://lh3.googleusercontent.com/s=w544-h544".to_string(),
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
                        thumb_url: "https://lh3.googleusercontent.com/a=w544-h544".to_string(),
                        duration_secs: None,
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        });
        st.music.queue = vec![YtmTrack {
            video_id: "Pgum6OT_VH8".to_string(),
            title: "Starlight".to_string(),
            artist: "Muse".to_string(),
            artist_browse_id: Some("UCmuse".to_string()),
            album: Some("Black Holes".to_string()),
            album_browse_id: Some("MPREb_123".to_string()),
            duration_text: Some("4:00".to_string()),
            duration_secs: Some(240),
            plays_text: None,
            thumb_url: "https://lh3.googleusercontent.com/s=w544-h544".to_string(),
            playlist_id: None,
            kind: YtmItemKind::Song,
        }];
        st.active_playback_video_id = Some("Pgum6OT_VH8".to_string());
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
        assert!(text.contains("[EQ:"));
        assert!(st.music.hitboxes.iter().any(|(_, h)| *h == MusicHit::OpenMusicEq));
        // Transparent background preserved across Music workspace panels (no dark RGB fill slabs)
        assert_eq!(term.backend().buffer()[(40, 8)].bg, Color::Reset);
        assert_eq!(term.backend().buffer()[(10, 15)].bg, Color::Reset);
        // Zero emoji / slop glyphs in workspace buffer
        for bad in ["🔀", "📻", "💾", "⚙", "➕", "❌", "♪"] {
            assert!(!text.contains(bad), "unexpected emoji {bad} in music workspace");
        }

        // Playlist popup dynamically binds hero cover & status to active playing song
        st.overlay = ActiveOverlay::MusicAlbum;
        st.music.album_popup.title = "Still listening".to_string();
        st.music.album_popup.browse_id = "VLPL_123".to_string();
        st.music.album_popup.page = Some(YtmPage {
            header_title: Some("Still listening".to_string()),
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
        assert!(pop_text.contains("Playing #1: Starlight"));
        assert!(pop_text.contains("PREVIEW INSPECTOR"));

        // Artist profile renders SINGLE TOP SONGS (even if Top songs shelf items carry album_browse_id!)
        st.music.album_popup.title = "Muse".to_string();
        st.music.album_popup.browse_id = "UCmuse".to_string();
        st.music.album_popup.page = Some(YtmPage {
            is_artist_profile: true,
            header_title: Some("Muse".to_string()),
            header_subscribers: Some("14.2M subscribers".to_string()),
            header_description: Some("English rock band from Teignmouth.".to_string()),
            header_radio_playlist_id: Some("RDEM_muse".to_string()),
            tracks: st.music.queue.clone(),
            shelves: vec![
                YtmShelf {
                    title: "Top songs".to_string(),
                    items: vec![YtmShelfItem {
                        kind: YtmItemKind::Song,
                        title: "Starlight".to_string(),
                        subtitle: "Muse".to_string(),
                        video_id: Some("Pgum6OT_VH8".to_string()),
                        browse_id: Some("MPREb_123".to_string()),
                        playlist_id: None,
                        params: None,
                        thumb_url: String::new(),
                        duration_secs: Some(240),
                    }],
                    ..Default::default()
                },
                YtmShelf {
                    title: "Albums".to_string(),
                    items: vec![YtmShelfItem {
                        kind: YtmItemKind::Album,
                        title: "Absolution".to_string(),
                        subtitle: "2003".to_string(),
                        video_id: None,
                        browse_id: Some("MPREb_abso".to_string()),
                        playlist_id: None,
                        params: None,
                        thumb_url: String::new(),
                        duration_secs: None,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        term.draw(|f| crate::ui::render_app(f, &mut st)).unwrap();
        let art_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert_eq!(art_text.matches("TOP SONGS").count(), 1);
        assert!(art_text.contains("ALBUMS"));
        assert!(art_text.contains("Absolution"));
        assert!(art_text.contains("14.2M subscribers"));
        assert!(art_text.contains("Teignmouth"));
        assert!(art_text.contains("PREVIEW INSPECTOR"));
        for bad in ["🔀", "📻", "💾", "⚙", "➕", "❌", "♪"] {
            assert!(!art_text.contains(bad), "unexpected emoji {bad} in artist popup");
        }

        // Dedicated YouTube Music 10-Band EQ Studio modal renders sleek Dropdown Preset menu
        st.overlay = ActiveOverlay::MusicEq;
        st.music.eq.dropdown_open = true;
        term.draw(|f| crate::ui::render_app(f, &mut st)).unwrap();
        let eq_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(eq_text.contains("10-Band EQ Studio"));
        assert!(eq_text.contains("31Hz"));
        assert!(eq_text.contains("16kHz"));
        assert!(eq_text.contains("Save As"));
        assert!(eq_text.contains("Harman Target"));
        assert!(st
            .music
            .eq
            .hitboxes
            .iter()
            .any(|(_, h)| matches!(h, MusicEqHit::SelectDropdownPreset(crate::config::AudioEqPreset::Harman))));
        for bad in ["🔀", "📻", "💾", "⚙", "➕", "❌", "♪"] {
            assert!(!eq_text.contains(bad), "unexpected emoji {bad} in eq modal");
        }

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
        assert!(set_text.contains("Open 10-Band EQ Studio"));
        for bad in ["🔀", "📻", "💾", "⚙", "➕", "❌", "♪"] {
            assert!(!set_text.contains(bad), "unexpected emoji {bad} in settings modal");
        }
    }
}
