use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, HomeDisplayItem, LoginField, Screen};
use crate::client::MediaItem;
use crate::download::DownloadStatus;

pub fn render(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Login => render_login(frame, app),
        Screen::Home => render_browser(frame, app),
        Screen::Library => render_browser(frame, app),
        Screen::Search => render_search(frame, app),
    }

    if app.show_downloads {
        render_downloads_popup(frame, app);
    }
}

fn render_login(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let outer_block = Block::default()
        .title("Jellytui - Login")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(outer_block, area);

    let inner_area = centered_rect(60, 50, area);

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(inner_area);

    let server_style = field_style(app.login_field == LoginField::ServerUrl);
    let server_block = Block::default()
        .title("Server URL")
        .borders(Borders::ALL)
        .border_style(server_style);
    let server_input = Paragraph::new(app.server_url_input.as_str()).block(server_block);
    frame.render_widget(server_input, chunks[0]);

    let username_style = field_style(app.login_field == LoginField::Username);
    let username_block = Block::default()
        .title("Username")
        .borders(Borders::ALL)
        .border_style(username_style);
    let username_input = Paragraph::new(app.username_input.as_str()).block(username_block);
    frame.render_widget(username_input, chunks[1]);

    let password_style = field_style(app.login_field == LoginField::Password);
    let password_block = Block::default()
        .title("Password")
        .borders(Borders::ALL)
        .border_style(password_style);
    let masked_password = "*".repeat(app.password_input.len());
    let password_input = Paragraph::new(masked_password).block(password_block);
    frame.render_widget(password_input, chunks[2]);

    let help_text = Line::from(vec![
        Span::raw("Tab: next field | Shift+Tab: prev | Enter: login | "),
        Span::styled("Esc: quit", Style::default().fg(Color::Red)),
    ]);
    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);

    if let Some(ref error) = app.login_error {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        frame.render_widget(error_text, chunks[4]);
    }

    if app.login_loading {
        let loading = Paragraph::new("Authenticating...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, chunks[4]);
    }

    set_cursor_for_input(frame, app, chunks);
}

fn render_browser(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let footer_height = if app.now_playing.is_some() { 3 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(footer_height),
    ])
    .split(area);

    let title = format!("Jellytui - {}", app.current_title());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if app.loading {
        let loading = Paragraph::new("Loading...")
            .style(Style::default().fg(Color::Yellow))
            .block(block);
        frame.render_widget(loading, chunks[0]);
    } else if let Some(ref error) = app.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .block(block);
        frame.render_widget(error_text, chunks[0]);
    } else {
        let list_items: Vec<ListItem> = match app.screen {
            Screen::Home => app
                .home_items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let (content, style) = match item {
                        HomeDisplayItem::Header(title) => (
                            title.clone(),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        ),
                        HomeDisplayItem::Library(lib) => (
                            format_item(lib),
                            if i == app.selected_index {
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                        HomeDisplayItem::Item(media) => (
                            format_item(media),
                            if i == app.selected_index {
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                    };
                    ListItem::new(content).style(style)
                })
                .collect(),
            Screen::Library => app
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let content = format_item(item);
                    let style = if i == app.selected_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(content).style(style)
                })
                .collect(),
            Screen::Login | Screen::Search => vec![],
        };

        let list = List::new(list_items).block(block);

        let mut state = ListState::default();
        state.select(Some(app.selected_index));

        frame.render_stateful_widget(list, chunks[0], &mut state);
    }

    let help_text = match app.screen {
        Screen::Home => "j/k: navigate | Enter: open | /: search | d: downloads | q: quit",
        Screen::Library => {
            "j/k: navigate | Enter: open/play | D: download | d: downloads | Esc: back | q: quit"
        }
        Screen::Login | Screen::Search => "",
    };
    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[1]);

    if app.now_playing.is_some() && chunks[2].height > 0 && chunks[2].width > 0 {
        render_now_playing_footer(frame, app, chunks[2]);
    }
}

fn format_item(item: &MediaItem) -> String {
    let type_icon = match item.r#type.as_str() {
        "Movie" => "[M]",
        "Series" => "[S]",
        "Season" => "[Sn]",
        "Episode" => "[E]",
        "Audio" => "[A]",
        "MusicAlbum" => "[Al]",
        "MusicArtist" => "[Ar]",
        "Folder" | "CollectionFolder" => "[D]",
        _ => "[ ]",
    };

    let year = item
        .production_year
        .map(|y| format!(" ({})", y))
        .unwrap_or_default();

    let episode_info = match (&item.parent_index_number, &item.index_number) {
        (Some(s), Some(e)) => format!(" S{:02}E{:02}", s, e),
        (None, Some(e)) => format!(" E{}", e),
        _ => String::new(),
    };

    format!("{} {}{}{}", type_icon, item.name, episode_info, year)
}

fn render_now_playing_footer(frame: &mut Frame, app: &App, area: Rect) {
    let Some(ref playing) = app.now_playing else {
        return;
    };

    let duration = app.playback_duration_secs;
    let percent = if duration > 0.0 {
        (app.playback_position_secs / duration * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let status = if app.playback_paused {
        "Paused"
    } else {
        "Playing"
    };
    let label = if duration > 0.0 {
        format!(
            "{} / {}  •  {}",
            format_duration(app.playback_position_secs),
            format_duration(duration),
            status
        )
    } else {
        format!(
            "{}  •  {}",
            format_duration(app.playback_position_secs),
            status
        )
    };

    let title = format!("Now Playing - {}", format_item(&playing.item));
    let gauge_color = if app.playback_paused {
        Color::Yellow
    } else {
        Color::Green
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .gauge_style(Style::default().fg(gauge_color).bg(Color::DarkGray))
        .percent(percent.round() as u16)
        .label(Span::styled(label, Style::default().fg(Color::White)));

    frame.render_widget(gauge, area);
}

fn format_duration(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0).floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;

    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

fn field_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

fn set_cursor_for_input(frame: &mut Frame, app: &App, chunks: std::rc::Rc<[Rect]>) {
    let (chunk_idx, input_len) = match app.login_field {
        LoginField::ServerUrl => (0, app.server_url_input.len()),
        LoginField::Username => (1, app.username_input.len()),
        LoginField::Password => (2, app.password_input.len()),
    };

    let chunk = chunks[chunk_idx];
    frame.set_cursor_position((chunk.x + input_len as u16 + 1, chunk.y + 1));
}

fn render_search(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let footer_height = if app.now_playing.is_some() { 3 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(footer_height),
    ])
    .split(area);

    let search_block = Block::default()
        .title("Search")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let search_input = Paragraph::new(app.search_query.as_str()).block(search_block);
    frame.render_widget(search_input, chunks[0]);

    frame.set_cursor_position((
        chunks[0].x + app.search_query.len() as u16 + 1,
        chunks[0].y + 1,
    ));

    let results_block = Block::default()
        .title(format!("Results ({})", app.search_results.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if app.loading {
        let loading = Paragraph::new("Searching...")
            .style(Style::default().fg(Color::Yellow))
            .block(results_block);
        frame.render_widget(loading, chunks[1]);
    } else if let Some(ref error) = app.error_message {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .block(results_block);
        frame.render_widget(error_text, chunks[1]);
    } else if app.search_results.is_empty() {
        let empty_msg = if app.search_query.is_empty() {
            "Type to search..."
        } else {
            "No results found"
        };
        let empty = Paragraph::new(empty_msg)
            .style(Style::default().fg(Color::DarkGray))
            .block(results_block);
        frame.render_widget(empty, chunks[1]);
    } else {
        let list_items: Vec<ListItem> = app
            .search_results
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let content = format_item(item);
                let style = if i == app.search_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(list_items).block(results_block);

        let mut state = ListState::default();
        state.select(Some(app.search_selected));

        frame.render_stateful_widget(list, chunks[1], &mut state);
    }

    let help = Paragraph::new("Type to search | Up/Down: navigate | Enter: open/play | Esc: close")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);

    if app.now_playing.is_some() && chunks[3].height > 0 && chunks[3].width > 0 {
        render_now_playing_footer(frame, app, chunks[3]);
    }
}

fn render_downloads_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_area = centered_rect(70, 60, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!("Downloads ({})", app.downloads.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    if app.downloads.is_empty() {
        let empty = Paragraph::new("No downloads. Press D on a media item to download.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty, popup_area);
        return;
    }

    let inner_area = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::vertical(
        app.downloads
            .iter()
            .map(|_| Constraint::Length(3))
            .collect::<Vec<_>>(),
    )
    .split(inner_area);

    for (i, task) in app.downloads.iter().enumerate() {
        if i >= chunks.len() {
            break;
        }

        let (status_text, status_color) = match task.status {
            DownloadStatus::Pending => ("Pending", Color::DarkGray),
            DownloadStatus::Downloading => ("Downloading", Color::Yellow),
            DownloadStatus::Completed => ("Completed", Color::Green),
            DownloadStatus::Failed => ("Failed", Color::Red),
        };

        let title = format!("{} [{}]", task.item_name, status_text);

        let gauge = Gauge::default()
            .block(Block::default().title(title).borders(Borders::ALL))
            .gauge_style(Style::default().fg(status_color))
            .percent(task.progress as u16)
            .label(format!("{}%", task.progress));

        frame.render_widget(gauge, chunks[i]);
    }
}
