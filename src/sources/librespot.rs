use std::{
    collections::VecDeque,
    io::{Read, Seek, Write},
    sync::{
        mpsc::{self, channel, Receiver, Sender},
        Arc,
    },
    time::Duration,
};

use librespot::{
    core::{
        authentication::Credentials,
        config::SessionConfig,
        session::Session,
        spotify_id::{SpotifyId, SpotifyItemType},
    },
    playback::{
        audio_backend::{Sink, SinkError, SinkResult},
        config::PlayerConfig,
        convert::Converter,
        decoder::AudioPacket,
        mixer::NoOpVolume,
        player::{Player, PlayerEvent, PlayerEventChannel},
    },
};
use parking_lot::Mutex;
use serenity::async_trait;
use snafu::Snafu;
use songbird::input::{
    AsyncAdapterStream, AudioStream, AudioStreamError, AuxMetadata, Compose, HttpRequest, Input,
    RawAdapter,
};
use symphonia::core::io::MediaSource;
use tracing::{debug, error, info, instrument};
use zerocopy::IntoBytes;

use crate::errors::ParrotError;

pub static RESPOT: Mutex<Result<Respot, ParrotError>> =
    Mutex::new(Err(ParrotError::Other("no auth respot attempts")));

#[derive(Clone)]
pub struct Respot {
    ims: InMemorySink,
    session: Session,
    player: Arc<Player>,
}

static OAUTH_SCOPES: &[&str] = &[
    "streaming",
    "user-modify-playback-state",
    "openid",
    "user-read-email",
    "user-read-private",
    "playlist-read",
    "playlist-read-collaborative",
    "playlist-read-private",
    "app-remote-control",
];

impl Respot {
    pub async fn auth(username: &str, password: &str) -> Result<Self, ParrotError> {
        let session_config = SessionConfig::default();
        let player_config = PlayerConfig::default();

        let credentials = Credentials::with_access_token("");
        // let credentials = Credentials::with_password(username, password);

        info!("Connecting librespot..");
        let session = Session::new(session_config, None);
        session.connect(credentials, false).await.unwrap();

        let ims = InMemorySink::new();
        let ims_c = ims.clone();

        let player = Player::new(
            player_config,
            session.clone(),
            Box::new(NoOpVolume),
            move || Box::new(ims_c),
        );

        info!("CONNECTED");

        Ok(Self {
            player,
            ims,
            session,
        })
    }
}

impl Sink for InMemorySink {
    fn start(&mut self) -> SinkResult<()> {
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        match packet {
            AudioPacket::Samples(samples) => {
                let samples_f32: &[f32] = &converter.f64_to_f32(&samples);
                self.0.write_bytes(samples_f32.as_bytes())
            }
            AudioPacket::Raw(samples) => self.0.write_bytes(&samples),
        }
    }
}

#[derive(Clone)]
struct InMemorySink(Arc<InMemorySinkInner>);

impl InMemorySink {
    fn new() -> Self {
        Self(InMemorySinkInner::new().into())
    }
}

struct InMemorySinkInner {
    buf: Mutex<VecDeque<u8>>,
    r: Receiver<Box<[u8]>>,
    w: Sender<Box<[u8]>>,
}

impl InMemorySinkInner {
    fn new() -> Self {
        let (w, r) = mpsc::channel();
        Self {
            buf: Mutex::new(VecDeque::new()),
            r,
            w,
        }
    }

    fn write_bytes(&self, bytes: &[u8]) -> SinkResult<()> {
        info!("Writing {} bytes into ims", bytes.len());
        let _ = self.buf.lock().write_all(bytes);
        let _ = self.w.send(bytes.into());
        Ok(())
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
    track: SpotifyId,
    player: Arc<Player>,
    ims: InMemorySink,
}

impl RespotTrack {
    pub fn new(id: &str, respot: Respot) -> Self {
        let mut track = SpotifyId::from_base62(id).unwrap();
        info!("Playing {}...", track);
        track.item_type = SpotifyItemType::Track;
        let player = respot.player.clone();
        let ims = respot.ims.clone();

        Self { player, ims, track }
    }
}

impl Read for InMemorySinkRunning {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        info!("Doing read of len {} ims", buf.len());

        loop {
            match self.events.try_recv() {
                Ok(e) => match e {
                    PlayerEvent::EndOfTrack { .. } => {
                        info!("End of track!");
                        return Ok(0);
                    }
                    _ => {}
                },
                Err(_) => {}
            }

            let mut b = self
                .ims
                .0
                .r
                .recv()
                .map_err(|_| std::io::Error::other("recv fail".into()))?;
            let read = b.read(buf);

            info!("ACTUAL READ, {:?}", read);
            return read;
        }
    }
}

impl Seek for InMemorySinkRunning {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        todo!()
    }
}

struct InMemorySinkRunning {
    ims: InMemorySink,
    events: PlayerEventChannel,
}

impl MediaSource for InMemorySinkRunning {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[async_trait]
impl Compose for RespotTrack {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        self.player.load(self.track, true, 0);

        info!("Starting Compose for RespotTrack");

        let input: Box<dyn MediaSource> = Box::new(RawAdapter::new(
            InMemorySinkRunning {
                ims: self.ims.clone(),
                events: self.player.get_player_event_channel(),
            },
            44100,
            2,
        ));

        Ok(AudioStream { input, hint: None })
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        Err(AudioStreamError::Unsupported)
    }

    fn should_create_async(&self) -> bool {
        false
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        Ok(AuxMetadata {
            track: Some("dank memers track".to_string()),
            artist: Some("Dankus memers".to_string()),
            album: Some("Dankus memers".to_string()),
            date: Some("Dankus memers".to_string()),
            channels: Some(2),
            channel: Some("Dankus memers".to_string()),
            start_time: Some(Duration::from_secs(0)),
            duration: Some(Duration::from_secs(200)),
            sample_rate: None,
            source_url: Some("http://fuck.what/ever".to_string()),
            title: Some("Dankus memers".to_string()),
            thumbnail: Some("http://sfjdisfhjsdih/sdfji.png".to_string()),
        })
    }
}

impl From<RespotTrack> for Input {
    fn from(val: RespotTrack) -> Self {
        Input::Lazy(Box::new(val))
    }
}
