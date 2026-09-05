use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    env,
    io::{self, Read, Seek, Write},
    sync::Arc,
    thread,
    time::Duration,
};

use base64::{Engine, engine::general_purpose};
use librespot::{
    core::{SpotifyUri, authentication::Credentials, config::SessionConfig, session::Session},
    metadata::Track,
    playback::{audio_backend::SinkError, config::PlayerConfig, decoder::AudioPacket, local_file},
};
use parking_lot::Mutex;
use rcgen::{CertificateParams, KeyPair};
use regex::Regex;
use reqwest::header;
use serde::Deserialize;
use serenity::{
    all::{CommandInteraction, Context},
    async_trait,
};
use snafu::Snafu;
use songbird::input::{AudioStream, AudioStreamError, AuxMetadata, Compose, Input, RawAdapter};
use symphonia::core::io::MediaSource;
use tiny_http::{Server, SslConfig};
use tokio::sync::Mutex as TMutex;
use tokio::{runtime::Handle, sync::oneshot};
use tracing::{error, info, instrument};
use url::Url;
use zerocopy::IntoBytes;

use librespot::playback::player::Decoder;
use librespot::playback::player::PlayerTrackLoader;

use lazy_static::lazy_static;
use thiserror::Error;

use crate::{
    commands::play::QueryType,
    errors::ParrotError,
    global_settings::GlobalSettingsMap,
    messaging::message::ParrotMessage,
    utils::{create_response, create_response_text},
};
use librespot::metadata::Metadata;
use std::str::FromStr;

use super::spotify::MediaType;

lazy_static! {
    pub static ref SPOTIFY_QUERY_REGEX: Regex =
        Regex::new(r"spotify.com/(?P<media_type>.+)/(?P<media_id>.*?)(?:\?|$)").unwrap();
    pub static ref RESPOT: TMutex<Result<Respot, ParrotError>> =
        TMutex::new(Err(ParrotError::Other("no auth respot attempts")));
}

#[derive(Clone)]
pub struct Respot {
    session: Session,
}

impl Respot {
    pub fn get_session(&self) -> Session {
        self.session.clone()
    }

    pub async fn new_track(id: SpotifyUri) -> Input {
        RespotTrack::new(id).into()
    }

    pub fn from_share_link(query: &str) -> Result<SpotifyUri, ParrotError> {
        // https://open.spotify.com/track/4XaPPJBrtBleKqcuXXRloI?si=547271dfe77249f9
        let captures = SPOTIFY_QUERY_REGEX
            .captures(query)
            .ok_or(ParrotError::Spotify(SpotifyError::InvalidQuery))?;

        let media_type = captures
            .name("media_type")
            .ok_or(ParrotError::Spotify(SpotifyError::InvalidQuery))?
            .as_str();

        let media_type = MediaType::from_str(media_type)
            .map_err(|_| ParrotError::Spotify(SpotifyError::InvalidQuery))?;

        let media_id = captures
            .name("media_id")
            .ok_or(ParrotError::Spotify(SpotifyError::InvalidQuery))?
            .as_str();

        match media_type {
            MediaType::Track => SpotifyUri::from_uri(&format!("spotify:track:{media_id}"))
                .map_err(|_| ParrotError::Spotify(SpotifyError::InvalidQuery)),
            MediaType::Album => SpotifyUri::from_uri(&format!("spotify:album:{media_id}"))
                .map_err(|_| ParrotError::Spotify(SpotifyError::InvalidQuery)),
            MediaType::Playlist => SpotifyUri::from_uri(&format!("spotify:playlist:{media_id}"))
                .map_err(|_| ParrotError::Spotify(SpotifyError::InvalidQuery)),
        }
    }
}

impl Respot {
    pub async fn auth(access_token: &str) -> Result<Self, ParrotError> {
        let session_config = SessionConfig::default();
        let credentials = Credentials::with_access_token(access_token);

        info!("Connecting librespot..");
        let session = Session::new(session_config, None);
        session.connect(credentials, false).await.unwrap();

        info!("CONNECTED");

        Ok(Self { session })
    }

    pub async fn reauth(refresh_token: &str) -> Result<Self, ParrotError> {
        let (client_id, client_secret) = match (
            env::var("SPOTIFY_CLIENT_ID"),
            env::var("SPOTIFY_CLIENT_SECRET"),
        ) {
            (Ok(id), Ok(secret)) => (id, secret),
            _ => return Err(ParrotError::Spotify(SpotifyError::AuthMissing)),
        };

        let encoded_pair = general_purpose::STANDARD.encode(format!("{client_id}:{client_secret}"));
        let auth_value = format!("Basic {}", encoded_pair);

        let client = reqwest::Client::new();
        let resp = client
            .post("https://accounts.spotify.com/api/token")
            .header(header::AUTHORIZATION, auth_value)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                // ("client_id", &client_id), ONLY REQUIRED FOR PKCE extension
            ])
            .send()
            .await
            .map_err(SpotifyError::DoNotRedeem)?
            .json::<ReAuthResponse>()
            .await
            .map_err(SpotifyError::DoNotRedeem)?;

        Respot::auth(&resp.access_token).await
    }

    pub fn extract(id: SpotifyUri) -> QueryType {
        QueryType::SpotifyUri(id)
    }
}

#[derive(Debug, Snafu)]
pub enum RespotError {}

#[derive(Debug, Snafu)]
pub enum RespotSinkError {
    OnWrite,
    OpenFailure,
    NoOutput,
}

impl From<RespotError> for SinkError {
    fn from(e: RespotError) -> SinkError {
        let es = e.to_string();
        SinkError::OnWrite(es)
    }
}

pub struct RespotTrack {
    track: SpotifyUri,
}

impl RespotTrack {
    pub fn new(track: SpotifyUri) -> Self {
        info!("Playing {}...", track);

        Self { track }
    }
}

struct RespotDecoder {
    decoder: Mutex<Decoder>,
    internal_buffer: VecDeque<u8>,
}

impl Seek for RespotDecoder {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::other("Don't support seek"))
    }
}

impl MediaSource for RespotDecoder {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl Read for RespotDecoder {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut d = self.decoder.lock();

        while let Some((_, pkt)) = d
            .next_packet()
            .map_err(|_| io::Error::other("Failed to get next spotify packet"))?
        {
            match pkt {
                AudioPacket::Samples(samples) => {
                    for b in samples.into_iter() {
                        self.internal_buffer.write((b as f32).as_bytes()).unwrap();
                    }
                }
                AudioPacket::Raw(vec) => {
                    self.internal_buffer.write(&vec).unwrap();
                }
            }

            if self.internal_buffer.len() >= buf.len() {
                break;
            }
        }
        self.internal_buffer.read(buf)
    }
}

#[async_trait]
impl Compose for RespotTrack {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        Err(AudioStreamError::Unsupported)
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        let respot_l = RESPOT.lock().await;
        let respot = respot_l.as_ref().unwrap();

        let ptl = PlayerTrackLoader {
            session: respot.session.clone(),
            config: PlayerConfig::default(),
            local_file_lookup: Arc::new(local_file::create_local_file_lookup(&[])),
        };
        drop(respot_l);

        info!("Creating in async!");

        let handle = Handle::current();
        let t = self.track.clone();
        let (result_tx, result_rx) = oneshot::channel();

        thread::spawn(move || {
            let data = handle.block_on(ptl.load_track(t, 0));
            if let Some(data) = data {
                let _ = result_tx.send(data);
            }
        });

        let track = result_rx.await.unwrap();

        info!("Got track");

        let input: Box<dyn MediaSource> = Box::new(RawAdapter::new(
            RespotDecoder {
                decoder: Mutex::new(track.decoder),
                internal_buffer: VecDeque::new(),
            },
            44100,
            2,
        ));

        Ok(AudioStream { input })
    }

    fn should_create_async(&self) -> bool {
        true
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        let l = RESPOT.lock().await;

        let respot = l.as_ref().map_err(|e| {
            error!("Got error when respot: {e:?}");
            AudioStreamError::Fail("".into())
        })?;

        let track = Track::get(&respot.session, &self.track)
            .await
            .expect("Failed to get track in respot");

        let artist = track
            .artists
            .iter()
            .fold(String::new(), |a, b| b.name.clone() + " " + &a);
        Ok(AuxMetadata {
            track: Some(track.name.clone()),
            artist: Some(artist),
            album: Some(track.album.name),
            date: None,
            channels: None,
            channel: None,
            start_time: Some(Duration::from_secs(0)),
            duration: Some(Duration::from_millis(track.duration as u64)),
            sample_rate: None,
            source_url: Some(format!(
                "https://open.spotify.com/track/{}",
                self.track.to_id()
            )),
            title: Some(track.name),
            thumbnail: None,
        })
    }
}

impl From<RespotTrack> for Input {
    fn from(val: RespotTrack) -> Self {
        Input::Lazy(Box::new(val))
    }
}

#[derive(Error, Debug)]
pub enum SpotifyError {
    #[error("State field on callback was bad")]
    InvalidState,
    #[error("Missing the auth parameter")]
    AuthMissing,
    #[error("Failed to start the server")]
    ServerFailed(Box<dyn std::error::Error + Send + Sync>),
    #[error("Some other error occured: {0:?}")]
    Other(Box<dyn std::error::Error + Send>),
    #[error("No app configured, create one at: https://developer.spotify.com/")]
    AppMissing,
    #[error("Failed to reedem the auth code {0:?}")]
    DoNotRedeem(reqwest::Error),
    #[error("Incorrectly formatted spotify link")]
    InvalidQuery,
    #[error("Failed to load the track")]
    FailedTrackLoad,
    #[error("Not implemented (yet)")]
    Todo,
}

fn get_http_callback(state: &str) -> Result<String, SpotifyError> {
    let addr = "0.0.0.0:12401";
    let subject_alt_names = vec![addr.to_string()];

    let signing_key = KeyPair::generate().expect("Failed to gen keypair");
    let cert = CertificateParams::new(subject_alt_names)
        .expect("Failed to alt name")
        .self_signed(&signing_key)
        .expect("Failed to generate cert param");

    let conf = SslConfig {
        certificate: cert.pem().into(),
        private_key: signing_key.serialize_pem().into(),
    };

    let server = match Server::https(addr, conf) {
        Ok(s) => s,
        Err(e) => {
            error!("Server failed {:?}", e);
            return Err(SpotifyError::ServerFailed(e));
        }
    };

    // This will block until the request!
    let req = server
        .recv()
        .map_err(|e| SpotifyError::Other(Box::new(e)))?;

    let base = Url::parse("https://{addr}").expect("My own url??");
    let url = base
        .join(req.url())
        .map_err(|e| SpotifyError::Other(Box::new(e)))?;

    let params: HashMap<Cow<_>, Cow<_>> = url.query_pairs().into_iter().collect();

    let cb_state = params.get("state").ok_or(SpotifyError::InvalidState)?;

    if cb_state != state {
        error!("Spotify auth state mismatch! '{cb_state}' vs expected: '{state}'");
        return Err(SpotifyError::InvalidState);
    }

    let code = params
        .get("code")
        .map(|s| s.to_string())
        .ok_or(SpotifyError::AuthMissing)?;

    // Don't really care, just being nice
    let _ = req.respond(tiny_http::Response::from_string("SUCCESS!"));

    return Ok(code);
}

#[derive(Deserialize, Debug)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize, Debug)]
struct ReAuthResponse {
    access_token: String,
}

#[instrument(level = "info", skip_all)]
pub async fn login(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let (client_id, client_secret) = match (
        env::var("SPOTIFY_CLIENT_ID"),
        env::var("SPOTIFY_CLIENT_SECRET"),
    ) {
        (Ok(id), Ok(secret)) => (id, secret),
        _ => return Err(ParrotError::Spotify(SpotifyError::AuthMissing)),
    };

    if RESPOT.lock().await.is_ok() {
        create_response_text(&ctx.http, interaction, "Spotify already logged in!").await?;
        info!("Already logged in");
        return Ok(());
    }

    let redirect_url = interaction
        .data
        .options
        .first()
        .and_then(|opt| opt.value.as_str())
        .map(String::from)
        .ok_or_else(|| {
            error!("Failed to read the login redirect url");
            ParrotError::Other("Requires a redirect url arg!")
        })?;

    info!("Got redirect url: {redirect_url}");

    let scope = vec![
        "streaming",
        "playlist-read-private",
        "app-remote-control",
        "user-read-currently-playing",
        "user-modify-playback-state",
        "user-read-playback-state",
    ]
    .join(" ");

    let state = "blep";

    let auth_url = Url::parse_with_params(
        "https://accounts.spotify.com/authorize",
        &[
            ("client_id", client_id.as_str()),
            ("response_type", "code"),
            ("scope", &scope),
            ("redirect_uri", &redirect_url),
            ("state", &state),
        ],
    )
    .expect("url wut??");

    info!("Starting callback webserver!");
    let result_handle = tokio::task::spawn_blocking(move || get_http_callback(&state));

    create_response(
        &ctx.http,
        interaction,
        ParrotMessage::Login {
            url: auth_url.to_string(),
        },
    )
    .await
    .expect("Failed to reply");

    info!("Sent auth message, await response");
    let auth_code = result_handle.await.expect("Join failed")?;

    println!("Auth code = {auth_code}");

    let (access_token, refresh_token) =
        get_tokens(&auth_code, &redirect_url, &client_id, &client_secret).await?;

    let mut d = ctx.data.write().await;
    let global_settings = d.get_mut::<GlobalSettingsMap>().unwrap();

    global_settings.set_tokens(&refresh_token, &access_token);

    if let Err(e) = global_settings.save() {
        error!("Error: {:?} while saving spotify settings", e);
    }

    *RESPOT.lock().await = Respot::auth(&access_token).await;

    Ok(())
}

async fn get_tokens(
    auth_code: &str,
    uri: &str,
    id: &str,
    secret: &str,
) -> Result<(String, String), SpotifyError> {
    let client = reqwest::Client::new();

    // Base 64 encoded string that contains the client ID and client secret key.
    // The field must have the format: Authorization: Basic <base64 encoded
    // client_id:client_secret>
    let encoded_pair = general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let auth_value = format!("Basic {}", encoded_pair);

    info!("Redeeming auth code for an access code");
    let resp = client
        .post("https://accounts.spotify.com/api/token")
        .header(header::AUTHORIZATION, auth_value)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", auth_code),
            ("redirect_uri", uri),
        ])
        .send()
        .await
        .map_err(SpotifyError::DoNotRedeem)?
        .json::<AuthResponse>()
        .await
        .map_err(SpotifyError::DoNotRedeem)?;

    Ok((resp.access_token, resp.refresh_token))
}
