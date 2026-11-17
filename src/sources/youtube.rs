use crate::commands::play::{Mode, QueryType};
use async_trait::async_trait;
use serde_json::Value;
use songbird::input::{
    AudioStream, AudioStreamError, AuxMetadata, ChildContainer, Compose, Input, YoutubeDl,
    core::io::{MediaSource, ReadOnlySource},
};
use std::io::BufRead;
use std::process::{Command as StdCommand, Stdio};
use std::sync::OnceLock;
use tokio::process::Command as TokioCommand;
use tracing::info;

const YTDL_COMMAND: &str = "yt-dlp";

/// Prefer opus (webm) audio, which Discord can use with minimal transcoding.
const YTDL_FORMAT: &str = "ba[acodec=opus]/ba[abr>0][vcodec=none]/best";

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(reqwest::Client::new)
}

pub struct YouTube {}

impl YouTube {
    pub fn extract(query: &str) -> Option<QueryType> {
        // Check for playlist parameter in URL query string
        // Match ?list= or &list= to avoid matching URLs with "list=" in other parts
        if query.contains("?list=") || query.contains("&list=") {
            Some(QueryType::PlaylistLink(query.to_string()))
        } else {
            Some(QueryType::VideoLink(query.to_string()))
        }
    }
}

/// A lazy input that lets yt-dlp download the audio itself and stream it to stdout.
///
/// Unlike songbird's [`YoutubeDl`], which only asks yt-dlp for a media URL and then
/// fetches it with reqwest, this keeps yt-dlp's own downloader (chunked range requests,
/// retries, client-specific headers), which YouTube is far less likely to reject with 403.
/// The process is spawned only when the track starts, so queued tracks don't hold
/// expiring URLs or idle processes. The stream is not seekable.
#[derive(Clone, Debug)]
pub struct YtDlpStdout {
    url: String,
    metadata: AuxMetadata,
}

impl YtDlpStdout {
    fn spawn(&self) -> std::io::Result<ChildContainer> {
        let child = StdCommand::new(YTDL_COMMAND)
            .args([
                "-f",
                YTDL_FORMAT,
                "--no-playlist",
                "--quiet",
                "--no-progress",
                "-o",
                "-",
                "--",
                &self.url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Let yt-dlp errors reach the bot's logs.
            .stderr(Stdio::inherit())
            .spawn()?;

        Ok(ChildContainer::from(child))
    }
}

#[async_trait]
impl Compose for YtDlpStdout {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        let container = self.spawn().map_err(|e| {
            AudioStreamError::Fail(if e.kind() == std::io::ErrorKind::NotFound {
                format!("could not find executable '{YTDL_COMMAND}' on path").into()
            } else {
                Box::new(e)
            })
        })?;
        Ok(AudioStream {
            input: Box::new(ReadOnlySource::new(container)) as Box<dyn MediaSource>,
        })
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        self.create()
    }

    fn should_create_async(&self) -> bool {
        false
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        Ok(self.metadata.clone())
    }
}

impl From<YtDlpStdout> for Input {
    fn from(val: YtDlpStdout) -> Self {
        Input::Lazy(Box::new(val))
    }
}

pub struct YouTubeRestartable {}

impl YouTubeRestartable {
    /// Fetches metadata with `yt-dlp -j` and returns an input that streams via yt-dlp stdout.
    async fn stream_from(
        mut source: YoutubeDl<'static>,
        fallback_url: Option<String>,
    ) -> Result<(Input, AuxMetadata), crate::errors::ParrotError> {
        let metadata = source.aux_metadata().await.map_err(|e| {
            crate::errors::ParrotError::TrackFail(format!("Failed to get metadata: {}", e))
        })?;

        let url = metadata
            .source_url
            .clone()
            .or(fallback_url)
            .ok_or_else(|| {
                crate::errors::ParrotError::TrackFail("yt-dlp returned no video URL".to_string())
            })?;

        info!("Got track: {url} {:?}", metadata.title);
        let input = YtDlpStdout {
            url,
            metadata: metadata.clone(),
        };

        Ok((input.into(), metadata))
    }

    /// Creates a YouTube input and fetches its metadata
    pub async fn ytdl<P: AsRef<str> + Send + Clone + Sync + 'static>(
        uri: P,
    ) -> Result<(Input, AuxMetadata), crate::errors::ParrotError> {
        let uri = uri.as_ref().to_string();
        let source = YoutubeDl::new(get_http_client().clone(), uri.clone());
        Self::stream_from(source, Some(uri)).await
    }

    /// Creates a YouTube search input and fetches its metadata
    pub async fn ytdl_search<P: AsRef<str> + Send + Clone + Sync + 'static>(
        uri: P,
    ) -> Result<(Input, AuxMetadata), crate::errors::ParrotError> {
        let source = YoutubeDl::new_search(get_http_client().clone(), uri.as_ref().to_string());
        Self::stream_from(source, None).await
    }

    pub async fn ytdl_playlist(uri: &str, mode: Mode) -> Option<Vec<String>> {
        let mut args = vec![uri, "--flat-playlist", "-j"];
        match mode {
            Mode::Reverse => args.push("--playlist-reverse"),
            Mode::Shuffle => args.push("--playlist-random"),
            _ => {}
        }

        let output = TokioCommand::new(YTDL_COMMAND)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;

        let lines: Vec<String> = output
            .stdout
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| {
                let entry: Value = serde_json::from_str(&line).ok()?;
                entry
                    .get("webpage_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use songbird::input::codecs::{get_codec_registry, get_probe};

    /// Needs network + yt-dlp: `cargo test -- --ignored ytdlp_stdout`
    #[tokio::test]
    #[ignore]
    async fn ytdlp_stdout_decodes() {
        let (input, meta) = YouTubeRestartable::ytdl("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
            .await
            .expect("metadata");
        assert!(meta.title.is_some());
        let input = input
            .make_playable_async(get_codec_registry(), get_probe())
            .await
            .expect("probe");
        let Input::Live(songbird::input::LiveInput::Parsed(mut parsed), _) = input else {
            panic!("not parsed");
        };
        for _ in 0..50 {
            let packet = parsed.format.next_packet().expect("packet");
            parsed.decoder.decode(&packet).expect("decode");
        }
    }
}
