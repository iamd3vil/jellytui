use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};
use ratatui_image::{Resize, StatefulImage, protocol::StatefulProtocol};

use crate::app::{App, LoginField, Screen};
use crate::download::DownloadStatus;
use crate::images::ImageManager;
use jellyfin_client::MediaItem;

const POSTER_WIDTH: u16 = 18;
const POSTER_IMG_HEIGHT: u16 = 10;
const POSTER_LABEL_HEIGHT: u16 = 3;
const POSTER_TOTAL_HEIGHT: u16 = POSTER_IMG_HEIGHT + POSTER_LABEL_HEIGHT;
const SECTION_HEADER_HEIGHT: u16 = 2;
const SECTION_HEIGHT: u16 = SECTION_HEADER_HEIGHT + POSTER_TOTAL_HEIGHT;
const POSTER_GAP: u16 = 1;

pub fn render(frame: &mut Frame, app: &mut App, images: &mut ImageManager) {
    match app.screen {
        Screen::Login => render_login(frame, app),
        Screen::Home => render_home(frame, app, images),
        Screen::Library => render_library(frame, app, images),
        Screen::Search => render_search(frame, app, images),
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

fn frame_layout(
    frame_area: Rect,
    now_playing: bool,
    search_bar: bool,
) -> (Option<Rect>, Rect, Rect, Option<Rect>) {
    let mut constraints = Vec::new();
    if search_bar {
        constraints.push(Constraint::Length(3));
    }
    constraints.push(Constraint::Min(3));
    constraints.push(Constraint::Length(1));
    if now_playing {
        constraints.push(Constraint::Length(3));
    } else {
        constraints.push(Constraint::Length(0));
    }
    let chunks = Layout::vertical(constraints).split(frame_area);

    let mut idx = 0;
    let search_rect = if search_bar {
        let r = chunks[idx];
        idx += 1;
        Some(r)
    } else {
        None
    };
    let content = chunks[idx];
    let help = chunks[idx + 1];
    let footer = if now_playing { Some(chunks[idx + 2]) } else { None };
    (search_rect, content, help, footer)
}

fn render_home(frame: &mut Frame, app: &mut App, images: &mut ImageManager) {
    let area = frame.area();
    let (_, content, help_area, footer_area) =
        frame_layout(area, app.now_playing.is_some(), false);

    let title = format!("Jellytui — {}", app.current_title());
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(content);
    frame.render_widget(block, content);

    if app.loading {
        let loading = Paragraph::new("Loading...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, inner);
    } else if let Some(ref error) = app.error_message {
        let error_text =
            Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, inner);
    } else if app.home_sections.is_empty() {
        let empty =
            Paragraph::new("No content yet. Press 'r' to refresh.").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, inner);
    } else {
        render_home_sections(frame, app, images, inner);
    }

    render_help(frame, help_area, home_help_text());
    if let Some(footer) = footer_area {
        if app.now_playing.is_some() && footer.height > 0 {
            render_now_playing_footer(frame, app, footer);
        }
    }
}

fn render_home_sections(
    frame: &mut Frame,
    app: &mut App,
    images: &mut ImageManager,
    area: Rect,
) {
    let visible_section_count = (area.height / SECTION_HEIGHT).max(1) as usize;
    let total_sections = app.home_sections.len();
    let scroll = if app.home_row + 1 > visible_section_count {
        app.home_row + 1 - visible_section_count
    } else {
        0
    };
    let scroll = scroll.min(total_sections.saturating_sub(visible_section_count));

    let mut y = area.y;
    let max_y = area.y + area.height;

    let end = (scroll + visible_section_count).min(total_sections);
    for section_idx in scroll..end {
        if y >= max_y {
            break;
        }

        let section_title = app.home_sections[section_idx].title.clone();
        let header_rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height: SECTION_HEADER_HEIGHT.min(max_y.saturating_sub(y)),
        };
        let is_selected_row = section_idx == app.home_row;
        let header_color = if is_selected_row {
            Color::Yellow
        } else {
            Color::Gray
        };
        let header = Paragraph::new(Line::from(vec![Span::styled(
            section_title,
            Style::default()
                .fg(header_color)
                .add_modifier(Modifier::BOLD),
        )]));
        frame.render_widget(header, header_rect);

        let row_rect = Rect {
            x: area.x,
            y: y.saturating_add(SECTION_HEADER_HEIGHT),
            width: area.width,
            height: POSTER_TOTAL_HEIGHT.min(max_y.saturating_sub(y + SECTION_HEADER_HEIGHT)),
        };

        if row_rect.height > 0 {
            render_poster_row(
                frame,
                app,
                images,
                section_idx,
                row_rect,
                is_selected_row,
            );
        }

        y = y.saturating_add(SECTION_HEIGHT);
    }
}

fn render_poster_row(
    frame: &mut Frame,
    app: &mut App,
    images: &mut ImageManager,
    section_idx: usize,
    area: Rect,
    is_selected_row: bool,
) {
    let section = match app.home_sections.get(section_idx) {
        Some(s) => s.clone(),
        None => return,
    };

    let item_count = section.items.len();
    if item_count == 0 {
        return;
    }

    let cell_width = POSTER_WIDTH + POSTER_GAP;
    let visible_cols = (area.width / cell_width).max(1) as usize;

    let selected_col = if is_selected_row { app.home_col } else { usize::MAX };

    let col_scroll = if is_selected_row {
        if app.home_col + 1 > visible_cols {
            app.home_col + 1 - visible_cols
        } else {
            0
        }
    } else {
        0
    };

    for col_offset in 0..visible_cols {
        let idx = col_scroll + col_offset;
        if idx >= item_count {
            break;
        }
        let item = &section.items[idx];
        let x = area.x + col_offset as u16 * cell_width;
        if x + POSTER_WIDTH > area.x + area.width {
            break;
        }
        let cell = Rect {
            x,
            y: area.y,
            width: POSTER_WIDTH,
            height: area.height,
        };

        let is_selected = idx == selected_col;
        let url = app.client.as_ref().map(|c| c.get_primary_image_url(&item.id, 240));
        render_poster_cell(frame, images, item, url, cell, is_selected);
    }
}

fn render_poster_cell(
    frame: &mut Frame,
    images: &mut ImageManager,
    item: &MediaItem,
    image_url: Option<String>,
    area: Rect,
    is_selected: bool,
) {
    if area.height < 3 || area.width < 4 {
        return;
    }

    let img_height = POSTER_IMG_HEIGHT.min(area.height.saturating_sub(POSTER_LABEL_HEIGHT));
    let img_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: img_height,
    };
    let label_area = Rect {
        x: area.x,
        y: area.y + img_height,
        width: area.width,
        height: area.height.saturating_sub(img_height),
    };

    let border_style = if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let poster_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let poster_inner = poster_block.inner(img_area);
    frame.render_widget(poster_block, img_area);

    if let Some(url) = image_url {
        images.ensure(&item.id, url);
    }

    if let Some(protocol) = images.get_mut(&item.id) {
        let widget = StatefulImage::<StatefulProtocol>::default().resize(Resize::Fit(None));
        frame.render_stateful_widget(widget, poster_inner, protocol);
    } else if images.is_failed(&item.id) {
        let placeholder = Paragraph::new(type_icon(item))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, centered_in(poster_inner, 3, 1));
    } else {
        let placeholder = Paragraph::new("...")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, centered_in(poster_inner, 3, 1));
    }

    let display_name = poster_label(item);
    let year = item
        .production_year
        .map(|y| format!("{}", y))
        .unwrap_or_default();
    let name_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let label_lines = vec![
        Line::from(Span::styled(display_name, name_style)),
        Line::from(Span::styled(
            year,
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let label = Paragraph::new(label_lines)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    frame.render_widget(label, label_area);
}

fn poster_label(item: &MediaItem) -> String {
    if item.r#type == "Episode" {
        match (&item.series_name, &item.parent_index_number, &item.index_number) {
            (Some(series), Some(s), Some(e)) => format!("{} S{:02}E{:02}", series, s, e),
            (Some(series), _, _) => series.clone(),
            _ => item.name.clone(),
        }
    } else {
        item.name.clone()
    }
}

fn type_icon(item: &MediaItem) -> &'static str {
    match item.r#type.as_str() {
        "Movie" => "Movie",
        "Series" => "Series",
        "Season" => "Season",
        "Episode" => "Episode",
        "Audio" => "Audio",
        "MusicAlbum" => "Album",
        "MusicArtist" => "Artist",
        "Folder" | "CollectionFolder" => "Folder",
        _ => "?",
    }
}

fn centered_in(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn render_library(frame: &mut Frame, app: &mut App, images: &mut ImageManager) {
    let area = frame.area();
    let (_, content, help_area, footer_area) =
        frame_layout(area, app.now_playing.is_some(), false);

    let title = format!("Jellytui — {}", app.current_title());
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(content);
    frame.render_widget(block, content);

    if app.loading {
        frame.render_widget(
            Paragraph::new("Loading...").style(Style::default().fg(Color::Yellow)),
            inner,
        );
    } else if let Some(ref error) = app.error_message {
        frame.render_widget(
            Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red)),
            inner,
        );
    } else if app.items.is_empty() {
        frame.render_widget(
            Paragraph::new("Empty.").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
    } else {
        let items = app.items.clone();
        render_grid(frame, app, images, &items, app.selected_index, inner);
    }

    render_help(frame, help_area, library_help_text());
    if let Some(footer) = footer_area {
        if app.now_playing.is_some() && footer.height > 0 {
            render_now_playing_footer(frame, app, footer);
        }
    }
}

fn render_search(frame: &mut Frame, app: &mut App, images: &mut ImageManager) {
    let area = frame.area();
    let (search_rect, content, help_area, footer_area) =
        frame_layout(area, app.now_playing.is_some(), true);

    if let Some(rect) = search_rect {
        let search_block = Block::default()
            .title("Search")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let search_input = Paragraph::new(app.search_query.as_str()).block(search_block);
        frame.render_widget(search_input, rect);
        frame.set_cursor_position((rect.x + app.search_query.len() as u16 + 1, rect.y + 1));
    }

    let results_block = Block::default()
        .title(format!("Results ({})", app.search_results.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = results_block.inner(content);
    frame.render_widget(results_block, content);

    if app.loading {
        frame.render_widget(
            Paragraph::new("Searching...").style(Style::default().fg(Color::Yellow)),
            inner,
        );
    } else if let Some(ref error) = app.error_message {
        frame.render_widget(
            Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red)),
            inner,
        );
    } else if app.search_results.is_empty() {
        let msg = if app.search_query.is_empty() {
            "Type to search..."
        } else {
            "No results found"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
            inner,
        );
    } else {
        let items = app.search_results.clone();
        render_grid(frame, app, images, &items, app.search_selected, inner);
    }

    render_help(frame, help_area, search_help_text());
    if let Some(footer) = footer_area {
        if app.now_playing.is_some() && footer.height > 0 {
            render_now_playing_footer(frame, app, footer);
        }
    }
}

fn render_grid(
    frame: &mut Frame,
    app: &mut App,
    images: &mut ImageManager,
    items: &[MediaItem],
    selected: usize,
    area: Rect,
) {
    if area.width < POSTER_WIDTH || area.height < POSTER_TOTAL_HEIGHT {
        return;
    }

    let cell_w = POSTER_WIDTH + POSTER_GAP;
    let cell_h = POSTER_TOTAL_HEIGHT + 1;
    let cols = (area.width / cell_w).max(1) as usize;
    let rows = (area.height / cell_h).max(1) as usize;

    app.grid_columns = cols;

    let total = items.len();
    let selected_row = selected / cols;
    let row_scroll = if selected_row + 1 > rows {
        selected_row + 1 - rows
    } else {
        0
    };

    for row_offset in 0..rows {
        let row_idx = row_scroll + row_offset;
        let start = row_idx * cols;
        if start >= total {
            break;
        }
        for col in 0..cols {
            let idx = start + col;
            if idx >= total {
                break;
            }
            let x = area.x + col as u16 * cell_w;
            let y = area.y + row_offset as u16 * cell_h;
            if x + POSTER_WIDTH > area.x + area.width
                || y + POSTER_TOTAL_HEIGHT > area.y + area.height
            {
                continue;
            }
            let cell = Rect {
                x,
                y,
                width: POSTER_WIDTH,
                height: POSTER_TOTAL_HEIGHT,
            };
            let item = &items[idx];
            let url = app
                .client
                .as_ref()
                .map(|c| c.get_primary_image_url(&item.id, 240));
            render_poster_cell(frame, images, item, url, cell, idx == selected);
        }
    }
}

fn render_help(frame: &mut Frame, area: Rect, text: &str) {
    if area.height == 0 {
        return;
    }
    let help = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

fn home_help_text() -> &'static str {
    "h/j/k/l: navigate | Enter: open | /: search | d: downloads | r: refresh | q: quit"
}

fn library_help_text() -> &'static str {
    "h/j/k/l: navigate | Enter: open/play | D: download | d: downloads | Esc: back | q: quit"
}

fn search_help_text() -> &'static str {
    "Type to search | h/j/k/l or arrows: navigate | Enter: open/play | Esc: close"
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

    let status = if app.playback_paused { "Paused" } else { "Playing" };
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

    let display = poster_label(&playing.item);
    let title = format!("Now Playing — {}", display);
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

