use crate::{
    errors::ParrotError,
    messaging::{message::ParrotMessage, messages::FAIL_SECONDS_PARSING},
    utils::{command_guild_id, create_response, parse_timestamp},
};
use serenity::{all::CommandInteraction, client::Context};
use std::time::Duration;
use tracing::instrument;

#[instrument(level = "info", skip_all)]
pub async fn seek(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let args = interaction.data.options.clone();
    let seek_time = args
        .first()
        .and_then(|a| a.value.as_str())
        .ok_or(ParrotError::Other("Missing arg"))?;

    let timestamp = parse_timestamp(seek_time).ok_or(ParrotError::Other(FAIL_SECONDS_PARSING))?;

    let guild_id = command_guild_id(interaction)?;
    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).ok_or(ParrotError::NotConnected)?;

    let handler = call.lock().await;
    let track = handler.queue().current().ok_or(ParrotError::QueueEmpty)?;

    let _ = track.seek(Duration::from_secs(timestamp));

    create_response(
        &ctx.http,
        interaction,
        ParrotMessage::Seek {
            timestamp: seek_time.to_owned(),
        },
    )
    .await
}
