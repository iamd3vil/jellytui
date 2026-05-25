mod app;
mod config;
mod download;
mod events;
mod images;
mod player;
mod ui;

use std::{
    io,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::{App, PlayingItem, Screen};
use crate::config::Config;
use crate::download::{DownloadEvent, perform_download};
use crate::events::{Event, EventHandler};
use crate::images::{ImageFetched, ImageManager};
use crate::player::{PlayerEvent, monitor_playback};
use jellyfin_client::{PlaybackProgressInfo, PlaybackStartInfo, PlaybackStopInfo};

const TICKS_PER_SECOND: u64 = 10_000_000;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().unwrap_or_default();

    enable_raw_mode()?;

    let (image_tx, image_rx) = mpsc::unbounded_channel::<ImageFetched>();
    let mut images = match ImageManager::new(image_tx) {
        Ok(m) => m,
        Err(e) => {
            disable_raw_mode().ok();
            eprintln!("Failed to initialize image renderer: {e}");
            eprintln!(
                "Your terminal may not support a graphics protocol. Try Kitty, Ghostty, WezTerm, or a Sixel-capable terminal."
            );
            return Ok(());
        }
    };

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let mut events = EventHandler::new(Duration::from_millis(250));

    if app.screen == Screen::Home {
        let _ = app.load_home_content().await;
    }

    let result = run_app(&mut terminal, &mut app, &mut events, &mut images, image_rx).await;

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
    images: &mut ImageManager,
    mut image_rx: mpsc::UnboundedReceiver<ImageFetched>,
) -> Result<()> {
    let (player_tx, mut player_rx) = mpsc::unbounded_channel::<PlayerEvent>();
    let (download_tx, mut download_rx) = mpsc::unbounded_channel::<DownloadEvent>();

    while app.running {
        terminal.draw(|frame| ui::render(frame, app, images))?;

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
            Some(image_event) = image_rx.recv() => {
                images.handle_fetched(image_event);
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
        KeyCode::Left | KeyCode::Char('h') => {
            app.move_left();
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.move_right();
        }
        KeyCode::Enter => {
            if let Ok(Some(playing_item)) = app.select_item().await {
                start_playback(app, playing_item, player_tx).await;
            }
        }
        KeyCode::Esc | KeyCode::Backspace => {
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
            app.move_up();
        }
        KeyCode::Down => {
            app.move_down();
        }
        KeyCode::Left => {
            app.move_left();
        }
        KeyCode::Right => {
            app.move_right();
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
    app.playback_session_id = app.playback_session_id.wrapping_add(1);
    let session_id = app.playback_session_id;
    let start_position_secs = playing_item.start_position_ticks as f64 / TICKS_PER_SECOND as f64;

    let stream_url = match &app.client {
        Some(client) => match client.get_stream_url(&playing_item.item.id) {
            Ok(url) => url,
            Err(_) => return,
        },
        None => return,
    };

    let external_sub_urls: Vec<String> = match &app.client {
        Some(client) => client
            .get_external_subtitles(&playing_item.item.id)
            .await
            .map(|subs| subs.into_iter().map(|s| s.url).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };

    if let Err(e) = app
        .player
        .start(&stream_url, Some(start_position_secs), &external_sub_urls)
    {
        app.error_message = Some(format!(
            "Failed to start player: {}. MPV log: {}",
            e,
            app.player.log_path().display()
        ));
        return;
    }

    if let Some(client) = app.client.clone() {
        let start_info = PlaybackStartInfo {
            item_id: playing_item.item.id.clone(),
            position_ticks: playing_item.start_position_ticks,
            is_paused: false,
            is_muted: false,
            volume_level: 100,
            play_method: "DirectStream".to_string(),
        };
        tokio::spawn(async move {
            let _ = client.report_playback_start(&start_info).await;
        });
    }

    app.playback_position_secs = start_position_secs;
    app.playback_duration_secs = playing_item
        .item
        .run_time_ticks
        .map(|ticks| ticks as f64 / TICKS_PER_SECOND as f64)
        .unwrap_or(0.0);
    app.playback_paused = false;
    app.last_progress_report = None;
    app.marked_as_played = false;
    app.now_playing = Some(playing_item);

    let socket_path = app.player.socket_path().clone();
    let log_path = app.player.log_path().clone();
    tokio::spawn(async move {
        monitor_playback(socket_path, log_path, session_id, player_tx).await;
    });
}

const PROGRESS_REPORT_INTERVAL_SECS: u64 = 5;

async fn handle_player_event(app: &mut App, event: PlayerEvent) {
    match event {
        PlayerEvent::Progress { session_id, state } => {
            if session_id != app.playback_session_id {
                return;
            }

            let position_ticks = (state.position_secs * TICKS_PER_SECOND as f64) as u64;
            app.last_position_ticks = position_ticks;
            app.playback_position_secs = state.position_secs;
            app.playback_duration_secs = state.duration_secs;
            app.playback_paused = state.paused;

            let should_report = app
                .last_progress_report
                .map(|t| t.elapsed().as_secs() >= PROGRESS_REPORT_INTERVAL_SECS)
                .unwrap_or(true);

            if should_report {
                if let (Some(playing), Some(client)) = (app.now_playing.clone(), app.client.clone())
                {
                    app.last_progress_report = Some(Instant::now());

                    let progress_info = PlaybackProgressInfo {
                        item_id: playing.item.id.clone(),
                        position_ticks,
                        is_paused: state.paused,
                        is_muted: false,
                        volume_level: 100,
                    };

                    tokio::spawn(async move {
                        let _ = client.report_playback_progress(&progress_info).await;
                    });
                }
            }

            if !app.marked_as_played {
                if let Some(playing) = app.now_playing.as_ref() {
                    if let Some(duration_ticks) = playing.item.run_time_ticks {
                        let progress_percent =
                            position_ticks as f64 / duration_ticks as f64 * 100.0;
                        if progress_percent >= 90.0 {
                            app.marked_as_played = true;
                            if let Some(client) = app.client.clone() {
                                let item_id = playing.item.id.clone();
                                tokio::spawn(async move {
                                    let _ = client.mark_played(&item_id).await;
                                });
                            }
                        }
                    }
                }
            }
        }
        PlayerEvent::Finished { session_id } => {
            if session_id != app.playback_session_id {
                return;
            }
            handle_playback_finished(app).await;
        }
        PlayerEvent::Error {
            session_id,
            message,
        } => {
            if session_id != app.playback_session_id {
                return;
            }

            if app.player.is_running() {
                return;
            }

            app.error_message = Some(format!(
                "Player error: {}. MPV log: {}",
                message,
                app.player.log_path().display()
            ));
            handle_playback_finished(app).await;
        }
    }
}

async fn handle_playback_finished(app: &mut App) {
    app.playback_session_id = app.playback_session_id.wrapping_add(1);

    if let (Some(playing), Some(client)) = (app.now_playing.take(), app.client.clone()) {
        let position_ticks = app.last_position_ticks;
        tokio::spawn(async move {
            let stop_info = PlaybackStopInfo {
                item_id: playing.item.id.clone(),
                position_ticks,
            };
            let _ = client.report_playback_stop(&stop_info).await;
        });
    }
    app.last_position_ticks = 0;
    app.playback_position_secs = 0.0;
    app.playback_duration_secs = 0.0;
    app.playback_paused = false;
    app.last_progress_report = None;
    app.marked_as_played = false;
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
