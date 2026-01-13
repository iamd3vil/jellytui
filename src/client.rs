use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct AuthRequest {
    username: String,
    pw: String,
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
    #[allow(dead_code)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub r#type: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub collection_type: Option<String>,
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
pub struct ViewsResponse {
    pub items: Vec<MediaItem>,
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

pub struct JellyfinClient {
    client: Client,
    pub server_url: String,
    pub access_token: Option<String>,
    pub user_id: Option<String>,
}

impl JellyfinClient {
    pub fn new(server_url: String) -> Self {
        Self {
            client: Client::new(),
            server_url,
            access_token: None,
            user_id: None,
        }
    }

    pub fn with_token(server_url: String, access_token: String, user_id: String) -> Self {
        Self {
            client: Client::new(),
            server_url,
            access_token: Some(access_token),
            user_id: Some(user_id),
        }
    }

    fn auth_header(&self) -> String {
        let token_part = self
            .access_token
            .as_ref()
            .map(|t| format!(", Token=\"{}\"", t))
            .unwrap_or_default();

        format!(
            "MediaBrowser Client=\"jellytui\", Device=\"PC\", DeviceId=\"jellytui-rust\", Version=\"0.1.0\"{}",
            token_part
        )
    }

    pub async fn authenticate(&mut self, username: &str, password: &str) -> Result<AuthResponse> {
        let url = format!("{}/Users/AuthenticateByName", self.server_url);

        let response = self
            .client
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(&AuthRequest {
                username: username.to_string(),
                pw: password.to_string(),
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Authentication failed: {} - {}", status, body);
        }

        let auth_response: AuthResponse = response.json().await?;
        self.access_token = Some(auth_response.access_token.clone());
        self.user_id = Some(auth_response.user.id.clone());

        Ok(auth_response)
    }

    pub async fn get_user_views(&self) -> Result<Vec<MediaItem>> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!("{}/Users/{}/Views", self.server_url, user_id);

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch views: {} - {}", status, body);
        }

        let views: ViewsResponse = response.json().await?;
        Ok(views.items)
    }

    pub async fn get_items(
        &self,
        parent_id: &str,
        start_index: u32,
        limit: u32,
    ) -> Result<ItemsResponse> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!(
            "{}/Users/{}/Items?ParentId={}&StartIndex={}&Limit={}&SortBy=SortName&SortOrder=Ascending",
            self.server_url, user_id, parent_id, start_index, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch items: {} - {}", status, body);
        }

        let items: ItemsResponse = response.json().await?;
        Ok(items)
    }

    pub async fn get_resume_items(&self, limit: u32) -> Result<ItemsResponse> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!(
            "{}/Users/{}/Items/Resume?Limit={}&Recursive=true&Fields=PrimaryImageAspectRatio,BasicSyncInfo,ProductionYear,Status,EndDate&ImageTypeLimit=1&EnableImageTypes=Primary,Backdrop,Banner,Thumb",
            self.server_url, user_id, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch resume items: {} - {}", status, body);
        }

        let items: ItemsResponse = response.json().await?;
        Ok(items)
    }

    pub async fn get_next_up_items(&self, limit: u32) -> Result<ItemsResponse> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!(
            "{}/Shows/NextUp?UserId={}&Limit={}&Fields=PrimaryImageAspectRatio,SeriesInfo,DateCreated,BasicSyncInfo,MediaSourceCount&ImageTypeLimit=1&EnableImageTypes=Primary,Backdrop,Banner,Thumb",
            self.server_url, user_id, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch next up items: {} - {}", status, body);
        }

        let items: ItemsResponse = response.json().await?;
        Ok(items)
    }

    pub async fn get_latest_items(&self, item_types: &[&str], limit: u32) -> Result<ItemsResponse> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let types = item_types.join(",");
        let url = format!(
            "{}/Users/{}/Items?IncludeItemTypes={}&Recursive=true&SortBy=DateCreated&SortOrder=Descending&Limit={}&Fields=PrimaryImageAspectRatio,ProductionYear,Status,EndDate&ImageTypeLimit=1&EnableImageTypes=Primary,Backdrop,Banner,Thumb",
            self.server_url, user_id, types, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch latest items: {} - {}", status, body);
        }

        let items: ItemsResponse = response.json().await?;
        Ok(items)
    }

    pub async fn get_item(&self, item_id: &str) -> Result<MediaItem> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!("{}/Users/{}/Items/{}", self.server_url, user_id, item_id);

        let response = self
            .client
            .get(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch item: {} - {}", status, body);
        }

        let item: MediaItem = response.json().await?;
        Ok(item)
    }

    pub fn get_stream_url(&self, item_id: &str) -> Result<String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        Ok(format!(
            "{}/Videos/{}/stream?Static=true&api_key={}",
            self.server_url, item_id, token
        ))
    }

    pub async fn report_playback_start(&self, info: &PlaybackStartInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing", self.server_url);

        let response = self
            .client
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to report playback start: {} - {}", status, body);
        }

        Ok(())
    }

    pub async fn report_playback_progress(&self, info: &PlaybackProgressInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing/Progress", self.server_url);

        let response = self
            .client
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to report playback progress: {} - {}", status, body);
        }

        Ok(())
    }

    pub async fn report_playback_stop(&self, info: &PlaybackStopInfo) -> Result<()> {
        let url = format!("{}/Sessions/Playing/Stopped", self.server_url);

        let response = self
            .client
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .json(info)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to report playback stop: {} - {}", status, body);
        }

        Ok(())
    }

    pub async fn mark_played(&self, item_id: &str) -> Result<()> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let url = format!(
            "{}/Users/{}/PlayedItems/{}",
            self.server_url, user_id, item_id
        );

        let response = self
            .client
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to mark as played: {} - {}", status, body);
        }

        Ok(())
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<ItemsResponse> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        let base_url = format!("{}/Users/{}/Items", self.server_url, user_id);

        let response = self
            .client
            .get(&base_url)
            .query(&[
                ("searchTerm", query),
                ("Recursive", "true"),
                ("Limit", &limit.to_string()),
            ])
            .header("X-Emby-Authorization", self.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to search: {} - {}", status, body);
        }

        let items: ItemsResponse = response.json().await?;
        Ok(items)
    }

    pub fn get_download_url(&self, item_id: &str) -> Result<String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;

        Ok(format!(
            "{}/Items/{}/Download?api_key={}",
            self.server_url, item_id, token
        ))
    }
}
