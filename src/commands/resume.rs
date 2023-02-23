use crate::{
    errors::{verify, ParrotError},
    messaging::message::ParrotMessage,
    utils::{create_response, queue::get_queue},
};
use serenity::{all::CommandInteraction, client::Context};

pub async fn resume(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let queue = get_queue(ctx, guild_id).await;

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;
    verify(queue.resume(), ParrotError::Other("Failed resuming track"))?;

    create_response(&ctx.http, interaction, ParrotMessage::Resume).await
}
