use async_trait::async_trait;
use dashmap::DashMap;
use log::{info, warn};
use parking_lot::Mutex;
use serenity::all::Context;
use songbird::{
    error::TrackResult,
    events::EventData,
    id::GuildId,
    input::{AuxMetadata, Input},
    tracks::{Track, TrackHandle},
    typemap::TypeMapKey,
    Driver, Event, EventContext, EventHandler, TrackEvent,
};
use std::{collections::VecDeque, ops::Deref, sync::Arc, time::Duration};

/// A simple queue for several audio sources, designed to
/// play in sequence.
///
/// This makes use of [`TrackEvent`]s to determine when the current
/// song or audio file has finished before playing the next entry.
///
/// One of these is automatically included via [`Driver::queue`] when
/// the `"builtin-queue"` feature is enabled.
///
/// `examples/serenity/voice_events_queue` demonstrates how a user might manage,
/// track and use this to run a song queue in many guilds in parallel.
/// This code is trivial to extend if extra functionality is needed.
///
/// # Example
///
/// ```rust,no_run
/// use songbird::{
///     driver::Driver,
///     id::GuildId,
///     input::File,
///     tracks::TrackQueue,
/// };
/// use std::collections::HashMap;
/// use std::num::NonZeroU64;
///
/// # async {
/// let guild = GuildId(NonZeroU64::new(1).unwrap());
/// // A Call is also valid here!
/// let mut driver: Driver = Default::default();
///
/// let mut queues: HashMap<GuildId, TrackQueue> = Default::default();
///
/// let source = File::new("../audio/my-favourite-song.mp3");
///
/// // We need to ensure that this guild has a TrackQueue created for it.
/// let queue = queues.entry(guild)
///     .or_default();
///
/// // Queueing a track is this easy!
/// queue.add_source(source.into(), &mut driver);
/// # };
/// ```
///
/// [`TrackEvent`]: crate::events::TrackEvent
/// [`Driver::queue`]: crate::driver::Driver
#[derive(Clone, Debug, Default)]
pub struct TrackQueue {
    // NOTE: the choice of a parking lot mutex is quite deliberate
    inner: Arc<Mutex<TrackQueueCore>>,
}

/// Reference to a track which is known to be part of a queue.
///
/// Instances *should not* be moved from one queue to another.
#[derive(Debug, Clone)]
pub struct Queued(pub TrackHandle, pub AuxMetadata);

impl Deref for Queued {
    type Target = TrackHandle;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Queued {
    /// Clones the inner handle
    #[must_use]
    pub fn handle(&self) -> TrackHandle {
        self.0.clone()
    }
}

impl TypeMapKey for TrackQueue {
    type Value = DashMap<GuildId, Arc<Mutex<TrackQueueCore>>>;
}

#[derive(Debug, Default)]
/// Inner portion of a [`TrackQueue`].
///
/// This abstracts away thread-safety from the user,
/// and offers a convenient location to store further state if required.
///
/// [`TrackQueue`]: TrackQueue
pub struct TrackQueueCore {
    tracks: VecDeque<Queued>,
}

struct QueueHandler {
    remote_lock: Arc<Mutex<TrackQueueCore>>,
}

#[async_trait]
impl EventHandler for QueueHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let mut inner = self.remote_lock.lock();

        // Due to possibility that users might remove, reorder,
        // or dequeue+stop tracks, we need to verify that the FIRST
        // track is the one who has ended.
        match ctx {
            EventContext::Track(ts) => {
                // This slice should have exactly one entry.
                // If the ended track has same id as the queue head, then
                // we can progress the queue.
                if inner.tracks.front()?.uuid() != ts.first()?.1.uuid() {
                    return None;
                }
            }
            _ => return None,
        }

        let _old = inner.tracks.pop_front();

        info!("Queued track ended: {:?}.", ctx);
        info!("{} tracks remain.", inner.tracks.len());

        // Keep going until we find one track which works, or we run out.
        while let Some(new) = inner.tracks.front() {
            if new.play().is_err() {
                // Discard files which cannot be used for whatever reason.
                warn!("Track in Queue couldn't be played...");
                inner.tracks.pop_front();
            } else {
                break;
            }
        }

        None
    }
}

struct SongPreloader {
    remote_lock: Arc<Mutex<TrackQueueCore>>,
}

#[async_trait]
impl EventHandler for SongPreloader {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let inner = self.remote_lock.lock();

        if let Some(track) = inner.tracks.get(1) {
            // This is the sync-version so that we can fire and ignore
            // the request ASAP.
            drop(track.0.make_playable());
        }

        None
    }
}

impl TrackQueue {
    /// Create a new, empty, track queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrackQueueCore {
                tracks: VecDeque::new(),
            })),
        }
    }

    /// Adds an audio source to the queue, to be played in the channel managed by `driver`.
    ///
    /// This method will preload the next track 5 seconds before the current track ends, if
    /// the [`AuxMetadata`] can be successfully queried for a [`Duration`].
    ///
    /// [`AuxMetadata`]: crate::input::AuxMetadata
    pub async fn add_source(&self, input: Input, driver: &mut Driver) -> Option<TrackHandle> {
        self.add(input.into(), driver).await
    }

    /// Adds a [`Track`] object to the queue, to be played in the channel managed by `driver`.
    ///
    /// This allows additional configuration or event handlers to be added
    /// before enqueueing the audio track. [`Track`]s will be paused pre-emptively.
    ///
    /// This method will preload the next track 5 seconds before the current track ends, if
    /// the [`AuxMetadata`] can be successfully queried for a [`Duration`].
    ///
    /// [`AuxMetadata`]: crate::input::AuxMetadata
    pub async fn add(&self, mut track: Track, driver: &mut Driver) -> Option<TrackHandle> {
        let preload_time = Self::get_preload_time(&mut track).await;
        self.add_with_preload(track, driver, preload_time).await
    }

    pub(crate) async fn get_preload_time(track: &mut Track) -> Option<Duration> {
        let meta = match track.input {
            Input::Lazy(ref mut rec) | Input::Live(_, Some(ref mut rec)) => {
                rec.aux_metadata().await.ok()
            }
            Input::Live(_, None) => None,
        };

        meta.and_then(|meta| meta.duration)
            .map(|d| d.saturating_sub(Duration::from_secs(5)))
    }

    /// Add an existing [`Track`] to the queue, using a known time to preload the next track.
    ///
    /// `preload_time` can be specified to enable gapless playback: this is the
    /// playback position *in this track* when the the driver will begin to load the next track.
    /// The standard [`Self::add`] method use [`AuxMetadata`] to set this to 5 seconds before
    /// a track ends.
    ///
    /// A `None` value will not ready the next track until this track ends, disabling preload.
    ///
    /// [`AuxMetadata`]: crate::input::AuxMetadata
    pub async fn add_with_preload(
        &self,
        mut track: Track,
        driver: &mut Driver,
        preload_time: Option<Duration>,
    ) -> Option<TrackHandle> {
        // Attempts to start loading the next track before this one ends.
        // Idea is to provide as close to gapless playback as possible,
        // while minimising memory use.
        info!("Track added to queue.");

        let remote_lock = self.inner.clone();
        track.events.add_event(
            EventData::new(Event::Track(TrackEvent::End), QueueHandler { remote_lock }),
            Duration::ZERO,
        );

        if let Some(time) = preload_time {
            let remote_lock = self.inner.clone();
            track.events.add_event(
                EventData::new(Event::Delayed(time), SongPreloader { remote_lock }),
                Duration::ZERO,
            );
        }
        let meta = track.input.aux_metadata().await.ok()?;

        let (should_play, handle) = {
            let mut inner = self.inner.lock();

            let handle = driver.play(track.pause());
            inner.tracks.push_back(Queued(handle.clone(), meta));

            (inner.tracks.len() == 1, handle)
        };

        if should_play {
            drop(handle.play());
        }

        Some(handle)
    }

    #[must_use]
    pub fn current(&self) -> Option<Queued> {
        let inner = self.inner.lock();

        inner.tracks.front().cloned()
    }

    /// Attempts to remove a track from the specified index.
    ///
    /// The returned entry can be readded to *this* queue via [`modify_queue`].
    ///
    /// [`modify_queue`]: TrackQueue::modify_queue
    #[must_use]
    pub fn dequeue(&self, index: usize) -> Option<Queued> {
        self.modify_queue(|vq| vq.remove(index))
    }

    /// Returns the number of tracks currently in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        let inner = self.inner.lock();

        inner.tracks.len()
    }

    /// Returns whether there are no tracks currently in the queue.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let inner = self.inner.lock();

        inner.tracks.is_empty()
    }

    /// Allows modification of the inner queue (i.e., deletion, reordering).
    ///
    /// Users must be careful to `stop` removed tracks, so as to prevent
    /// resource leaks.
    pub fn modify_queue<F, O>(&self, func: F) -> O
    where
        F: FnOnce(&mut VecDeque<Queued>) -> O,
    {
        let mut inner = self.inner.lock();
        func(&mut inner.tracks)
    }

    /// Pause the track at the head of the queue.
    pub fn pause(&self) -> TrackResult<()> {
        let inner = self.inner.lock();

        if let Some(handle) = inner.tracks.front() {
            handle.pause()
        } else {
            Ok(())
        }
    }

    /// Resume the track at the head of the queue.
    pub fn resume(&self) -> TrackResult<()> {
        let inner = self.inner.lock();

        if let Some(handle) = inner.tracks.front() {
            handle.play()
        } else {
            Ok(())
        }
    }

    /// Stop the currently playing track, and clears the queue.
    pub fn stop(&self) {
        let mut inner = self.inner.lock();

        for track in inner.tracks.drain(..) {
            // Errors when removing tracks don't really make
            // a difference: an error just implies it's already gone.
            drop(track.stop());
        }
    }

    /// Skip to the next track in the queue, if it exists.
    pub fn skip(&self) -> TrackResult<()> {
        let inner = self.inner.lock();

        inner.stop_current()
    }

    /// Returns a list of currently queued tracks.
    ///
    /// Does not allow for modification of the queue, instead returns a snapshot of the queue at the time of calling.
    ///
    /// Use [`modify_queue`] for direct modification of the queue.
    ///
    /// [`modify_queue`]: TrackQueue::modify_queue
    #[must_use]
    pub fn current_queue(&self) -> Vec<Queued> {
        let inner = self.inner.lock();

        inner.tracks.clone().into()
    }
}

impl TrackQueueCore {
    /// Skip to the next track in the queue, if it exists.
    fn stop_current(&self) -> TrackResult<()> {
        if let Some(handle) = self.tracks.front() {
            handle.stop()
        } else {
            Ok(())
        }
    }
}

pub async fn get_queue<G: Into<GuildId>>(ctx: &Context, guild_id: G) -> TrackQueue {
    // calls: DashMap<GuildId, Arc<Mutex<Call>>>,
    let data = ctx.data.read().await;

    let qs = data
        .get::<crate::utils::queue::TrackQueue>()
        .expect("Missing Queue mapping!");

    let x = qs.entry(guild_id.into()).or_default().downgrade().clone();
    TrackQueue { inner: x }
}
