use crate::{
    errors::{verify, ParrotError},
    handlers::track_end::update_queue_messages,
    messaging::{message::ParrotMessage, messages::REMOVED_QUEUE},
    utils::{
        create_embed_response, create_response,
        queue::{get_queue, Queued},
    },
};
use serenity::{all::CommandInteraction, builder::CreateEmbed, client::Context};
use std::cmp::min;

pub async fn remove(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();

    let args = interaction.data.options.clone();
    let remove_index = args.first().and_then(|a| a.value.as_i64()).unwrap() as usize;
    let remove_until = args
        .get(1)
        .and_then(|a| a.value.as_i64())
        .unwrap_or(remove_index as i64) as usize;

    let queue = get_queue(ctx, guild_id).await;

    let queue_len = queue.len();
    let remove_until = min(remove_until, queue_len.saturating_sub(1));

    verify(queue_len > 1, ParrotError::QueueEmpty)?;
    verify(
        remove_index < queue_len,
        ParrotError::NotInRange("index", remove_index as isize, 1, queue_len as isize),
    )?;
    verify(
        remove_until >= remove_index,
        ParrotError::NotInRange(
            "until",
            remove_until as isize,
            remove_index as isize,
            queue_len as isize,
        ),
    )?;

    let track = queue.current_queue().get(remove_index).cloned().unwrap();

    queue.modify_queue(|v| {
        v.drain(remove_index..=remove_until);
    });

    if remove_until == remove_index {
        let embed = create_remove_enqueued_embed(&track).await;
        create_embed_response(&ctx.http, interaction, embed).await?;
    } else {
        create_response(&ctx.http, interaction, ParrotMessage::RemoveMultiple).await?;
    }

    update_queue_messages(&ctx.http, &ctx.data, &queue.current_queue(), guild_id).await;
    Ok(())
}

async fn create_remove_enqueued_embed(track: &Queued) -> CreateEmbed {
    let metadata = &track.1;
    CreateEmbed::default()
        .field(
            REMOVED_QUEUE,
            format!(
                "[**{}**]({})",
                metadata.title.as_ref().cloned().unwrap(),
                metadata.source_url.as_ref().cloned().unwrap()
            ),
            false,
        )
        .thumbnail(metadata.thumbnail.as_ref().cloned().unwrap())
}
