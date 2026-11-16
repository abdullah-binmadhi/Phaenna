pub mod analytics;
pub mod calendar;
pub mod cast;
pub mod channel;
pub mod components;
pub mod detail;
pub mod download;
pub mod help;
pub mod home;
pub mod login;
pub mod play_options;
pub mod remote;
pub mod search;
pub mod settings;
pub mod setup;
pub mod tailored;
pub mod theme;
pub mod update_modal;

use crate::state::{active_modal_rect, ActiveOverlay, AppState, Screen};
use ratatui::Frame;

pub fn render_app(frame: &mut Frame, state: &mut AppState) {
    let modal_rect = active_modal_rect(frame.area(), state.overlay);

    match state.screen {
        Screen::Home => home::render_home_screen(frame, state, modal_rect),
        Screen::Detail => detail::render_detail_screen(frame, state, modal_rect),
        Screen::Channel => channel::render_channel_screen(frame, state, modal_rect),
    }

    match state.overlay {
        ActiveOverlay::None => {}
        ActiveOverlay::Search => search::render_search_modal(frame, state),
        ActiveOverlay::Login => login::render_login_modal(frame, state),
        ActiveOverlay::Filter => home::render_filter_modal(frame, state),
        ActiveOverlay::PlayOptions => play_options::render_play_options_modal(frame, state),
        ActiveOverlay::Settings => settings::render_settings_modal(frame, state),
        ActiveOverlay::Help => help::render_help_modal(frame, state),
        ActiveOverlay::Setup => setup::render_setup_modal(frame, state),
        ActiveOverlay::Analytics => analytics::render_analytics_modal(frame, state),
        ActiveOverlay::TailoredWizard => tailored::render_tailored_wizard(frame, state),
        ActiveOverlay::TailoredSummary => tailored::render_tailored_summary(frame, state),
        ActiveOverlay::TailoredResults => tailored::render_tailored_results(frame, state),
        ActiveOverlay::Calendar => calendar::render_calendar_modal(frame, state),
        ActiveOverlay::Remote => remote::render_remote_modal(frame, state),
        ActiveOverlay::Cast => cast::render_cast_modal(frame, state),
        ActiveOverlay::Download => download::render_download_modal(frame, state),
        ActiveOverlay::Update => update_modal::render_update_modal(frame, state),
        ActiveOverlay::ConfirmQuit => help::render_confirm_quit_modal(frame, state),
    }

    if let Some((ref msg, _)) = state.toast {
        help::render_toast(frame, msg, state);
    }
}
