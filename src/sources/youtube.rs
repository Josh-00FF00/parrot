use crate::commands::play::{Mode, QueryType};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};
use tracing::error;

pub struct YouTube {}

impl YouTube {
    pub fn extract(query: &str) -> Option<QueryType> {
        if query.contains("list=") {
            Some(QueryType::PlaylistLink(query.to_string()))
        } else {
            Some(QueryType::VideoLink(query.to_string()))
        }
    }

    pub async fn ytdl_playlist(uri: &str, mode: Mode) -> Option<Vec<String>> {
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

        let mut child = Command::new("yt-dlp")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| error!("Failed to spawn yt-dlp: {e}"))
            .ok()?;

        let Some(stdout) = child.stdout.take() else {
            error!("yt-dlp produced no stdout");
            return None;
        };

        let reader = BufReader::new(stdout);

        let mut urls = Vec::new();

        for line in reader.lines().map_while(Result::ok) {
            if let Ok(entry) = serde_json::from_str::<Value>(&line)
                && let Some(url) = entry.get("webpage_url").and_then(Value::as_str)
            {
                urls.push(url.to_string());
            }
        }

        if let Err(e) = child.wait() {
            error!("Failed to wait for yt-dlp: {e}");
        }

        Some(urls)
    }
}
