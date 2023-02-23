use crate::{
    errors::{verify, ParrotError},
    messaging::message::ParrotMessage,
    utils::{
        create_response,
        queue::{get_queue, Queued, TrackQueue},
    },
};
use serenity::{all::CommandInteraction, client::Context};
use std::cmp::min;

pub async fn skip(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let args = interaction.data.options.clone();

    let to_skip = args.first().and_then(|a| a.value.as_i64()).unwrap_or(1) as usize;

    let queue = get_queue(ctx, guild_id).await;

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;

    let tracks_to_skip = min(to_skip, queue.len());

    queue.modify_queue(|v| {
        v.drain(1..tracks_to_skip);
    });

    force_skip_top_track(&queue).await?;
    create_skip_response(ctx, interaction, tracks_to_skip).await
}

pub async fn create_skip_response(
    ctx: &Context,
    interaction: &mut CommandInteraction,
    tracks_to_skip: usize,
) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let queue = get_queue(ctx, guild_id).await;

    match queue.current() {
        Some(track) => {
            create_response(
                &ctx.http,
                interaction,
                ParrotMessage::SkipTo {
                    title: track.1.title.unwrap(),
                    url: track.1.source_url.unwrap(),
                },
            )
            .await
        }
        None => {
            if tracks_to_skip > 1 {
                create_response(&ctx.http, interaction, ParrotMessage::SkipAll).await
            } else {
                create_response(&ctx.http, interaction, ParrotMessage::Skip).await
            }
        }
    }
}

pub async fn force_skip_top_track(queue: &TrackQueue) -> Result<Vec<Queued>, ParrotError> {
    // this is an odd sequence of commands to ensure the queue is properly updated
    // apparently, skipping/stopping a track takes a while to remove it from the queue
    // also, manually removing tracks doesn't trigger the next track to play
    // so first, stop the top song, manually remove it and then resume playback
    queue.current().unwrap().stop().ok();
    let _ = queue.dequeue(0);
    queue.resume().ok();

    Ok(queue.current_queue())
}
