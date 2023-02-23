use log::info;
use serenity::{
    all::EditMessage,
    async_trait,
    http::Http,
    model::id::GuildId,
    prelude::{RwLock, TypeMap},
};
use songbird::{Event, EventContext, EventHandler};
use std::sync::Arc;

use crate::{
    commands::{
        queue::{build_nav_btns, calculate_num_pages, create_queue_embed, forget_queue_message},
        voteskip::forget_skip_votes,
    },
    guild::{cache::GuildCacheMap, settings::GuildSettingsMap},
    utils::queue::{Queued, TrackQueue},
};

pub struct TrackEndHandler {
    pub guild_id: GuildId,
    pub queue: TrackQueue,
    pub ctx_data: Arc<RwLock<TypeMap>>,
}

pub struct ModifyQueueHandler {
    pub http: Arc<Http>,
    pub ctx_data: Arc<RwLock<TypeMap>>,
    pub queue: TrackQueue,
    pub guild_id: GuildId,
}

#[async_trait]
impl EventHandler for TrackEndHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let data_rlock = self.ctx_data.read().await;
        let settings = data_rlock.get::<GuildSettingsMap>().unwrap();

        let autopause = settings
            .get(&self.guild_id)
            .map(|guild_settings| guild_settings.autopause)
            .unwrap_or_default();

        if autopause {
            self.queue.pause().ok();
        }

        drop(data_rlock);
        forget_skip_votes(&self.ctx_data, self.guild_id).await.ok();

        None
    }
}

#[async_trait]
impl EventHandler for ModifyQueueHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        info!("Updating Queue song end...");
        update_queue_messages(
            &self.http,
            &self.ctx_data,
            &self.queue.current_queue(),
            self.guild_id,
        )
        .await;
        None
    }
}

pub async fn update_queue_messages(
    http: &Arc<Http>,
    ctx_data: &Arc<RwLock<TypeMap>>,
    tracks: &[Queued],
    guild_id: GuildId,
) {
    let data = ctx_data.read().await;
    let cache_map = data.get::<GuildCacheMap>().unwrap();

    let mut messages = match cache_map.get(&guild_id) {
        Some(cache) => cache.queue_messages.clone(),
        None => return,
    };
    drop(data);

    for (message, page_lock) in messages.iter_mut() {
        // has the page size shrunk?
        let num_pages = calculate_num_pages(tracks);
        let mut page = page_lock.write().await;
        *page = usize::min(*page, num_pages - 1);

        let embed = create_queue_embed(tracks, *page);

        let edit_message = message
            .edit(
                &http,
                EditMessage::new()
                    .add_embed(embed)
                    .components(build_nav_btns(*page, num_pages)),
            )
            .await;

        if edit_message.is_err() {
            forget_queue_message(ctx_data, message, guild_id).await.ok();
        };
    }
}
