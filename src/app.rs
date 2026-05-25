use std::time::Instant;

use jellyfin_client::{ClientInfo, Error as JellyfinError, JellyfinClient, MediaItem};

use crate::config::Config;
use crate::download::{DownloadManager, DownloadStatus, DownloadTask};
use crate::player::MpvPlayer;

fn client_info() -> ClientInfo {
    ClientInfo::new("jellytui", env!("CARGO_PKG_VERSION")).with_device_id("jellytui-rust")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Home,
    Library,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSectionKind {
    Libraries,
    Items,
}

#[derive(Debug, Clone)]
pub struct HomeSection {
    pub title: String,
    pub kind: HomeSectionKind,
    pub items: Vec<MediaItem>,
}

#[derive(Debug, Clone)]
pub struct PlayingItem {
    pub item: MediaItem,
    pub start_position_ticks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginField {
    ServerUrl,
    Username,
    Password,
}

impl LoginField {
    pub fn next(self) -> Self {
        match self {
            Self::ServerUrl => Self::Username,
            Self::Username => Self::Password,
            Self::Password => Self::ServerUrl,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::ServerUrl => Self::Password,
            Self::Username => Self::ServerUrl,
            Self::Password => Self::Username,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NavEntry {
    pub parent_id: String,
    pub title: String,
}

pub struct App {
    pub running: bool,
    pub screen: Screen,
    pub config: Config,
    pub client: Option<JellyfinClient>,

    pub login_field: LoginField,
    pub server_url_input: String,
    pub username_input: String,
    pub password_input: String,
    pub login_error: Option<String>,
    pub login_loading: bool,

    pub home_sections: Vec<HomeSection>,
    pub home_row: usize,
    pub home_col: usize,

    pub items: Vec<MediaItem>,
    pub selected_index: usize,
    pub grid_columns: usize,
    pub nav_stack: Vec<NavEntry>,
    pub loading: bool,
    pub error_message: Option<String>,
    pub total_items: u32,

    pub player: MpvPlayer,
    pub playback_session_id: u64,
    pub now_playing: Option<PlayingItem>,
    pub last_position_ticks: u64,
    pub playback_position_secs: f64,
    pub playback_duration_secs: f64,
    pub playback_paused: bool,
    pub last_progress_report: Option<Instant>,
    pub marked_as_played: bool,

    pub search_query: String,
    pub search_results: Vec<MediaItem>,
    pub search_selected: usize,
    pub previous_screen: Option<Screen>,

    pub downloads: Vec<DownloadTask>,
    pub show_downloads: bool,
    pub download_manager: Option<DownloadManager>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (screen, client) = if config.is_authenticated() {
            let client = JellyfinClient::with_token(
                config.server_url.clone().unwrap(),
                client_info(),
                config.access_token.clone().unwrap(),
                config.user_id.clone().unwrap(),
            );
            (Screen::Home, Some(client))
        } else {
            (Screen::Login, None)
        };

        let server_url_input = config.server_url.clone().unwrap_or_default();

        Self {
            running: true,
            screen,
            config,
            client,
            login_field: LoginField::ServerUrl,
            server_url_input,
            username_input: String::new(),
            password_input: String::new(),
            login_error: None,
            login_loading: false,
            home_sections: Vec::new(),
            home_row: 0,
            home_col: 0,
            items: Vec::new(),
            selected_index: 0,
            grid_columns: 1,
            nav_stack: Vec::new(),
            loading: false,
            error_message: None,
            total_items: 0,
            player: MpvPlayer::new(),
            playback_session_id: 0,
            now_playing: None,
            last_position_ticks: 0,
            playback_position_secs: 0.0,
            playback_duration_secs: 0.0,
            playback_paused: false,
            last_progress_report: None,
            marked_as_played: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_selected: 0,
            previous_screen: None,
            downloads: Vec::new(),
            show_downloads: false,
            download_manager: DownloadManager::new().ok(),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn current_input_mut(&mut self) -> &mut String {
        match self.login_field {
            LoginField::ServerUrl => &mut self.server_url_input,
            LoginField::Username => &mut self.username_input,
            LoginField::Password => &mut self.password_input,
        }
    }

    pub fn handle_unauthorized(&mut self, error: &JellyfinError) -> bool {
        if matches!(error, JellyfinError::Unauthorized) {
            self.reset_to_login("Session expired. Please log in again.");
            true
        } else {
            false
        }
    }

    fn reset_to_login(&mut self, message: &str) {
        self.screen = Screen::Login;
        self.client = None;
        self.login_loading = false;
        self.login_error = Some(message.to_string());
        self.password_input.clear();
        self.error_message = None;
        self.nav_stack.clear();
        self.home_sections.clear();
        self.home_row = 0;
        self.home_col = 0;
        self.items.clear();
        self.selected_index = 0;
        self.search_query.clear();
        self.search_results.clear();
        self.search_selected = 0;
        self.show_downloads = false;
        self.playback_session_id = 0;
        self.now_playing = None;
        self.last_position_ticks = 0;
        self.playback_position_secs = 0.0;
        self.playback_duration_secs = 0.0;
        self.playback_paused = false;
        self.last_progress_report = None;
        self.marked_as_played = false;
        self.player.stop();
        self.config.access_token = None;
        self.config.user_id = None;
        let _ = self.config.save();
    }

    pub async fn attempt_login(&mut self) -> anyhow::Result<()> {
        self.login_error = None;
        self.login_loading = true;

        let mut client = JellyfinClient::new(self.server_url_input.clone(), client_info());

        match client
            .authenticate(&self.username_input, &self.password_input)
            .await
        {
            Ok(_) => {
                self.config.server_url = Some(self.server_url_input.clone());
                self.config.access_token = client.access_token().map(str::to_string);
                self.config.user_id = client.user_id().map(str::to_string);
                self.config.save()?;

                self.client = Some(client);
                self.screen = Screen::Home;
                self.login_loading = false;
                self.load_home_content().await?;
                Ok(())
            }
            Err(e) => {
                self.login_error = Some(e.to_string());
                self.login_loading = false;
                Err(e.into())
            }
        }
    }

    pub async fn load_home_content(&mut self) -> anyhow::Result<()> {
        self.loading = true;
        self.error_message = None;
        self.home_sections.clear();
        self.home_row = 0;
        self.home_col = 0;

        if self.client.is_some() {
            // Libraries
            match {
                let client = self.client.as_ref().unwrap();
                client.get_user_views().await
            } {
                Ok(libs) => {
                    if !libs.is_empty() {
                        self.home_sections.push(HomeSection {
                            title: "Libraries".to_string(),
                            kind: HomeSectionKind::Libraries,
                            items: libs,
                        });
                    }
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    self.error_message = Some(e.to_string());
                }
            }

            // Continue Watching
            match {
                let client = self.client.as_ref().unwrap();
                client.get_resume_items(10).await
            } {
                Ok(resp) => {
                    if !resp.items.is_empty() {
                        self.home_sections.push(HomeSection {
                            title: "Continue Watching".to_string(),
                            kind: HomeSectionKind::Items,
                            items: resp.items,
                        });
                    }
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    if self.error_message.is_none() {
                        self.error_message = Some(e.to_string());
                    }
                }
            }

            // Next Up
            match {
                let client = self.client.as_ref().unwrap();
                client.get_next_up_items(10).await
            } {
                Ok(resp) => {
                    if !resp.items.is_empty() {
                        self.home_sections.push(HomeSection {
                            title: "Next Up".to_string(),
                            kind: HomeSectionKind::Items,
                            items: resp.items,
                        });
                    }
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    if self.error_message.is_none() {
                        self.error_message = Some(e.to_string());
                    }
                }
            }

            // Recently in Movies
            match {
                let client = self.client.as_ref().unwrap();
                client.get_latest_items(&["Movie"], 10).await
            } {
                Ok(resp) => {
                    if !resp.items.is_empty() {
                        self.home_sections.push(HomeSection {
                            title: "Recently in Movies".to_string(),
                            kind: HomeSectionKind::Items,
                            items: resp.items,
                        });
                    }
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    if self.error_message.is_none() {
                        self.error_message = Some(e.to_string());
                    }
                }
            }

            // Recently in TV Shows
            match {
                let client = self.client.as_ref().unwrap();
                client.get_latest_items(&["Series"], 10).await
            } {
                Ok(resp) => {
                    if !resp.items.is_empty() {
                        self.home_sections.push(HomeSection {
                            title: "Recently in TV Shows".to_string(),
                            kind: HomeSectionKind::Items,
                            items: resp.items,
                        });
                    }
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    if self.error_message.is_none() {
                        self.error_message = Some(e.to_string());
                    }
                }
            }

            self.loading = false;
        }
        Ok(())
    }

    pub async fn load_items(&mut self, parent_id: &str) -> anyhow::Result<()> {
        self.loading = true;
        self.error_message = None;

        if let Some(ref client) = self.client {
            match client.get_items(parent_id, 0, 100).await {
                Ok(response) => {
                    self.items = response.items;
                    self.total_items = response.total_record_count;
                    self.selected_index = 0;
                    self.loading = false;
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    self.error_message = Some(e.to_string());
                    self.loading = false;
                }
            }
        }
        Ok(())
    }

    pub fn current_home_item(&self) -> Option<&MediaItem> {
        self.home_sections
            .get(self.home_row)
            .and_then(|s| s.items.get(self.home_col))
    }

    pub fn move_up(&mut self) {
        match self.screen {
            Screen::Home => {
                if self.home_row > 0 {
                    self.home_row -= 1;
                    let len = self
                        .home_sections
                        .get(self.home_row)
                        .map(|s| s.items.len())
                        .unwrap_or(0);
                    if len > 0 && self.home_col >= len {
                        self.home_col = len - 1;
                    }
                }
            }
            Screen::Library => {
                let cols = self.grid_columns.max(1);
                if self.selected_index >= cols {
                    self.selected_index -= cols;
                }
            }
            Screen::Search => {
                let cols = self.grid_columns.max(1);
                if self.search_selected >= cols {
                    self.search_selected -= cols;
                }
            }
            Screen::Login => {}
        }
    }

    pub fn move_down(&mut self) {
        match self.screen {
            Screen::Home => {
                if self.home_row + 1 < self.home_sections.len() {
                    self.home_row += 1;
                    let len = self
                        .home_sections
                        .get(self.home_row)
                        .map(|s| s.items.len())
                        .unwrap_or(0);
                    if len > 0 && self.home_col >= len {
                        self.home_col = len - 1;
                    }
                }
            }
            Screen::Library => {
                let cols = self.grid_columns.max(1);
                let len = self.items.len();
                let new_index = self.selected_index + cols;
                if new_index < len {
                    self.selected_index = new_index;
                } else if len > 0 {
                    // snap to last item on the last row
                    self.selected_index = len - 1;
                }
            }
            Screen::Search => {
                let cols = self.grid_columns.max(1);
                let len = self.search_results.len();
                let new_index = self.search_selected + cols;
                if new_index < len {
                    self.search_selected = new_index;
                } else if len > 0 {
                    self.search_selected = len - 1;
                }
            }
            Screen::Login => {}
        }
    }

    pub fn move_left(&mut self) {
        match self.screen {
            Screen::Home => {
                if self.home_col > 0 {
                    self.home_col -= 1;
                }
            }
            Screen::Library => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
            }
            Screen::Search => {
                if self.search_selected > 0 {
                    self.search_selected -= 1;
                }
            }
            Screen::Login => {}
        }
    }

    pub fn move_right(&mut self) {
        match self.screen {
            Screen::Home => {
                let len = self
                    .home_sections
                    .get(self.home_row)
                    .map(|s| s.items.len())
                    .unwrap_or(0);
                if len > 0 && self.home_col + 1 < len {
                    self.home_col += 1;
                }
            }
            Screen::Library => {
                if self.selected_index + 1 < self.items.len() {
                    self.selected_index += 1;
                }
            }
            Screen::Search => {
                if self.search_selected + 1 < self.search_results.len() {
                    self.search_selected += 1;
                }
            }
            Screen::Login => {}
        }
    }

    pub async fn select_item(&mut self) -> anyhow::Result<Option<PlayingItem>> {
        match self.screen {
            Screen::Home => {
                let (kind, item) = match self.home_sections.get(self.home_row) {
                    Some(section) => match section.items.get(self.home_col) {
                        Some(item) => (section.kind, item.clone()),
                        None => return Ok(None),
                    },
                    None => return Ok(None),
                };

                match kind {
                    HomeSectionKind::Libraries => {
                        let lib_id = item.id.clone();
                        let lib_name = item.name.clone();
                        self.nav_stack.push(NavEntry {
                            parent_id: lib_id.clone(),
                            title: lib_name,
                        });
                        self.screen = Screen::Library;
                        self.load_items(&lib_id).await?;
                        Ok(None)
                    }
                    HomeSectionKind::Items => {
                        if item.is_folder {
                            let item_id = item.id.clone();
                            let item_name = item.name.clone();
                            self.nav_stack.push(NavEntry {
                                parent_id: item_id.clone(),
                                title: item_name,
                            });
                            self.screen = Screen::Library;
                            self.load_items(&item_id).await?;
                            Ok(None)
                        } else {
                            let full_item = if let Some(ref client) = self.client {
                                client.get_item(&item.id).await.unwrap_or(item)
                            } else {
                                item
                            };
                            let start_position_ticks = full_item
                                .user_data
                                .as_ref()
                                .map(|ud| ud.playback_position_ticks)
                                .unwrap_or(0);
                            Ok(Some(PlayingItem {
                                item: full_item,
                                start_position_ticks,
                            }))
                        }
                    }
                }
            }
            Screen::Library => {
                if let Some(item) = self.items.get(self.selected_index).cloned() {
                    if item.is_folder {
                        let item_id = item.id.clone();
                        let item_name = item.name.clone();
                        self.nav_stack.push(NavEntry {
                            parent_id: item_id.clone(),
                            title: item_name,
                        });
                        self.load_items(&item_id).await?;
                        Ok(None)
                    } else {
                        let full_item = if let Some(ref client) = self.client {
                            match client.get_item(&item.id).await {
                                Ok(full_item) => full_item,
                                Err(e) => {
                                    if self.handle_unauthorized(&e) {
                                        return Ok(None);
                                    }
                                    item
                                }
                            }
                        } else {
                            item
                        };

                        let start_position_ticks = full_item
                            .user_data
                            .as_ref()
                            .map(|ud| ud.playback_position_ticks)
                            .unwrap_or(0);

                        Ok(Some(PlayingItem {
                            item: full_item,
                            start_position_ticks,
                        }))
                    }
                } else {
                    Ok(None)
                }
            }
            Screen::Search => {
                if let Some(item) = self.search_results.get(self.search_selected).cloned() {
                    if item.is_folder {
                        let item_id = item.id.clone();
                        let item_name = item.name.clone();
                        self.nav_stack.clear();
                        self.nav_stack.push(NavEntry {
                            parent_id: item_id.clone(),
                            title: item_name,
                        });
                        self.screen = Screen::Library;
                        self.load_items(&item_id).await?;
                        Ok(None)
                    } else {
                        let full_item = if let Some(ref client) = self.client {
                            match client.get_item(&item.id).await {
                                Ok(full_item) => full_item,
                                Err(e) => {
                                    if self.handle_unauthorized(&e) {
                                        return Ok(None);
                                    }
                                    item
                                }
                            }
                        } else {
                            item
                        };

                        let start_position_ticks = full_item
                            .user_data
                            .as_ref()
                            .map(|ud| ud.playback_position_ticks)
                            .unwrap_or(0);

                        Ok(Some(PlayingItem {
                            item: full_item,
                            start_position_ticks,
                        }))
                    }
                } else {
                    Ok(None)
                }
            }
            Screen::Login => Ok(None),
        }
    }

    pub async fn perform_search(&mut self) -> anyhow::Result<()> {
        if self.search_query.is_empty() {
            self.search_results.clear();
            return Ok(());
        }

        self.loading = true;
        self.error_message = None;

        if let Some(ref client) = self.client {
            match client.search(&self.search_query, 50).await {
                Ok(response) => {
                    self.search_results = response.items;
                    self.search_selected = 0;
                    self.loading = false;
                }
                Err(e) => {
                    if self.handle_unauthorized(&e) {
                        self.loading = false;
                        return Ok(());
                    }
                    self.error_message = Some(e.to_string());
                    self.loading = false;
                }
            }
        }
        Ok(())
    }

    pub fn open_search(&mut self) {
        self.previous_screen = Some(self.screen);
        self.screen = Screen::Search;
        self.search_query.clear();
        self.search_results.clear();
        self.search_selected = 0;
    }

    pub fn close_search(&mut self) {
        if let Some(prev) = self.previous_screen.take() {
            self.screen = prev;
        } else {
            self.screen = Screen::Home;
        }
    }

    pub fn toggle_downloads(&mut self) {
        self.show_downloads = !self.show_downloads;
    }

    pub fn queue_download(&mut self) -> Option<DownloadTask> {
        let item = match self.screen {
            Screen::Library => self.items.get(self.selected_index).cloned(),
            Screen::Search => self.search_results.get(self.search_selected).cloned(),
            Screen::Home => self.current_home_item().cloned(),
            _ => None,
        }?;

        if item.is_folder {
            return None;
        }

        if self.downloads.iter().any(|d| d.item_id == item.id) {
            return None;
        }

        let client = self.client.as_ref()?;
        let url = client.get_download_url(&item.id).ok()?;
        let manager = self.download_manager.as_ref()?;

        let task = manager.create_task(item.id, item.name, url);
        self.downloads.push(task.clone());
        Some(task)
    }

    pub fn update_download_progress(&mut self, item_id: &str, progress: u8) {
        if let Some(task) = self.downloads.iter_mut().find(|d| d.item_id == item_id) {
            task.progress = progress;
            task.status = DownloadStatus::Downloading;
        }
    }

    pub fn mark_download_completed(&mut self, item_id: &str) {
        if let Some(task) = self.downloads.iter_mut().find(|d| d.item_id == item_id) {
            task.progress = 100;
            task.status = DownloadStatus::Completed;
        }
    }

    pub fn mark_download_failed(&mut self, item_id: &str, error: String) {
        if let Some(task) = self.downloads.iter_mut().find(|d| d.item_id == item_id) {
            task.status = DownloadStatus::Failed;
            task.error = Some(error);
        }
    }

    pub fn mark_download_started(&mut self, item_id: &str) {
        if let Some(task) = self.downloads.iter_mut().find(|d| d.item_id == item_id) {
            task.status = DownloadStatus::Downloading;
        }
    }

    pub async fn go_back(&mut self) -> anyhow::Result<()> {
        match self.screen {
            Screen::Library => {
                self.nav_stack.pop();
                if let Some(entry) = self.nav_stack.last() {
                    let parent_id = entry.parent_id.clone();
                    self.load_items(&parent_id).await?;
                } else {
                    self.screen = Screen::Home;
                    self.selected_index = 0;
                }
            }
            Screen::Home | Screen::Login | Screen::Search => {}
        }
        Ok(())
    }

    pub fn current_title(&self) -> String {
        match self.screen {
            Screen::Login => "Login".to_string(),
            Screen::Home => "Home".to_string(),
            Screen::Library => self
                .nav_stack
                .last()
                .map(|e| e.title.clone())
                .unwrap_or_else(|| "Library".to_string()),
            Screen::Search => "Search".to_string(),
        }
    }
}
