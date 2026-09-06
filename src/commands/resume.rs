use crate::{
    errors::{ParrotError, verify},
    messaging::message::ParrotMessage,
    utils::{command_guild_id, create_response},
};
use serenity::{all::CommandInteraction, client::Context};
use tracing::instrument;

#[instrument(level = "info", skip_all)]
pub async fn resume(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = command_guild_id(interaction)?;
    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).unwrap();

    let handler = call.lock().await;
    let queue = handler.queue();

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;
    verify(queue.resume(), ParrotError::Other("Failed resuming track"))?;

    create_response(&ctx.http, interaction, ParrotMessage::Resume).await
}
