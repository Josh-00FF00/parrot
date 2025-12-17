use serenity::{all::CommandInteraction, async_trait, http::Http};
use songbird::{Event, EventContext, EventHandler, Songbird, tracks::PlayMode};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tracing::{error, info};

pub struct IdleHandler {
    pub http: Arc<Http>,
    pub manager: Arc<Songbird>,
    pub interaction: CommandInteraction,
    pub limit: usize,
    pub count: Arc<AtomicUsize>,
}

#[async_trait]
impl EventHandler for IdleHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let EventContext::Track(track_list) = ctx else {
            return None;
        };

        // looks like the track list isn't ordered here, so the first track in the list isn't
        // guaranteed to be the first track in the actual queue, so search the entire list
        let bot_is_playing = track_list
            .iter()
            .any(|track| matches!(track.0.playing, PlayMode::Play));

        // if there's a track playing, then reset the counter
        if bot_is_playing {
            self.count.store(0, Ordering::Relaxed);
            return None;
        }

        if self.count.fetch_add(1, Ordering::Relaxed) >= self.limit {
            info!("Bot's idled too long, disconnect!");
            let guild_id = self.interaction.guild_id?;

            if let Err(e) = self.manager.remove(guild_id).await {
                error!("Failed to disconnect from guild {guild_id} {e:?}");
            } else {
                info!("Disconnected from guild due to idle {guild_id}");
            }
        }

        None
    }
}
