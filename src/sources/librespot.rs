use std::{
    collections::VecDeque,
    io::{self, Read, Seek, Write},
    thread,
    time::Duration,
};

use librespot::{
    core::{
        authentication::Credentials,
        config::SessionConfig,
        session::Session,
        spotify_id::{SpotifyId, SpotifyItemType},
    },
    playback::{audio_backend::SinkError, config::PlayerConfig, decoder::AudioPacket},
};
use parking_lot::Mutex;
use serenity::async_trait;
use snafu::Snafu;
use songbird::input::{AudioStream, AudioStreamError, AuxMetadata, Compose, Input, RawAdapter};
use symphonia::core::io::MediaSource;
use tokio::{runtime::Handle, sync::oneshot};
use tracing::{debug, info};
use zerocopy::IntoBytes;

use librespot::playback::player::Decoder;
use librespot::playback::player::PlayerTrackLoader;

use crate::errors::ParrotError;

pub static RESPOT: Mutex<Result<Respot, ParrotError>> =
    Mutex::new(Err(ParrotError::Other("no auth respot attempts")));

#[derive(Clone)]
pub struct Respot {
    session: Session,
}

impl Respot {
    pub fn get_session(&self) -> Session {
        self.session.clone()
    }
}

impl Respot {
    pub async fn auth(username: &str, password: &str) -> Result<Self, ParrotError> {
        let session_config = SessionConfig::default();

        let credentials = Credentials::with_access_token("");
        // let credentials = Credentials::with_password(username, password);

        info!("Connecting librespot..");
        let session = Session::new(session_config, None);
        session.connect(credentials, false).await.unwrap();

        info!("CONNECTED");

        Ok(Self { session })
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
    session: Session,
}

impl RespotTrack {
    pub fn new(id: &str, respot: Respot) -> Self {
        let mut track = SpotifyId::from_base62(id).unwrap();
        info!("Playing {}...", track);
        track.item_type = SpotifyItemType::Track;

        Self {
            track,
            session: respot.get_session(),
        }
    }
}

struct RespotDecoder {
    decoder: Mutex<Decoder>,
    internal_buffer: VecDeque<u8>,
}

impl Seek for RespotDecoder {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        todo!();
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

        debug!("Doing read of len {}", buf.len());
        while let Some((_, pkt)) = d.next_packet().map_err(|_| io::Error::other("aaaaa"))? {
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
        let ptl = PlayerTrackLoader {
            session: self.session.clone(),
            config: PlayerConfig::default(),
        };

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

        Ok(AudioStream { input, hint: None })
    }

    fn should_create_async(&self) -> bool {
        true
    }

    async fn aux_metadata(&mut self) -> Result<AuxMetadata, AudioStreamError> {
        Ok(AuxMetadata {
            track: Some("dank memers track".to_string()),
            artist: Some(" memers".to_string()),
            album: Some(" memers".to_string()),
            date: Some(" memers".to_string()),
            channels: Some(2),
            channel: Some(" memers".to_string()),
            start_time: Some(Duration::from_secs(0)),
            duration: Some(Duration::from_secs(200)),
            sample_rate: None,
            source_url: Some("http://what/ever".to_string()),
            title: Some("memers".to_string()),
            thumbnail: Some("http://sfjdisfhjsdih/sdfji.png".to_string()),
        })
    }
}

impl From<RespotTrack> for Input {
    fn from(val: RespotTrack) -> Self {
        Input::Lazy(Box::new(val))
    }
}
