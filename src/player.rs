use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use directories::ProjectDirs;
use mpv_ipc::MpvIpc;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use uuid::Uuid;

const MPV_STARTUP_DELAY: Duration = Duration::from_millis(500);
const MPV_CONNECT_DELAY: Duration = Duration::from_millis(1000);
const MPV_SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const MPV_SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MPV_IPC_TIMEOUT: Duration = Duration::from_secs(2);

pub struct PlaybackState {
    pub position_secs: f64,
    #[allow(dead_code)]
    pub duration_secs: f64,
    pub paused: bool,
}

pub enum PlayerEvent {
    Progress {
        session_id: u64,
        state: PlaybackState,
    },
    Finished {
        session_id: u64,
    },
    #[allow(dead_code)]
    Error {
        session_id: u64,
        message: String,
    },
}

pub struct MpvPlayer {
    process: Option<Child>,
    socket_path: PathBuf,
    log_path: PathBuf,
}

impl MpvPlayer {
    pub fn new() -> Self {
        let socket_path = Self::new_socket_path();
        let log_path = Self::default_log_path();
        Self {
            process: None,
            socket_path,
            log_path,
        }
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }

    pub fn start(&mut self, url: &str, start_position_secs: Option<f64>) -> Result<()> {
        self.stop();
        self.socket_path = Self::new_socket_path();

        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.log_path)?;

        use std::io::Write as _;
        let _ = writeln!(log_file, "jellytui mpv log");
        let _ = writeln!(log_file, "url={url}");
        let _ = writeln!(
            log_file,
            "start_position_secs={}",
            start_position_secs.unwrap_or(0.0)
        );
        let _ = writeln!(log_file, "socket={}", self.socket_path.display());
        let _ = writeln!(log_file, "---");

        let stdout_log = log_file.try_clone()?;
        let stderr_log = log_file.try_clone()?;

        let mut cmd = Command::new("mpv");
        cmd.arg(format!("--input-ipc-server={}", self.socket_path.display()))
            .arg("--force-window=yes")
            .arg("--terminal=no");

        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            cmd.arg("--gpu-context=waylandvk");
        }

        cmd.arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_log))
            .stderr(Stdio::from(stderr_log));

        if let Some(pos) = start_position_secs
            && pos > 0.0
        {
            cmd.arg(format!("--start={}", pos));
        }

        let mut child = cmd.spawn()?;

        std::thread::sleep(MPV_STARTUP_DELAY);

        match child.try_wait() {
            Ok(Some(status)) => {
                anyhow::bail!("mpv exited immediately with status {}", status);
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

    fn default_log_path() -> PathBuf {
        if let Some(proj_dirs) = ProjectDirs::from("", "", "jellytui") {
            return proj_dirs.config_dir().join("logs").join("mpv.log");
        }

        std::env::temp_dir().join("jellytui-mpv.log")
    }

    fn new_socket_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "jellytui-mpv-{}-{}.sock",
            std::process::id(),
            Uuid::new_v4()
        ))
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub async fn monitor_playback(
    socket_path: PathBuf,
    log_path: PathBuf,
    session_id: u64,
    tx: mpsc::UnboundedSender<PlayerEvent>,
) {
    tokio::time::sleep(MPV_CONNECT_DELAY).await;

    if let Err(e) = wait_for_socket(&socket_path).await {
        append_monitor_log(&log_path, &e);
        let _ = tx.send(PlayerEvent::Error {
            session_id,
            message: e,
        });
        return;
    }

    let mut mpv = match timeout(MPV_IPC_TIMEOUT, MpvIpc::connect(&socket_path)).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            let message = format!("Failed to connect to MPV: {}", e);
            append_monitor_log(&log_path, &message);
            let _ = tx.send(PlayerEvent::Error {
                session_id,
                message,
            });
            return;
        }
        Err(_) => {
            let message = "Timed out while connecting to MPV".to_string();
            append_monitor_log(&log_path, &message);
            let _ = tx.send(PlayerEvent::Error {
                session_id,
                message,
            });
            return;
        }
    };

    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        let position: f64 = match timeout(MPV_IPC_TIMEOUT, mpv.get_prop("time-pos")).await {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => {
                let _ = tx.send(PlayerEvent::Finished { session_id });
                break;
            }
            Err(_) => {
                let message = "Timed out while reading playback state from MPV".to_string();
                append_monitor_log(&log_path, &message);
                let _ = tx.send(PlayerEvent::Error {
                    session_id,
                    message,
                });
                break;
            }
        };

        let duration: f64 = match timeout(MPV_IPC_TIMEOUT, mpv.get_prop("duration")).await {
            Ok(Ok(d)) => d,
            _ => 0.0,
        };
        let paused: bool = match timeout(MPV_IPC_TIMEOUT, mpv.get_prop("pause")).await {
            Ok(Ok(p)) => p,
            _ => false,
        };

        let state = PlaybackState {
            position_secs: position,
            duration_secs: duration,
            paused,
        };

        if tx
            .send(PlayerEvent::Progress { session_id, state })
            .is_err()
        {
            break;
        }
    }
}

async fn wait_for_socket(socket_path: &PathBuf) -> Result<(), String> {
    let deadline = Instant::now() + MPV_SOCKET_WAIT_TIMEOUT;

    while Instant::now() < deadline {
        if socket_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(MPV_SOCKET_POLL_INTERVAL).await;
    }

    Err(format!(
        "Timed out waiting for MPV IPC socket at {}",
        socket_path.display()
    ))
}

fn append_monitor_log(log_path: &PathBuf, message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(log_path) {
        use std::io::Write as _;
        let _ = writeln!(file, "jellytui monitor: {}", message);
    }
}
