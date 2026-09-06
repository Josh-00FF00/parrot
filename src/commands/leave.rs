use crate::{
    errors::ParrotError,
    messaging::message::ParrotMessage,
    utils::{command_guild_id, create_response},
};
use serenity::{all::CommandInteraction, client::Context};
use tracing::instrument;

#[instrument(level = "info", skip_all)]
pub async fn leave(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = command_guild_id(interaction)?;
    let manager = songbird::get(ctx).await.unwrap();
    manager
        .remove(guild_id)
        .await
        .map_err(|e| ParrotError::OtherS(format!("Failed to leave the voice channel: {e:?}")))?;

    create_response(&ctx.http, interaction, ParrotMessage::Leaving).await
}
