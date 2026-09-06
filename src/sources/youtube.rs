use crate::{
    commands::play::{Mode, QueryType},
    utils::{get_reqwest_client, yt_dlp_cookie_args},
};
use async_trait::async_trait;
use serde_json::Value;
use songbird::input::{AudioStream, AudioStreamError, AuxMetadata, Compose, Input, YoutubeDl};
use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
use symphonia::core::io::MediaSource;
use tokio::task::spawn_blocking;
use tracing::{error, instrument};
use url::Url;

pub struct YouTube {}

#[derive(Clone)]
pub struct PlaylistEntry {
    pub url: String,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

impl From<PlaylistEntry> for QueryType {
    fn from(entry: PlaylistEntry) -> Self {
        QueryType::VideoMeta {
            url: entry.url,
            title: entry.title,
            duration: entry.duration,
        }
    }
}

impl YouTube {
    pub fn extract(query: &str) -> Option<QueryType> {
        let url_data = Url::parse(query).ok()?;

        let is_playlist = url_data.query_pairs().any(|(k, _)| k == "list")
            && !url_data.query_pairs().any(|(k, _)| k == "v");

        if is_playlist {
            Some(QueryType::PlaylistLink(query.to_string()))
        } else {
            Some(QueryType::VideoLink(query.to_string()))
        }
    }

    #[instrument(level = "info", skip_all)]
    pub async fn ytdl_playlist(uri: &str, mode: Mode) -> Option<Vec<PlaylistEntry>> {
        let mut args = vec![
            uri.to_string(),
            "--flat-playlist".to_string(),
            "-j".to_string(),
        ];
        match mode {
            Mode::Reverse => args.push("--playlist-reverse".to_string()),
            Mode::Shuffle => args.push("--playlist-random".to_string()),
            _ => {}
        }
        args.extend(yt_dlp_cookie_args());

        match spawn_blocking(move || run_ytdl_playlist(&args)).await {
            Ok(entries) => entries,
            Err(e) => {
                error!("yt-dlp playlist task failed: {e}");
                None
            }
        }
    }
}

fn run_ytdl_playlist(args: &[String]) -> Option<Vec<PlaylistEntry>> {
    let mut child = Command::new("yt-dlp")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| error!("Failed to spawn yt-dlp: {e}"))
        .ok()?;

    let Some(stdout) = child.stdout.take() else {
        error!("yt-dlp produced no stdout");
        return None;
    };

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stderr.read_to_end(&mut buffer);
            buffer
        })
    });

    let mut entries = Vec::new();

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Ok(entry) = serde_json::from_str::<Value>(&line)
            && entry.get("_type").and_then(Value::as_str) != Some("playlist")
            && let Some(url) = entry.get("webpage_url").and_then(Value::as_str)
        {
            entries.push(PlaylistEntry {
                url: url.to_string(),
                title: entry.get("title").and_then(Value::as_str).map(String::from),
                duration: entry.get("duration").and_then(Value::as_f64),
            });
        }
    }

    let status = child
        .wait()
        .map_err(|e| error!("Failed to wait for yt-dlp: {e}"))
        .ok()?;

    if !status.success() {
        let stderr = stderr_handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        error!(
            "yt-dlp exited with {}: {}",
            status,
            String::from_utf8_lossy(&stderr)
        );
        return None;
    }

    if entries.is_empty() {
        error!("yt-dlp produced no playlist entries");
        return None;
    }

    Some(entries)
}

#[derive(Clone)]
pub struct YouTubeTrack {
    url: String,
    meta: AuxMetadata,
}

impl YouTubeTrack {
    pub fn new(url: String, title: Option<String>, duration: Option<f64>) -> Self {
        let meta = AuxMetadata {
            title,
            source_url: Some(url.clone()),
            duration: aux_duration(duration),
            start_time: Some(Duration::from_secs(0)),
            ..AuxMetadata::default()
        };

        Self { url, meta }
    }

    pub fn metadata(&self) -> AuxMetadata {
        self.meta.clone()
    }
}

fn aux_duration(duration: Option<f64>) -> Option<Duration> {
    duration
        .filter(|duration| duration.is_finite() && *duration >= 0.0 && *duration <= 1.0e9)
        .map(Duration::from_secs_f64)
}

#[async_trait]
impl Compose for YouTubeTrack {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        Err(AudioStreamError::Unsupported)
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        YoutubeDl::new(get_reqwest_client().clone(), self.url.clone())
            .user_args(yt_dlp_cookie_args())
            .create_async()
            .await
    }

    fn should_create_async(&self) -> bool {
        true
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        Ok(self.meta.clone())
    }
}

impl From<YouTubeTrack> for Input {
    fn from(val: YouTubeTrack) -> Self {
        Input::Lazy(Box::new(val))
    }
}
