use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use mpv_ipc::MpvIpc;
use tokio::sync::mpsc;
use tokio::time::interval;

pub struct PlaybackState {
    pub position_secs: f64,
    #[allow(dead_code)]
    pub duration_secs: f64,
    pub paused: bool,
}

pub enum PlayerEvent {
    Progress(PlaybackState),
    Finished,
    #[allow(dead_code)]
    Error(String),
}

pub struct MpvPlayer {
    process: Option<Child>,
    socket_path: PathBuf,
}

impl MpvPlayer {
    pub fn new() -> Self {
        let socket_path =
            std::env::temp_dir().join(format!("jellytui-mpv-{}.sock", std::process::id()));
        Self {
            process: None,
            socket_path,
        }
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn start(&mut self, url: &str, start_position_secs: Option<f64>) -> Result<()> {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        let mut cmd = Command::new("mpv");
        cmd.arg(format!("--input-ipc-server={}", self.socket_path.display()))
            .arg("--force-window=yes");

        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            cmd.arg("--gpu-context=waylandvk");
        }

        cmd.arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(pos) = start_position_secs
            && pos > 0.0
        {
            cmd.arg(format!("--start={}", pos));
        }

        let mut child = cmd.spawn()?;

        std::thread::sleep(Duration::from_millis(500));

        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child.stderr.take();
                let stderr_output = stderr
                    .map(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();

                anyhow::bail!(
                    "mpv exited immediately with status {}: {}",
                    status,
                    stderr_output.trim()
                );
            }
            Ok(None) => {
                self.process = Some(child);
            }
            Err(e) => {
                anyhow::bail!("Failed to check mpv status: {}", e);
            }
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(ref mut process) = self.process {
            let _ = process.kill();
        }
        self.process = None;

        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut process) = self.process {
            match process.try_wait() {
                Ok(Some(_)) => {
                    self.process = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub async fn monitor_playback(socket_path: PathBuf, tx: mpsc::UnboundedSender<PlayerEvent>) {
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let mut mpv = match MpvIpc::connect(&socket_path).await {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(PlayerEvent::Error(format!(
                "Failed to connect to MPV: {}",
                e
            )));
            return;
        }
    };

    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        let position: f64 = match mpv.get_prop("time-pos").await {
            Ok(p) => p,
            Err(_) => {
                let _ = tx.send(PlayerEvent::Finished);
                break;
            }
        };

        let duration: f64 = mpv.get_prop("duration").await.unwrap_or(0.0);
        let paused: bool = mpv.get_prop("pause").await.unwrap_or(false);

        let state = PlaybackState {
            position_secs: position,
            duration_secs: duration,
            paused,
        };

        if tx.send(PlayerEvent::Progress(state)).is_err() {
            break;
        }
    }
}
