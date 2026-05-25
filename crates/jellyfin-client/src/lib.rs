//! Async Rust client for the [Jellyfin](https://jellyfin.org) media server API.
//!
//! # Quick start
//!
//! ```no_run
//! use jellyfin_client::{ClientInfo, JellyfinClient};
//!
//! # async fn run() -> Result<(), jellyfin_client::Error> {
//! let info = ClientInfo::new("my-app", "0.1.0").with_device_id("host-abc123");
//! let mut client = JellyfinClient::new("https://jellyfin.example.com", info);
//!
//! client.authenticate("alice", "hunter2").await?;
//!
//! for view in client.get_user_views().await? {
//!     println!("{}: {}", view.r#type, view.name);
//! }
//! # Ok(()) }
//! ```
//!
//! # Authentication
//!
//! Most endpoints require authentication. Either call
//! [`JellyfinClient::authenticate`] with a username and password, or restore a
//! prior session with [`JellyfinClient::with_token`]. Unauthenticated requests
//! to user-scoped endpoints return [`Error::Unauthenticated`]; expired tokens
//! return [`Error::Unauthorized`].

use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};

mod error;
pub use error::{Error, Result};

/// Identifies the client application to the Jellyfin server.
///
/// Jellyfin uses these fields to populate the active-sessions list and to
/// distinguish devices. Set [`device_id`](Self::device_id) to a stable,
/// per-installation identifier so the server can track sessions correctly.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub name: String,
    pub device: String,
    pub device_id: String,
    pub version: String,
}

impl ClientInfo {
    /// Create a `ClientInfo` with sensible defaults for `device` and `device_id`.
    ///
    /// Callers should override [`device_id`](Self::with_device_id) with a stable
    /// per-installation identifier.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            device_id: name.clone(),
            name,
            device: "PC".to_string(),
            version: version.into(),
        }
    }

    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = device.into();
        self
    }

    pub fn with_device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = device_id.into();
        self
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct AuthRequest<'a> {
    username: &'a str,
    pw: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthResponse {
    pub access_token: String,
    pub user: User,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub collection_type: Option<String>,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub production_year: Option<u32>,
    #[serde(default)]
    pub index_number: Option<u32>,
    #[serde(default)]
    pub parent_index_number: Option<u32>,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default)]
    pub run_time_ticks: Option<u64>,
    #[serde(default)]
    pub user_data: Option<UserData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserData {
    #[serde(default)]
    pub playback_position_ticks: u64,
    #[serde(default)]
    pub played: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsResponse {
    pub items: Vec<MediaItem>,
    pub total_record_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ViewsResponse {
    items: Vec<MediaItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfoResponse {
    #[serde(default)]
    pub media_sources: Vec<MediaSource>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
    pub id: String,
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub is_external: bool,
    #[serde(default)]
    pub is_text_subtitle_stream: bool,
    pub index: i32,
}

#[derive(Debug, Clone)]
pub struct ExternalSubtitle {
    pub url: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackStartInfo {
    pub item_id: String,
    pub position_ticks: u64,
    pub is_paused: bool,
    pub is_muted: bool,
    pub volume_level: u32,
    pub play_method: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackProgressInfo {
    pub item_id: String,
    pub position_ticks: u64,
    pub is_paused: bool,
    pub is_muted: bool,
    pub volume_level: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackStopInfo {
    pub item_id: String,
    pub position_ticks: u64,
}

/// Async client for a single Jellyfin server.
///
/// The client is cheap to clone — internally it holds a `reqwest::Client` which
/// shares its connection pool across clones. After authenticating, clone freely
/// to share the session across tasks.
#[derive(Clone)]
pub struct JellyfinClient {
    http: Client,
    info: ClientInfo,
    server_url: String,
    access_token: Option<String>,
    user_id: Option<String>,
}

impl JellyfinClient {
    fn build_http() -> Client {
        Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client builder should not fail with default settings")
    }

    /// Create a new unauthenticated client.
    ///
    /// Call [`authenticate`](Self::authenticate) before using user-scoped endpoints.
    pub fn new(server_url: impl Into<String>, info: ClientInfo) -> Self {
        Self {
            http: Self::build_http(),
            info,
            server_url: server_url.into().trim_end_matches('/').to_string(),
            access_token: None,
            user_id: None,
        }
    }

    /// Create a client from a previously-obtained access token.
    ///
    /// Use this to restore a session across application restarts.
    pub fn with_token(
        server_url: impl Into<String>,
        info: ClientInfo,
        access_token: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            http: Self::build_http(),
            info,
            server_url: server_url.into().trim_end_matches('/').to_string(),
            access_token: Some(access_token.into()),
            user_id: Some(user_id.into()),
        }
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    fn user_id_required(&self) -> Result<&str> {
        self.user_id.as_deref().ok_or(Error::Unauthenticated)
    }

    fn token_required(&self) -> Result<&str> {
        self.access_token.as_deref().ok_or(Error::Unauthenticated)
    }

    fn auth_header(&self) -> String {
        let mut header = format!(
            "MediaBrowser Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
            self.info.name, self.info.device, self.info.device_id, self.info.version
        );
        if let Some(token) = &self.access_token {
            header.push_str(&format!(", Token=\"{}\"", token));
        }
        header
    }

    /// Authenticate by username and password and store the resulting token.
    pub async fn authenticate(&mut self, username: &str, password: &str) -> Result<AuthResponse> {
        let url = format!("{}/Users/AuthenticateByName", self.server_url);

        let response = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(&AuthRequest {
                username,
                pw: password,
            })
            .send()
            .await?;

        let response = check_status(response, "authenticate").await?;
        let auth_response: AuthResponse = response.json().await?;
        self.access_token = Some(auth_response.access_token.clone());
        self.user_id = Some(auth_response.user.id.clone());
        Ok(auth_response)
    }

    /// List the user's libraries (Views).
    pub async fn get_user_views(&self) -> Result<Vec<MediaItem>> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Views", self.server_url, user_id);

        let response = self
            .http
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_user_views").await?;
        let views: ViewsResponse = response.json().await?;
        Ok(views.items)
    }

    /// Paginated children of a library or folder, sorted by name.
    pub async fn get_items(
        &self,
        parent_id: &str,
        start_index: u32,
        limit: u32,
    ) -> Result<ItemsResponse> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Items", self.server_url, user_id);

        let response = self
            .http
            .get(&url)
            .query(&[
                ("ParentId", parent_id),
                ("StartIndex", &start_index.to_string()),
                ("Limit", &limit.to_string()),
                ("SortBy", "SortName"),
                ("SortOrder", "Ascending"),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_items").await?;
        Ok(response.json().await?)
    }

    /// Items the user can resume (i.e. have partial playback progress).
    pub async fn get_resume_items(&self, limit: u32) -> Result<ItemsResponse> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Items/Resume", self.server_url, user_id);

        let response = self
            .http
            .get(&url)
            .query(&[
                ("Limit", &limit.to_string()[..]),
                ("Recursive", "true"),
                (
                    "Fields",
                    "PrimaryImageAspectRatio,BasicSyncInfo,ProductionYear,Status,EndDate",
                ),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Banner,Thumb"),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_resume_items").await?;
        Ok(response.json().await?)
    }

    /// "Next Up" episodes across the user's series.
    pub async fn get_next_up_items(&self, limit: u32) -> Result<ItemsResponse> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Shows/NextUp", self.server_url);

        let response = self
            .http
            .get(&url)
            .query(&[
                ("UserId", user_id),
                ("Limit", &limit.to_string()),
                (
                    "Fields",
                    "PrimaryImageAspectRatio,SeriesInfo,DateCreated,BasicSyncInfo,MediaSourceCount",
                ),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Banner,Thumb"),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_next_up_items").await?;
        Ok(response.json().await?)
    }

    /// Most recently added items of the given types (e.g. `["Movie"]`, `["Series"]`).
    pub async fn get_latest_items(&self, item_types: &[&str], limit: u32) -> Result<ItemsResponse> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Items", self.server_url, user_id);
        let types = item_types.join(",");

        let response = self
            .http
            .get(&url)
            .query(&[
                ("IncludeItemTypes", &types[..]),
                ("Recursive", "true"),
                ("SortBy", "DateCreated"),
                ("SortOrder", "Descending"),
                ("Limit", &limit.to_string()),
                (
                    "Fields",
                    "PrimaryImageAspectRatio,ProductionYear,Status,EndDate",
                ),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Banner,Thumb"),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_latest_items").await?;
        Ok(response.json().await?)
    }

    /// Fetch a single item by ID with all user-scoped fields populated.
    pub async fn get_item(&self, item_id: &str) -> Result<MediaItem> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Items/{}", self.server_url, user_id, item_id);

        let response = self
            .http
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "get_item").await?;
        Ok(response.json().await?)
    }

    /// External text subtitles (those served as separate files, not muxed
    /// inside the container) associated with an item.
    ///
    /// The returned URLs include the API key — treat them as sensitive.
    pub async fn get_external_subtitles(&self, item_id: &str) -> Result<Vec<ExternalSubtitle>> {
        let user_id = self.user_id_required()?;
        let token = self.token_required()?;

        let url = format!("{}/Items/{}/PlaybackInfo", self.server_url, item_id);

        let response = self
            .http
            .post(&url)
            .query(&[("UserId", user_id)])
            .header("X-Emby-Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await?;
        let response = check_status(response, "get_external_subtitles").await?;
        let info: PlaybackInfoResponse = response.json().await?;

        let mut subs = Vec::new();
        for source in &info.media_sources {
            for stream in &source.media_streams {
                if stream.r#type != "Subtitle"
                    || !stream.is_external
                    || !stream.is_text_subtitle_stream
                {
                    continue;
                }
                let codec = match stream.codec.as_deref() {
                    Some(c) if !c.is_empty() => c,
                    _ => continue,
                };
                let sub_url = format!(
                    "{}/Videos/{}/{}/Subtitles/{}/0/Stream.{}?api_key={}",
                    self.server_url, item_id, source.id, stream.index, codec, token
                );
                subs.push(ExternalSubtitle {
                    url: sub_url,
                    language: stream.language.clone(),
                    title: stream.display_title.clone(),
                });
            }
        }
        Ok(subs)
    }

    /// URL for an item's primary image (poster). Returns `None` if the item
    /// has no primary image tag, which lets callers skip fetching for items
    /// that would 404. `max_height` caps the server-side resize.
    pub fn get_primary_image_url(&self, item_id: &str, max_height: u32) -> String {
        let mut url = format!(
            "{}/Items/{}/Images/Primary?maxHeight={}&quality=90",
            self.server_url, item_id, max_height
        );
        if let Some(token) = &self.access_token {
            url.push_str(&format!("&api_key={}", token));
        }
        url
    }

    /// Direct-stream URL for an item. Suitable for handing to an external
    /// player like mpv. The returned URL includes the API key.
    pub fn get_stream_url(&self, item_id: &str) -> Result<String> {
        let token = self.token_required()?;
        Ok(format!(
            "{}/Videos/{}/stream?Static=true&api_key={}",
            self.server_url, item_id, token
        ))
    }

    /// Direct download URL for an item.
    pub fn get_download_url(&self, item_id: &str) -> Result<String> {
        let token = self.token_required()?;
        Ok(format!(
            "{}/Items/{}/Download?api_key={}",
            self.server_url, item_id, token
        ))
    }

    /// Report that playback has started. The server uses this to keep the
    /// sessions list in sync and to power "currently playing" indicators.
    pub async fn report_playback_start(&self, info: &PlaybackStartInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing", self.server_url);
        let response = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;
        check_status(response, "report_playback_start").await?;
        Ok(())
    }

    /// Report playback progress. Call periodically (e.g. every 5 seconds)
    /// while playing so the server can update resume position.
    pub async fn report_playback_progress(&self, info: &PlaybackProgressInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing/Progress", self.server_url);
        let response = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;
        check_status(response, "report_playback_progress").await?;
        Ok(())
    }

    /// Report that playback has stopped. Pair this with [`report_playback_start`](Self::report_playback_start).
    pub async fn report_playback_stop(&self, info: &PlaybackStopInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing/Stopped", self.server_url);
        let response = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;
        check_status(response, "report_playback_stop").await?;
        Ok(())
    }

    /// Mark an item as played for the current user.
    pub async fn mark_played(&self, item_id: &str) -> Result<()> {
        let user_id = self.user_id_required()?;
        let url = format!(
            "{}/Users/{}/PlayedItems/{}",
            self.server_url, user_id, item_id
        );
        let response = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        check_status(response, "mark_played").await?;
        Ok(())
    }

    /// Free-text search across the user's library.
    pub async fn search(&self, query: &str, limit: u32) -> Result<ItemsResponse> {
        let user_id = self.user_id_required()?;
        let url = format!("{}/Users/{}/Items", self.server_url, user_id);

        let response = self
            .http
            .get(&url)
            .query(&[
                ("searchTerm", query),
                ("Recursive", "true"),
                ("Limit", &limit.to_string()),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;
        let response = check_status(response, "search").await?;
        Ok(response.json().await?)
    }
}

async fn check_status(response: Response, op: &'static str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED {
        return Err(Error::Unauthorized);
    }
    Err(Error::Server {
        operation: op,
        status: status.as_u16(),
        body,
    })
}
