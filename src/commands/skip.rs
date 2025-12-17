use crate::{
    errors::{ParrotError, verify},
    messaging::message::ParrotMessage,
    utils::{create_response, track_to_meta},
};
use serenity::{all::CommandInteraction, client::Context};
use songbird::tracks::{TrackHandle, TrackQueue};
use std::cmp::min;
use tracing::{info, instrument};

#[instrument(level = "info", skip_all)]
pub async fn skip(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let args = interaction.data.options.clone();

    let to_skip = args.first().and_then(|a| a.value.as_i64()).unwrap_or(1) as usize;

    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).unwrap();
    let handler = call.lock().await;
    let queue = handler.queue();

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;

    let tracks_to_skip = min(to_skip, queue.len());

    queue.modify_queue(|v| {
        v.drain(1..tracks_to_skip);
    });

    force_skip_top_track(&queue).await?;
    info!("Skipped! Creating response");
    create_skip_response(ctx, interaction, handler.queue(), tracks_to_skip).await
}

pub async fn create_skip_response(
    ctx: &Context,
    interaction: &mut CommandInteraction,
    queue: &TrackQueue,
    tracks_to_skip: usize,
) -> Result<(), ParrotError> {
    match queue.current() {
        Some(track) => {
            let meta = track_to_meta(&track);
            let title = meta.title.clone().unwrap_or("Missing Title".to_string());
            let url = meta
                .source_url
                .clone()
                .unwrap_or("Missing Source Url".to_string());
            create_response(&ctx.http, interaction, ParrotMessage::SkipTo { title, url }).await
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

pub async fn force_skip_top_track(queue: &TrackQueue) -> Result<Vec<TrackHandle>, ParrotError> {
    // this is an odd sequence of commands to ensure the queue is properly updated
    // apparently, skipping/stopping a track takes a while to remove it from the queue
    // also, manually removing tracks doesn't trigger the next track to play
    // so first, stop the top song, manually remove it and then resume playback
    queue.current().unwrap().stop().ok();
    let _ = queue.dequeue(0);
    queue.resume().ok();

    Ok(queue.current_queue())
}
