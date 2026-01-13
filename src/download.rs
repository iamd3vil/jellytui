use std::path::PathBuf;

use anyhow::Result;
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub item_id: String,
    pub item_name: String,
    pub url: String,
    pub file_path: PathBuf,
    pub progress: u8,
    pub status: DownloadStatus,
    pub error: Option<String>,
}

impl DownloadTask {
    pub fn new(item_id: String, item_name: String, url: String, file_path: PathBuf) -> Self {
        Self {
            item_id,
            item_name,
            url,
            file_path,
            progress: 0,
            status: DownloadStatus::Pending,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Started { item_id: String },
    Progress { item_id: String, progress: u8 },
    Completed { item_id: String },
    Failed { item_id: String, error: String },
}

pub struct DownloadManager {
    pub download_dir: PathBuf,
}

impl DownloadManager {
    pub fn new() -> Result<Self> {
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("jellytui");

        std::fs::create_dir_all(&download_dir)?;

        Ok(Self { download_dir })
    }

    pub fn create_task(&self, item_id: String, item_name: String, url: String) -> DownloadTask {
        let safe_name = item_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();

        let file_path = self.download_dir.join(format!("{}.mkv", safe_name));

        DownloadTask::new(item_id, item_name, url, file_path)
    }
}

pub async fn perform_download(task: DownloadTask, tx: mpsc::UnboundedSender<DownloadEvent>) {
    let _ = tx.send(DownloadEvent::Started {
        item_id: task.item_id.clone(),
    });

    let client = Client::new();

    let response = match client.get(&task.url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed {
                item_id: task.item_id,
                error: e.to_string(),
            });
            return;
        }
    };

    if !response.status().is_success() {
        let _ = tx.send(DownloadEvent::Failed {
            item_id: task.item_id,
            error: format!("HTTP error: {}", response.status()),
        });
        return;
    }

    let total_size = response.content_length().unwrap_or(0);

    let mut file = match File::create(&task.file_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed {
                item_id: task.item_id,
                error: e.to_string(),
            });
            return;
        }
    };

    let mut downloaded: u64 = 0;
    let mut last_progress: u8 = 0;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(DownloadEvent::Failed {
                    item_id: task.item_id,
                    error: e.to_string(),
                });
                return;
            }
        };

        if file.write_all(&chunk).await.is_err() {
            let _ = tx.send(DownloadEvent::Failed {
                item_id: task.item_id,
                error: "Failed to write to file".to_string(),
            });
            return;
        }

        downloaded += chunk.len() as u64;

        if total_size > 0 {
            let progress = ((downloaded as f64 / total_size as f64) * 100.0) as u8;
            if progress != last_progress {
                last_progress = progress;
                let _ = tx.send(DownloadEvent::Progress {
                    item_id: task.item_id.clone(),
                    progress,
                });
            }
        }
    }

    let _ = file.flush().await;

    let _ = tx.send(DownloadEvent::Completed {
        item_id: task.item_id,
    });
}
