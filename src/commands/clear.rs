use crate::{
    errors::{verify, ParrotError},
    handlers::track_end::update_queue_messages,
    messaging::message::ParrotMessage,
    utils::create_response,
};
use serenity::all::{CommandInteraction, Context};
use tracing::instrument;

#[instrument(level = "info", skip_all)]
pub async fn clear(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).unwrap();
    let handler = call.lock().await;
    let queue = handler.queue();

    verify(queue.len() > 1, ParrotError::QueueEmpty)?;

    queue.modify_queue(|v| {
        v.drain(1..);
    });

    create_response(&ctx.http, interaction, ParrotMessage::Clear).await?;
    update_queue_messages(&ctx.http, &ctx.data, &queue.current_queue(), guild_id).await;
    Ok(())
}
