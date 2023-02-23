use crate::{
    errors::{verify, ParrotError},
    handlers::track_end::update_queue_messages,
    messaging::message::ParrotMessage,
    utils::{create_response, queue::get_queue},
};
use serenity::{all::CommandInteraction, client::Context};

pub async fn clear(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let queue = get_queue(ctx, guild_id).await;

    verify(queue.len() > 1, ParrotError::QueueEmpty)?;

    queue.modify_queue(|v| {
        v.drain(1..);
    });

    create_response(&ctx.http, interaction, ParrotMessage::Clear).await?;
    update_queue_messages(&ctx.http, &ctx.data, &queue.current_queue(), guild_id).await;
    Ok(())
}
