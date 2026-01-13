mod app;
mod client;
mod config;
mod download;
mod events;
mod player;
mod ui;

use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::{App, PlayingItem, Screen};
use crate::client::{PlaybackProgressInfo, PlaybackStartInfo, PlaybackStopInfo};
use crate::config::Config;
use crate::download::{DownloadEvent, perform_download};
use crate::events::{Event, EventHandler};
use crate::player::{PlayerEvent, monitor_playback};

const TICKS_PER_SECOND: u64 = 10_000_000;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().unwrap_or_default();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let mut events = EventHandler::new(Duration::from_millis(250));

    if app.screen == Screen::Home {
        let _ = app.load_home_content().await;
    }

    let result = run_app(&mut terminal, &mut app, &mut events).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e:?}");
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventHandler,
) -> Result<()> {
    let (player_tx, mut player_rx) = mpsc::unbounded_channel::<PlayerEvent>();
    let (download_tx, mut download_rx) = mpsc::unbounded_channel::<DownloadEvent>();

    while app.running {
        terminal.draw(|frame| ui::render(frame, app))?;

        tokio::select! {
            event = events.next() => {
                match event? {
                    Event::Key(key) => match app.screen {
                        Screen::Login => handle_login_input(app, key.code, key.modifiers).await,
                        Screen::Home | Screen::Library => {
                            handle_browser_input(app, key.code, key.modifiers, player_tx.clone(), download_tx.clone()).await
                        }
                        Screen::Search => {
                            handle_search_input(app, key.code, key.modifiers, player_tx.clone(), download_tx.clone()).await
                        }
                    },
                    Event::Tick => {
                        if !app.player.is_running() && app.now_playing.is_some() {
                            handle_playback_finished(app).await;
                        }
                    }
                }
            }
            Some(player_event) = player_rx.recv() => {
                handle_player_event(app, player_event).await;
            }
            Some(download_event) = download_rx.recv() => {
                handle_download_event(app, download_event);
            }
        }
    }

    if app.now_playing.is_some() {
        handle_playback_finished(app).await;
    }

    Ok(())
}

async fn handle_login_input(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Char('q') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
        }
        KeyCode::Tab => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                app.login_field = app.login_field.prev();
            } else {
                app.login_field = app.login_field.next();
            }
        }
        KeyCode::BackTab => {
            app.login_field = app.login_field.prev();
        }
        KeyCode::Enter => {
            let _ = app.attempt_login().await;
        }
        KeyCode::Backspace => {
            app.current_input_mut().pop();
        }
        KeyCode::Char(c) => {
            app.current_input_mut().push(c);
        }
        KeyCode::Esc => {
            app.quit();
        }
        _ => {}
    }
}

async fn handle_browser_input(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    player_tx: mpsc::UnboundedSender<PlayerEvent>,
    download_tx: mpsc::UnboundedSender<DownloadEvent>,
) {
    match code {
        KeyCode::Char('q') => {
            app.quit();
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_down();
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Ok(Some(playing_item)) = app.select_item().await {
                start_playback(app, playing_item, player_tx).await;
            }
        }
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
            if app.screen == Screen::Library {
                let _ = app.go_back().await;
            }
        }
        KeyCode::Char('r') => match app.screen {
            Screen::Home => {
                let _ = app.load_home_content().await;
            }
            Screen::Library => {
                if let Some(entry) = app.nav_stack.last() {
                    let parent_id = entry.parent_id.clone();
                    let _ = app.load_items(&parent_id).await;
                }
            }
            Screen::Login | Screen::Search => {}
        },
        KeyCode::Char('/') | KeyCode::Char('s') => {
            app.open_search();
        }
        KeyCode::Char('d') => {
            app.toggle_downloads();
        }
        KeyCode::Char('D') => {
            if let Some(task) = app.queue_download() {
                let tx = download_tx.clone();
                tokio::spawn(async move {
                    perform_download(task, tx).await;
                });
            }
        }
        _ => {}
    }
}

async fn handle_search_input(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    player_tx: mpsc::UnboundedSender<PlayerEvent>,
    download_tx: mpsc::UnboundedSender<DownloadEvent>,
) {
    match code {
        KeyCode::Esc => {
            app.close_search();
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
        }
        KeyCode::Up => {
            app.search_move_up();
        }
        KeyCode::Down => {
            app.search_move_down();
        }
        KeyCode::Enter => {
            if !app.search_results.is_empty()
                && let Ok(Some(playing_item)) = app.select_item().await
            {
                start_playback(app, playing_item, player_tx).await;
            }
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            let _ = app.perform_search().await;
        }
        KeyCode::Char('D') if modifiers.contains(KeyModifiers::SHIFT) => {
            if let Some(task) = app.queue_download() {
                let tx = download_tx.clone();
                tokio::spawn(async move {
                    perform_download(task, tx).await;
                });
            }
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
            let _ = app.perform_search().await;
        }
        _ => {}
    }
}

async fn start_playback(
    app: &mut App,
    playing_item: PlayingItem,
    player_tx: mpsc::UnboundedSender<PlayerEvent>,
) {
    let start_position_secs = playing_item.start_position_ticks as f64 / TICKS_PER_SECOND as f64;

    let stream_url = match &app.client {
        Some(client) => match client.get_stream_url(&playing_item.item.id) {
            Ok(url) => url,
            Err(_) => return,
        },
        None => return,
    };

    if let Err(e) = app.player.start(&stream_url, Some(start_position_secs)) {
        app.error_message = Some(format!("Failed to start player: {}", e));
        return;
    }

    let mut start_error = None;
    if let Some(ref client) = app.client {
        let start_info = PlaybackStartInfo {
            item_id: playing_item.item.id.clone(),
            position_ticks: playing_item.start_position_ticks,
            is_paused: false,
            is_muted: false,
            volume_level: 100,
            play_method: "DirectStream".to_string(),
        };
        start_error = client.report_playback_start(&start_info).await.err();
    }
    if let Some(e) = start_error {
        if app.handle_unauthorized(&e) {
            return;
        }
    }

    app.playback_position_secs = start_position_secs;
    app.playback_duration_secs = playing_item
        .item
        .run_time_ticks
        .map(|ticks| ticks as f64 / TICKS_PER_SECOND as f64)
        .unwrap_or(0.0);
    app.playback_paused = false;
    app.now_playing = Some(playing_item);

    let socket_path = app.player.socket_path().clone();
    tokio::spawn(async move {
        monitor_playback(socket_path, player_tx).await;
    });
}

async fn handle_player_event(app: &mut App, event: PlayerEvent) {
    match event {
        PlayerEvent::Progress(state) => {
            if let Some(playing) = app.now_playing.clone() {
                let position_ticks = (state.position_secs * TICKS_PER_SECOND as f64) as u64;
                app.last_position_ticks = position_ticks;
                app.playback_position_secs = state.position_secs;
                app.playback_duration_secs = state.duration_secs;
                app.playback_paused = state.paused;

                let mut auth_error = None;
                if let Some(ref client) = app.client {
                    let progress_info = PlaybackProgressInfo {
                        item_id: playing.item.id.clone(),
                        position_ticks,
                        is_paused: state.paused,
                        is_muted: false,
                        volume_level: 100,
                    };
                    auth_error = client.report_playback_progress(&progress_info).await.err();

                    if auth_error.is_none() {
                        if let Some(duration_ticks) = playing.item.run_time_ticks {
                            let progress_percent =
                                position_ticks as f64 / duration_ticks as f64 * 100.0;
                            if progress_percent >= 90.0 {
                                auth_error = client.mark_played(&playing.item.id).await.err();
                            }
                        }
                    }
                }
                if let Some(e) = auth_error {
                    if app.handle_unauthorized(&e) {
                        return;
                    }
                }
            }
        }
        PlayerEvent::Finished => {
            handle_playback_finished(app).await;
        }
        PlayerEvent::Error(_) => {
            handle_playback_finished(app).await;
        }
    }
}

async fn handle_playback_finished(app: &mut App) {
    let mut stop_error = None;
    if let Some(playing) = app.now_playing.take()
        && let Some(ref client) = app.client
    {
        let stop_info = PlaybackStopInfo {
            item_id: playing.item.id.clone(),
            position_ticks: app.last_position_ticks,
        };
        stop_error = client.report_playback_stop(&stop_info).await.err();
    }
    if let Some(e) = stop_error {
        if app.handle_unauthorized(&e) {
            return;
        }
    }
    app.last_position_ticks = 0;
    app.playback_position_secs = 0.0;
    app.playback_duration_secs = 0.0;
    app.playback_paused = false;
    app.player.stop();
}

fn handle_download_event(app: &mut App, event: DownloadEvent) {
    match event {
        DownloadEvent::Started { item_id } => {
            app.mark_download_started(&item_id);
        }
        DownloadEvent::Progress { item_id, progress } => {
            app.update_download_progress(&item_id, progress);
        }
        DownloadEvent::Completed { item_id } => {
            app.mark_download_completed(&item_id);
        }
        DownloadEvent::Failed { item_id, error } => {
            app.mark_download_failed(&item_id, error);
        }
    }
}
