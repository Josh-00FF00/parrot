use crate::commands::play::{Mode, QueryType};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::Command,
    process::Stdio,
};

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
        let mut args = vec![uri, "--flat-playlist", "-j"];
        match mode {
            Mode::Reverse => args.push("--playlist-reverse"),
            Mode::Shuffle => args.push("--playlist-random"),
            _ => {}
        }

        let child = Command::new("yt-dlp")
            .args(args)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout?;
        let reader = BufReader::new(stdout);

        let lines = reader.lines().map_while(Result::ok).map(|line| {
            let entry: Value = serde_json::from_str(&line).unwrap();
            entry
                .get("webpage_url")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        });

        Some(lines.collect())
    }
}
