use crate::{
    commands::play::QueryType,
    errors::ParrotError,
    messaging::messages::{SPOTIFY_INVALID_QUERY, SPOTIFY_PLAYLIST_FAILED},
};
use futures::{StreamExt, TryStreamExt};
use lazy_static::lazy_static;
use rspotify::{
    ClientCredsSpotify, ClientError, Credentials,
    clients::BaseClient,
    model::{
        AlbumId, FullTrack, Image, PlayableItem, PlaylistId, SimplifiedArtist, SimplifiedTrack,
        TrackId,
    },
};
use songbird::input::AuxMetadata;
use std::{env, str::FromStr, time::Duration};
use tokio::sync::Mutex;
use tracing::error;
use url::Url;

lazy_static! {
    pub static ref SPOTIFY: Mutex<Result<ClientCredsSpotify, ParrotError>> =
        Mutex::new(Err(ParrotError::Other("no auth attempts")));
}

/// Default upper bound on tracks queued from one album/playlist; override with `SPOTIFY_MAX_TRACKS`.
const DEFAULT_MAX_TRACKS: usize = 500;

fn max_tracks() -> usize {
    env::var("SPOTIFY_MAX_TRACKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_TRACKS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Track,
    Album,
    Playlist,
}

impl FromStr for MediaType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "track" => Ok(Self::Track),
            "album" => Ok(Self::Album),
            "playlist" => Ok(Self::Playlist),
            _ => Err(()),
        }
    }
}

/// A Spotify track's metadata, resolved to a YouTube stream only when it starts playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotifyTrack {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub artwork: Option<String>,
    pub url: Option<String>,
}

impl SpotifyTrack {
    fn from_full(track: &FullTrack) -> Self {
        Self {
            title: track.name.clone(),
            artists: artist_names(&track.artists),
            album: Some(track.album.name.clone()),
            duration: track.duration.to_std().ok(),
            artwork: first_image(&track.album.images),
            url: track.external_urls.get("spotify").cloned(),
        }
    }

    fn from_simplified(track: &SimplifiedTrack, album: &str, artwork: Option<String>) -> Self {
        Self {
            title: track.name.clone(),
            artists: artist_names(&track.artists),
            album: Some(album.to_string()),
            duration: track.duration.to_std().ok(),
            artwork,
            url: track.external_urls.get("spotify").cloned(),
        }
    }

    /// The YouTube search query used to find this track.
    pub fn search_query(&self) -> String {
        format!("{} - {} audio", self.artists.join(", "), self.title)
    }

    pub fn aux_metadata(&self) -> AuxMetadata {
        let artists = self.artists.join(", ");
        AuxMetadata {
            title: Some(self.title.clone()),
            track: Some(self.title.clone()),
            artist: Some(artists.clone()),
            channel: Some(artists),
            album: self.album.clone(),
            duration: self.duration,
            thumbnail: self.artwork.clone(),
            source_url: self.url.clone(),
            ..Default::default()
        }
    }
}

fn artist_names(artists: &[SimplifiedArtist]) -> Vec<String> {
    artists.iter().map(|a| a.name.clone()).collect()
}

fn first_image(images: &[Image]) -> Option<String> {
    // Spotify returns images widest first.
    images.first().map(|i| i.url.clone())
}

/// Parses `https://open.spotify.com/[intl-xx/][embed/]{type}/{id}` links and `spotify:{type}:{id}` URIs.
pub fn parse_link(input: &str) -> Option<(MediaType, String)> {
    let (kind, id) = if let Some(rest) = input.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        (parts.next()?.to_string(), parts.next()?.to_string())
    } else {
        let url = Url::parse(input).ok()?;
        if !url.host_str()?.ends_with("spotify.com") {
            return None;
        }
        let mut segments = url
            .path_segments()?
            .filter(|s| !s.is_empty())
            .skip_while(|s| s.starts_with("intl-") || *s == "embed");
        (segments.next()?.to_string(), segments.next()?.to_string())
    };

    let media_type = MediaType::from_str(&kind).ok()?;
    let valid_id = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric());
    valid_id.then_some((media_type, id))
}

/// True for inputs that should be handled by the Spotify source.
pub fn is_spotify_link(input: &str) -> bool {
    input.starts_with("spotify:")
        || Url::parse(input)
            .ok()
            .and_then(|u| u.host_str().map(|h| h == "open.spotify.com"))
            .unwrap_or(false)
}

fn log_err(context: &'static str) -> impl FnOnce(ClientError) -> ClientError {
    move |e| {
        error!("Spotify {context} failed: {e:?}");
        e
    }
}

pub struct Spotify {}

impl Spotify {
    pub async fn auth() -> Result<ClientCredsSpotify, ParrotError> {
        let spotify_client_id = env::var("SPOTIFY_CLIENT_ID")
            .map_err(|_| ParrotError::Other("missing spotify client ID"))?;

        let spotify_client_secret = env::var("SPOTIFY_CLIENT_SECRET")
            .map_err(|_| ParrotError::Other("missing spotify client secret"))?;

        let creds = Credentials::new(&spotify_client_id, &spotify_client_secret);

        let spotify = ClientCredsSpotify::new(creds);
        spotify.request_token().await.map_err(log_err("auth"))?;

        Ok(spotify)
    }

    pub async fn extract(
        spotify: &ClientCredsSpotify,
        query: &str,
    ) -> Result<QueryType, ParrotError> {
        let (media_type, media_id) =
            parse_link(query).ok_or(ParrotError::Other(SPOTIFY_INVALID_QUERY))?;

        match media_type {
            MediaType::Track => Self::get_track_info(spotify, &media_id).await,
            MediaType::Album => Self::get_album_info(spotify, &media_id).await,
            MediaType::Playlist => Self::get_playlist_info(spotify, &media_id).await,
        }
    }

    async fn get_track_info(
        spotify: &ClientCredsSpotify,
        id: &str,
    ) -> Result<QueryType, ParrotError> {
        let track_id = TrackId::from_id(id)
            .map_err(|_| ParrotError::Other("track ID contains invalid characters"))?;

        let track = spotify
            .track(track_id, None)
            .await
            .map_err(log_err("track lookup"))
            .map_err(|_| ParrotError::Other("failed to fetch track"))?;

        Ok(QueryType::Spotify(Box::new(SpotifyTrack::from_full(
            &track,
        ))))
    }

    async fn get_album_info(
        spotify: &ClientCredsSpotify,
        id: &str,
    ) -> Result<QueryType, ParrotError> {
        let album_id = AlbumId::from_id(id)
            .map_err(|_| ParrotError::Other("album ID contains invalid characters"))?;

        let album = spotify
            .album(album_id.clone(), None)
            .await
            .map_err(log_err("album lookup"))
            .map_err(|_| ParrotError::Other("failed to fetch album"))?;
        let artwork = first_image(&album.images);

        // The album object only embeds the first page of tracks; page through all of them.
        let tracks: Vec<SimplifiedTrack> = spotify
            .album_track(album_id, None)
            .take(max_tracks())
            .try_collect()
            .await
            .map_err(log_err("album tracks"))
            .map_err(|_| ParrotError::Other("failed to fetch album"))?;

        let tracks = tracks
            .iter()
            .map(|t| SpotifyTrack::from_simplified(t, &album.name, artwork.clone()))
            .collect();

        Ok(QueryType::SpotifyList(tracks))
    }

    async fn get_playlist_info(
        spotify: &ClientCredsSpotify,
        id: &str,
    ) -> Result<QueryType, ParrotError> {
        let playlist_id = PlaylistId::from_id(id)
            .map_err(|_| ParrotError::Other("playlist ID contains invalid characters"))?;

        let items: Vec<_> = spotify
            .playlist_items(playlist_id, None, None)
            .take(max_tracks())
            .try_collect()
            .await
            .map_err(log_err("playlist items"))
            .map_err(|_| ParrotError::Other(SPOTIFY_PLAYLIST_FAILED))?;

        let tracks = items
            .iter()
            .filter(|item| !item.is_local)
            .filter_map(|item| match item.track.as_ref()? {
                PlayableItem::Track(t) => Some(SpotifyTrack::from_full(t)),
                PlayableItem::Episode(_) => None,
            })
            .collect();

        Ok(QueryType::SpotifyList(tracks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4cOdK2wGLETKBW3PvgPWqT";

    #[test]
    fn parses_web_links() {
        assert_eq!(
            parse_link(&format!("https://open.spotify.com/track/{ID}?si=abc123")),
            Some((MediaType::Track, ID.to_string()))
        );
        assert_eq!(
            parse_link(&format!("https://open.spotify.com/intl-de/album/{ID}")),
            Some((MediaType::Album, ID.to_string()))
        );
        assert_eq!(
            parse_link(&format!("https://open.spotify.com/embed/playlist/{ID}")),
            Some((MediaType::Playlist, ID.to_string()))
        );
    }

    #[test]
    fn parses_uris() {
        assert_eq!(
            parse_link(&format!("spotify:playlist:{ID}")),
            Some((MediaType::Playlist, ID.to_string()))
        );
        assert!(is_spotify_link(&format!("spotify:track:{ID}")));
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(
            parse_link(&format!("https://open.spotify.com/show/{ID}")),
            None
        );
        assert_eq!(parse_link("https://open.spotify.com/track/"), None);
        assert_eq!(parse_link(&format!("https://example.com/track/{ID}")), None);
        assert_eq!(parse_link("spotify:track:bad/id"), None);
        assert!(!is_spotify_link("https://www.youtube.com/watch?v=x"));
    }

    /// Needs SPOTIFY_CLIENT_ID/SECRET and SPOTIFY_TEST_PLAYLIST (a public playlist with >100
    /// tracks): `cargo test --lib -- --ignored spotify_`
    #[tokio::test]
    #[ignore]
    async fn spotify_playlist_paginates() {
        let spotify = Spotify::auth().await.expect("auth");
        let link = env::var("SPOTIFY_TEST_PLAYLIST").expect("set SPOTIFY_TEST_PLAYLIST");
        let QueryType::SpotifyList(tracks) =
            Spotify::extract(&spotify, &link).await.expect("fetch")
        else {
            panic!("expected list");
        };
        assert!(tracks.len() > 100, "only got {} tracks", tracks.len());
        assert!(tracks.iter().all(|t| !t.artists.is_empty()));
    }
}
