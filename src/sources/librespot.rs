use std::sync::mpsc::{channel, Receiver, Sender};

use librespot::{
    core::{
        authentication::Credentials, config::SessionConfig, session::Session, spotify_id::SpotifyId,
    },
    playback::{
        audio_backend::{Sink, SinkError, SinkResult},
        config::PlayerConfig,
        convert::Converter,
        decoder::AudioPacket,
        mixer::NoOpVolume,
        player::Player,
    },
};
use log::info;
use parking_lot::Mutex;
use serenity::async_trait;
use snafu::Snafu;
use songbird::input::{AudioStream, AudioStreamError, Compose, HttpRequest};
use symphonia::core::io::MediaSource;
use zerocopy::IntoBytes;

use crate::errors::ParrotError;

pub static RESPOT: Mutex<Result<Respot, ParrotError>> =
    Mutex::new(Err(ParrotError::Other("no auth respot attempts")));

pub struct Respot {
    rx: Receiver<Vec<u8>>,
    player: Player,
}

impl Respot {
    pub async fn auth(username: String, password: String) -> Result<Self, ParrotError> {
        let session_config = SessionConfig::default();
        let player_config = PlayerConfig::default();
        let credentials = Credentials::with_password(username, password);

        info!("Connecting ..");
        let (session, _) = Session::connect(session_config, credentials, None, false)
            .await
            .unwrap();

        let (ims, rx) = InMemorySink::new();

        let (mut player, _) =
            Player::new(player_config, session, Box::new(NoOpVolume), move || {
                Box::new(ims)
            });
    }

    pub async fn play_track(&mut self, id: &str) {
        let track = SpotifyId::from_base62(id).unwrap();
        info!("Playing {}...", id);
        self.player.load(track, true, 0);
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
                let samples_s16: &[i16] = &converter.f64_to_s16(&samples);
                self.write_bytes(samples_s16.as_bytes())
            }
            AudioPacket::OggData(samples) => self.write_bytes(&samples),
        }
    }
}

struct InMemorySink {
    sender: Sender<Vec<u8>>,
}

impl InMemorySink {
    pub fn new() -> (Self, Receiver<Vec<u8>>) {
        let (tx, rx) = channel();
        (Self { sender: tx }, rx)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> SinkResult<()> {
        self.sender.send(bytes.to_vec());
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
        match e {
            OnWrite => SinkError::OnWrite(es),
            OpenFailure => SinkError::ConnectionRefused(es),
            NoOutput => SinkError::NotConnected(es),
        }
    }
}

#[async_trait]
impl Compose for Respot {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        Err(AudioStreamError::Unsupported)
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
    }

    fn should_create_async(&self) -> bool {
        true
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        if let Some(meta) = self.metadata.as_ref() {
            return Ok(meta.clone());
        }

        self.query(1).await?;

        self.metadata.clone().ok_or_else(|| {
            let msg: Box<dyn Error + Send + Sync + 'static> =
                "Failed to instansiate any metadata... Should be unreachable.".into();
            AudioStreamError::Fail(msg)
        })
    }
}
