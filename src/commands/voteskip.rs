use crate::{
    commands::skip::{create_skip_response, force_skip_top_track},
    connection::get_voice_channel_for_user,
    errors::{ParrotError, verify},
    guild::cache::GuildCacheMap,
    messaging::message::ParrotMessage,
    utils::{command_guild_id, create_response},
};
use serenity::{
    all::{CommandInteraction, GuildId},
    client::Context,
    prelude::{Mentionable, RwLock, TypeMap},
};
use std::{cmp::max, collections::HashSet, sync::Arc};
use tracing::instrument;

#[instrument(level = "info", skip_all)]
pub async fn voteskip(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = command_guild_id(interaction)?;

    let listeners = {
        let guild = ctx.cache.guild(guild_id).ok_or(ParrotError::NotConnected)?;
        let bot_id = ctx.cache.current_user().id;

        let bot_channel_id =
            get_voice_channel_for_user(&guild, &bot_id).ok_or(ParrotError::NotConnected)?;

        guild
            .voice_states
            .iter()
            .filter(|(_, voice_state)| {
                voice_state.channel_id == Some(bot_channel_id) && voice_state.user_id != bot_id
            })
            .count()
    };

    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).ok_or(ParrotError::NotConnected)?;
    let handler = call.lock().await;
    let queue = handler.queue();

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;

    let mut data = ctx.data.write().await;
    let cache_map = data.get_mut::<GuildCacheMap>().unwrap();

    let cache = cache_map.entry(guild_id).or_default();
    cache.current_skip_votes.insert(interaction.user.id);

    let skip_threshold = max(1, listeners / 2);

    if cache.current_skip_votes.len() >= skip_threshold {
        force_skip_top_track(queue).await?;
        create_skip_response(ctx, interaction, queue, 1).await
    } else {
        create_response(
            &ctx.http,
            interaction,
            ParrotMessage::VoteSkip {
                mention: interaction.user.id.mention(),
                missing: skip_threshold - cache.current_skip_votes.len(),
            },
        )
        .await
    }
}

pub async fn forget_skip_votes(
    data: &Arc<RwLock<TypeMap>>,
    guild_id: GuildId,
) -> Result<(), ParrotError> {
    let mut data = data.write().await;

    let cache_map = data
        .get_mut::<GuildCacheMap>()
        .ok_or(ParrotError::Other("guild cache missing"))?;
    let cache = cache_map
        .get_mut(&guild_id)
        .ok_or(ParrotError::Other("guild cache entry missing"))?;
    cache.current_skip_votes = HashSet::new();

    Ok(())
}
