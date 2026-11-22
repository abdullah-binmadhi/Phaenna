//! Multi-Page Playback Studio (`p`) & Docked Live Now-Playing Bar.
//! Dispatches across 6 dedicated Studio pages inside a persistent shell.

pub mod audio;
pub mod captions;
pub mod deck;
pub mod mastering;
pub mod shell;
pub mod stream;
pub mod telemetry;
pub mod widgets;

use crate::state::{AppState, NowPlayingHit, StudioPage};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub use widgets::format_hms;

pub fn render_studio_modal(frame: &mut Frame, state: &mut AppState) {
    let Some(canvas) = shell::render_studio_shell(frame, state) else {
        return;
    };
    match state.studio.page {
        StudioPage::Deck => deck::render_deck_page(frame, canvas, state),
        StudioPage::StreamNet => stream::render_stream_page(frame, canvas, state),
        StudioPage::MasteringGpu => mastering::render_mastering_page(frame, canvas, state),
        StudioPage::AudioSync => audio::render_audio_page(frame, canvas, state),
        StudioPage::CaptionsSkip => captions::render_captions_page(frame, canvas, state),
        StudioPage::LiveTelemetry => telemetry::render_telemetry_page(frame, canvas, state),
    }
}

/// Docked 1-line live playback control bar rendered on Home & Detail while `mpv` IPC is active.
pub fn render_now_playing_bar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    state.live_playback.now_playing_hitboxes.clear();
    if area.width < 32 || area.height == 0 || !state.has_live_ipc() {
        return;
    }
    let theme = state.base_theme();
    let title_raw = state
        .active_playback_title
        .as_deref()
        .unwrap_or("Now Playing");
    let play_lbl = if state.live_playback.paused { "[Play]" } else { "[Pause]" };
    let dub_lbl = if state.config.dub_preference.is_dub_on() {
        format!("[Dub:{}]", state.config.dub_lang.to_uppercase())
    } else {
        "[Dub:OFF]".to_string()
    };
    let spd_lbl = format!("{:.2}x", state.live_playback.speed);
    let vol_lbl = if state.live_playback.muted {
        "MUTE".to_string()
    } else {
        format!("{}%", state.live_playback.volume)
    };
    let shader_lbl = format!("[{}]", state.config.shader_mode.short_label());
    let is_live_str = state.live_playback.is_live_stream;
    let time_lbl = if is_live_str {
        let lag = state.live_playback.live_edge_lag_secs(state.active_playback_pos);
        format!("Edge:-{lag:.1}s Buf:{:.1}s", state.live_playback.cache_ahead_secs)
    } else if state.live_playback.paused_for_cache {
        format!(
            "{}/{} BUF:{}%",
            format_hms(state.active_playback_pos),
            format_hms(state.active_playback_dur.max(0.0)),
            state.live_playback.buffering_pct.clamp(0, 100)
        )
    } else {
        format!(
            "{}/{} Buf:{:.0}s",
            format_hms(state.active_playback_pos),
            format_hms(state.active_playback_dur.max(0.0)),
            state.live_playback.cache_ahead_secs
        )
    };

    let bg = Color::Rgb(18, 22, 32);
    let mut spans: Vec<(String, Style, Option<NowPlayingHit>)> = vec![
        (
            if is_live_str { " LIVE " } else { " PLAY " }.to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(if is_live_str { Color::Rgb(248, 113, 113) } else { Color::Rgb(52, 211, 153) })
                .add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::OpenStudio),
        ),
        (format!(" {time_lbl} "), Style::default().fg(theme.text).bg(bg).add_modifier(Modifier::BOLD), None),
        (format!(" {play_lbl} "), Style::default().fg(theme.primary).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::TogglePause)),
        ("[-10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekBack)),
        (" ".to_string(), Style::default().bg(bg), None),
        ("[+10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekFwd)),
        (" ".to_string(), Style::default().bg(bg), None),
    ];
    if is_live_str {
        spans.push((
            "[E Edge]".to_string(),
            Style::default().fg(Color::Rgb(52, 211, 153)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::JumpToLiveEdge),
        ));
        spans.push((" ".to_string(), Style::default().bg(bg), None));
        spans.push((
            format!("[{}]", state.config.live_mode.short_badge()),
            Style::default().fg(Color::Rgb(248, 113, 113)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CycleLiveMode),
        ));
        spans.push((" ".to_string(), Style::default().bg(bg), None));
    }
    spans.extend([
        (
            dub_lbl,
            Style::default()
                .fg(Color::Black)
                .bg(if state.config.dub_preference.is_dub_on() { Color::Rgb(251, 191, 36) } else { theme.accent })
                .add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::ToggleDub),
        ),
        (" [<]".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::SpeedStep(-1))),
        (spd_lbl, Style::default().fg(theme.text).bg(bg), Some(NowPlayingHit::OpenStudio)),
        ("[>] ".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::SpeedStep(1))),
        ("[-]".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::VolumeDelta(-5))),
        (vol_lbl, Style::default().fg(theme.text).bg(bg), Some(NowPlayingHit::ToggleMute)),
        ("[+] ".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::VolumeDelta(5))),
        (
            format!("[{}]", state.config.performance_mode.short_badge()),
            Style::default().fg(Color::Rgb(56, 189, 248)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CyclePerfMode),
        ),
        (" ".to_string(), Style::default().bg(bg), None),
        (
            format!("[{}]", state.config.network_mode.short_badge()),
            Style::default()
                .fg(if state.config.network_mode.is_congested() {
                    Color::Rgb(251, 191, 36)
                } else {
                    theme.text_dim
                })
                .bg(bg)
                .add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CycleNetworkMode),
        ),
        (" ".to_string(), Style::default().bg(bg), None),
        (
            format!("[{}]", state.config.motion_mode.short_badge()),
            Style::default().fg(theme.secondary).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CycleMotion),
        ),
        (" ".to_string(), Style::default().bg(bg), None),
        (shader_lbl, Style::default().fg(theme.primary).bg(bg), Some(NowPlayingHit::CycleShader)),
        (" [Studio(p)]".to_string(), Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::OpenStudio)),
        (" [Stop] ".to_string(), Style::default().fg(Color::Rgb(239, 68, 68)).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::StopPlayer)),
    ]);

    let fixed_w: usize = spans.iter().map(|(s, _, _)| s.chars().count()).sum();
    let title_budget = (area.width as usize).saturating_sub(fixed_w + 3);
    if title_budget >= 8 {
        let trunc = marquee_window(title_raw, title_budget, state.ticker_step);
        spans.insert(
            1,
            (
                format!(" {trunc} ·"),
                Style::default().fg(theme.secondary).bg(bg),
                Some(NowPlayingHit::OpenStudio),
            ),
        );
    }

    let mut cur_x = area.x;
    let mut rendered_spans = Vec::with_capacity(spans.len() + 1);
    for (text, style, hit) in spans {
        let len = text.chars().count() as u16;
        if cur_x + len > area.x + area.width {
            break;
        }
        if let Some(action) = hit {
            state
                .live_playback
                .now_playing_hitboxes
                .push((Rect::new(cur_x, area.y, len, 1), action));
        }
        cur_x += len;
        rendered_spans.push(Span::styled(text, style));
    }
    if cur_x < area.x + area.width {
        rendered_spans.push(Span::styled(
            " ".repeat((area.x + area.width - cur_x) as usize),
            Style::default().bg(bg),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(rendered_spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::sponsorblock::SponsorSegment;
    use crate::api::ytdlp::{ResolvedAudioVariant, ResolvedStreamLadder, ResolvedVideoVariant};
    use crate::config::{Config, DubPreference, PerformanceMode};
    use crate::services::ipc::{LiveMediaTrack, TrackKind};
    use crate::state::{play_options_modal_rect, TELEMETRY_CAPACITY};
    use mastering::estimate_mastering_impact;
    use ratatui::{backend::TestBackend, Terminal};

    fn buffer_text(term: &Terminal<TestBackend>) -> String {
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    fn seed_rich_live_state() -> AppState {
        let mut st = AppState::new(Config::default());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        st.active_playback_video_id = Some("dQw4w9WgXcQ".into());
        st.active_playback_title = Some("Shooting with an old MFT camera.".into());
        st.active_playback_pos = 711.0;
        st.active_playback_dur = 1444.0;
        st.sponsor_skips_session = 2;
        st.live_playback.cmd_tx = Some(tx);
        st.live_playback.current_stream_height = 1440;
        st.live_playback.current_stream_fps = 60.0;
        st.live_playback.current_stream_tbr_kbps = 12800.0;
        st.live_playback.video_bitrate_kbps = 11200.0;
        st.live_playback.cache_speed_bps = 18_400_000;
        st.live_playback.cache_bytes = 256 * 1024 * 1024;
        st.live_playback.cache_ahead_secs = 42.5;
        st.live_playback.estimated_vf_fps = 59.94;
        st.live_playback.hwdec_current = "videotoolbox".into();
        st.live_playback.video_codec_live = "av1".into();
        st.live_playback.decoded_width = 2560;
        st.live_playback.decoded_height = 1440;
        st.live_playback.colormatrix = "bt.709".into();
        st.live_playback.audio_samplerate = 48000;
        st.live_playback.audio_channels = "stereo".into();
        st.config.dub_preference = DubPreference::PreferDubbed;

        st.live_playback.audio_tracks = vec![
            LiveMediaTrack {
                id: 2,
                kind: TrackKind::Audio,
                lang: Some("en-US".into()),
                title: Some("English (US) original".into()),
                is_dubbed: false,
                is_original: true,
                selected: false,
            },
            LiveMediaTrack {
                id: 3,
                kind: TrackKind::Audio,
                lang: Some("ar".into()),
                title: Some("Arabic dubbed".into()),
                is_dubbed: true,
                is_original: false,
                selected: true,
            },
        ];
        st.live_playback.sub_tracks = vec![LiveMediaTrack {
            id: 1,
            kind: TrackKind::Sub,
            lang: Some("en".into()),
            title: Some("English CC".into()),
            is_dubbed: false,
            is_original: false,
            selected: true,
        }];
        st.detail_sponsor_segments = vec![
            SponsorSegment {
                uuid: "s1".into(),
                category: "sponsor".into(),
                action_type: "skip".into(),
                start_time: 252.0,
                end_time: 308.0,
                video_duration: 1444.0,
            },
            SponsorSegment {
                uuid: "s2".into(),
                category: "outro".into(),
                action_type: "skip".into(),
                start_time: 1380.0,
                end_time: 1444.0,
                video_duration: 1444.0,
            },
        ];
        st.stream_ladder_cache.insert(
            "dQw4w9WgXcQ".into(),
            ResolvedStreamLadder {
                video_id: "dQw4w9WgXcQ".into(),
                resolved_at_unix: 9_999_999_999,
                video_variants: vec![
                    ResolvedVideoVariant {
                        format_id: "399".into(),
                        url: "https://cdn.example/1080.mp4".into(),
                        height: 1080,
                        fps: 60.0,
                        tbr_kbps: 5400.0,
                        vcodec: "av01.0.08M.08".into(),
                        has_audio: false,
                        user_agent: None,
                    },
                    ResolvedVideoVariant {
                        format_id: "400".into(),
                        url: "https://cdn.example/1440.mp4".into(),
                        height: 1440,
                        fps: 60.0,
                        tbr_kbps: 12800.0,
                        vcodec: "av01.0.12M.08".into(),
                        has_audio: false,
                        user_agent: None,
                    },
                ],
                audio_variants: vec![ResolvedAudioVariant {
                    format_id: "251".into(),
                    url: "https://cdn.example/a.webm".into(),
                    abr_kbps: 128.0,
                    acodec: "opus".into(),
                    lang: Some("en".into()),
                    format_note: "original".into(),
                    language_preference: 10,
                    is_dubbed: false,
                    is_original: true,
                    is_drc: false,
                    user_agent: None,
                }],
                preferred_sub_url: None,
            },
        );
        for i in 0..20 {
            st.live_playback.telemetry_history.push_sample(
                14000.0 + (i as f64) * 200.0,
                11000.0,
                35.0 + (i as f64) * 0.4,
                59.9,
                (i as f64 - 10.0) * 0.8,
                0.0,
                i == 7,
            );
        }
        st.log_studio_event("LAUNCH", "Started mpv IPC session (1440p60 AV1)");
        st.log_studio_event("SKIP", "SponsorBlock skipped Sponsor (+56s)");
        st
    }

    #[test]
    fn test_studio_all_6_pages_render_across_viewports() {
        let mut pre = AppState::new(Config::default());
        for page in StudioPage::ALL {
            pre.studio.page = page;
            let mut term = Terminal::new(TestBackend::new(136, 36)).unwrap();
            term.draw(|f| render_studio_modal(f, &mut pre)).unwrap();
            let txt = buffer_text(&term);
            assert!(txt.contains("[PRE-LAUNCH]"));
            assert!(!pre.live_playback.play_hitboxes.is_empty());
        }

        let expected_markers = [
            (StudioPage::Deck, "Picture Equalizer, Presets & Geometry"),
            (StudioPage::StreamNet, "Resolved Bitrate Ladder vs Throughput"),
            (StudioPage::MasteringGpu, "Active GPU Signal-Flow Pipeline"),
            (StudioPage::AudioSync, "10-Band Interactive Graphic Equalizer"),
            (StudioPage::CaptionsSkip, "Ad-Block & SponsorBlock Shield Engine"),
            (StudioPage::LiveTelemetry, "60s Throughput vs Video Bitrate"),
        ];

        for (w, h) in [(100, 28), (136, 36), (160, 44)] {
            let mut st = seed_rich_live_state();
            let modal = play_options_modal_rect(Rect::new(0, 0, w, h));
            assert!(modal.width >= 96 && modal.height >= 26);

            for (page, marker) in expected_markers {
                st.studio.page = page;
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| render_studio_modal(f, &mut st)).unwrap();
                let txt = buffer_text(&term);
                assert!(txt.contains("[LIVE mpv IPC]"), "Missing IPC badge on {page:?}");
                assert!(
                    txt.contains(marker),
                    "Missing '{marker}' on {page:?} at {w}x{h}"
                );
                assert!(!st.live_playback.play_hitboxes.is_empty());
            }
        }
    }

    #[test]
    fn test_estimate_mastering_impact_monotonicity_and_ring_buffer_bounds() {
        let mut cfg = Config {
            performance_mode: PerformanceMode::Efficient,
            ..Default::default()
        };
        let eco = estimate_mastering_impact(&cfg);
        cfg.performance_mode = PerformanceMode::Balanced;
        let bal = estimate_mastering_impact(&cfg);
        cfg.performance_mode = PerformanceMode::Quality;
        let qual = estimate_mastering_impact(&cfg);

        assert!(eco.gpu_pct < bal.gpu_pct && bal.gpu_pct < qual.gpu_pct);
        assert!(eco.ram_mb < bal.ram_mb && bal.ram_mb < qual.ram_mb);
        assert!(eco.thermal_pct <= bal.thermal_pct && bal.thermal_pct < qual.thermal_pct);

        let mut st = AppState::new(Config::default());
        for i in 0..85 {
            st.live_playback.telemetry_history.push_sample(
                i as f64 * 100.0,
                5000.0,
                20.0,
                60.0,
                1.5,
                0.0,
                false,
            );
            st.log_studio_event("MODE", format!("Event {i}"));
        }
        assert_eq!(st.live_playback.telemetry_history.cache_speed_kbps.len(), TELEMETRY_CAPACITY);
        assert_eq!(st.live_playback.telemetry_history.events.len(), 32);
    }

    #[test]
    fn test_deck_zero_concatenation_zero_dead_space_and_all_new_features() {
        for (w, h) in [(100, 28), (136, 36), (156, 40)] {
            let mut st = seed_rich_live_state();
            st.studio.page = StudioPage::Deck;
            st.config.picture.preset = crate::config::PicturePreset::Cinema;
            st.live_playback.sleep_timer_mode = crate::state::SleepTimerMode::Mins30;
            st.live_playback.ab_loop_a = Some(120.0);
            st.live_playback.ab_loop_b = Some(156.0);

            let modal = play_options_modal_rect(Rect::new(0, 0, w, h));
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render_studio_modal(f, &mut st)).unwrap();

            let buf = term.backend().buffer();
            let mut rows_text = Vec::new();
            for y in modal.y..modal.y + modal.height {
                let line: String = (modal.x..modal.x + modal.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect();
                assert!(
                    !line.contains("]["),
                    "Found concatenated pills '][' at {w}x{h} y={y}: {line:?}"
                );
                rows_text.push(line);
            }

            let full = rows_text.join("\n");
            for needle in [
                "Presets [g]:",
                "[Cinema]",
                "[OLED]",
                "[Daylight]",
                "[Noir]",
                "Brightness",
                "Saturation",
                "Grade Scope",
                "Set A",
                "Sleep Timer",
            ] {
                assert!(
                    full.contains(needle) || full.contains("A:02:00"),
                    "Missing '{needle}' on Deck at {w}x{h}"
                );
            }

            if h >= 36 {
                for needle in ["Hue / Tint", "Sharpness", "Zoom:", "[Reset Geom]", "[Save Custom]", "Screenshot"] {
                    assert!(
                        full.contains(needle),
                        "Missing expanded Deck control '{needle}' at {w}x{h}"
                    );
                }

                // Verify bottom action rows of both Deck panes sit at bottom (no trailing empty rows)
                let save_row = rows_text
                    .iter()
                    .position(|l| l.contains("[Save Custom]"))
                    .expect("Missing [Save Custom] row");
                let launch_row = rows_text
                    .iter()
                    .position(|l| l.contains("[Enter Relaunch / Apply]"))
                    .expect("Missing [Enter Relaunch / Apply] row");
                assert_eq!(
                    save_row, launch_row,
                    "Left and Right pane bottom action rows should align flush at {w}x{h}"
                );
                // Immediately below the shared bottom action row is the pane bottom border (`╰` / `─`)
                assert!(
                    rows_text[save_row + 1].contains('╰') || rows_text[save_row + 1].contains('─'),
                    "Unexpected dead rows below Deck panes at {w}x{h}: {:?}",
                    &rows_text[save_row..rows_text.len().min(save_row + 4)]
                );
            }
        }
    }

    #[test]
    fn test_pages_2_to_6_zero_concatenation_zero_cutoff_and_new_features() {
        use crate::db::cache::{VideoChapter, VideoItem};
        let pages_with_markers = [
            (
                StudioPage::StreamNet,
                vec!["Codec [C]:", "FPS Cap :", "Data Est :", "Audio:", "Auth Vault", "[Refresh Ladder (r)]"],
            ),
            (
                StudioPage::MasteringGpu,
                vec!["Profiles:", "[Cinema 24p]", "Chain (", "Thermal Verdict:", "Inspector [5K 2x Upscale]", "Use <=1080p for cool 4K streams.", "Flags:", "Delta vs BAL:"],
            ),
            (
                StudioPage::AudioSync,
                vec!["Presets [e]:", "31Hz", "16kHz", "[Cinema]", "[Rock]", "[EDM]", "Preamp:", "Spatial:"],
            ),
            (
                StudioPage::CaptionsSkip,
                vec!["Lang :", "Font :", "MgnY :", "Shd:", "[R Re-Scan]", "[U Undo Skip]", "Non-Music Section", "Stream Ads:", "View [u]:", "[Shield & WL"],
            ),
            (
                StudioPage::LiveTelemetry,
                vec!["Pipeline :", "Render Spec :", "Session IPC :", "Live [v]:", "DVR Cmd :", "Events [F]:"],
            ),
        ];

        for is_live in [false, true] {
            for (w, h) in [(100, 28), (136, 36), (156, 40)] {
                let mut st = if is_live {
                    seed_rich_live_state()
                } else {
                    let mut pre = AppState::new(Config::default());
                    pre.detail_video = Some(VideoItem {
                        id: "dQw4w9WgXcQ".into(),
                        title: "Shooting with an old MFT camera.".into(),
                        duration: Some(1444),
                        chapters: Some(vec![
                            VideoChapter { start_time: 0.0, end_time: 310.0, title: "01 Intro & Setup".into() },
                            VideoChapter { start_time: 310.0, end_time: 740.0, title: "02 Sensor Test".into() },
                            VideoChapter { start_time: 740.0, end_time: 1444.0, title: "03 Verdict".into() },
                        ]),
                        ..Default::default()
                    });
                    pre.screen = crate::state::Screen::Detail;
                    pre
                };
                if is_live {
                    st.detail_video = Some(VideoItem {
                        id: "dQw4w9WgXcQ".into(),
                        title: "Shooting with an old MFT camera.".into(),
                        duration: Some(1444),
                        chapters: Some(vec![
                            VideoChapter { start_time: 0.0, end_time: 310.0, title: "01 Intro & Setup".into() },
                            VideoChapter { start_time: 310.0, end_time: 1444.0, title: "02 Sensor Test".into() },
                        ]),
                        ..Default::default()
                    });
                    st.screen = crate::state::Screen::Detail;
                }

                let modal = play_options_modal_rect(Rect::new(0, 0, w, h));
                for (page, needles) in &pages_with_markers {
                    st.studio.page = *page;
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    term.draw(|f| render_studio_modal(f, &mut st)).unwrap();

                    let buf = term.backend().buffer();
                    let mut rows_text = Vec::new();
                    let mut split_char_col = modal.width as usize / 2;
                    for y in modal.y..modal.y + modal.height {
                        let line: String = (modal.x..modal.x + modal.width)
                            .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                            .collect();
                        if y > modal.y + 4 {
                            if let Some(byte_pos) = line.find("╮╭").or_else(|| line.find("╯╭")) {
                                split_char_col = line[..byte_pos].chars().count() + 1;
                            }
                        }
                        assert!(
                            !line.contains("]["),
                            "Concatenated '][' on {page:?} live={is_live} at {w}x{h} y={y}: {line:?}"
                        );
                        assert!(
                            !line.contains(":["),
                            "Concatenated ':[' on {page:?} live={is_live} at {w}x{h} y={y}: {line:?}"
                        );
                        rows_text.push(line);
                    }
                    let full = rows_text.join("\n");
                    let mut left_rows = Vec::new();
                    let mut right_rows = Vec::new();
                    for line in &rows_text {
                        let chars: Vec<char> = line.chars().collect();
                        let cut = split_char_col.min(chars.len());
                        left_rows.push(chars[..cut].iter().collect::<String>());
                        right_rows.push(chars[cut..].iter().collect::<String>());
                    }
                    let strip_borders = |s: String| -> String {
                        s.chars()
                            .map(|c| {
                                if matches!(c, '│' | '╭' | '╮' | '╰' | '╯') {
                                    ' '
                                } else {
                                    c
                                }
                            })
                            .collect::<String>()
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    let left_norm = strip_borders(left_rows.join(" "));
                    let right_norm = strip_borders(right_rows.join(" "));
                    let normalized: String = format!("{left_norm} {right_norm}");
                    assert!(
                        !full.contains("Use <=1080p for..."),
                        "Broken truncation returned on {page:?} at {w}x{h}"
                    );
                    if !is_live && *page == StudioPage::CaptionsSkip {
                        assert!(
                            full.contains("01 Intro & Setup"),
                            "Missing automatic Chapters fallback when 0 SB segments at {w}x{h}"
                        );
                    }
                    if !is_live && *page == StudioPage::LiveTelemetry {
                        assert!(
                            full.contains("Standby"),
                            "Missing pre-launch Standby target baseline on Page 6 at {w}x{h}"
                        );
                    }
                    if *page == StudioPage::AudioSync {
                        assert!(
                            !full.contains("A/V Sync") && !full.contains("Live A/V Drift"),
                            "Deprecated A/V Sync UI leaked back onto Page 4 at {w}x{h}"
                        );
                    }
                    if h >= 36 {
                        for needle in needles {
                            assert!(
                                full.contains(needle) || normalized.contains(needle),
                                "Missing '{needle}' on {page:?} live={is_live} at {w}x{h}\nRendered:\n{full}"
                            );
                        }
                        if *page == StudioPage::CaptionsSkip {
                            let start_idx = rows_text
                                .iter()
                                .position(|l| l.contains("Live Preview ("))
                                .expect("Missing bounded Live Preview box on Page 5");
                            let end_rel = rows_text[start_idx + 1..]
                                .iter()
                                .position(|l| l.contains('╰'))
                                .expect("Missing closing border on Live Preview");
                            assert!(
                                end_rel <= 4,
                                "Live Preview box grew too tall ({end_rel} inner rows) at {w}x{h}"
                            );
                        }
                    }
                }
            }
        }
    }
}
