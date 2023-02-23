use crate::{
    errors::{verify, ParrotError},
    handlers::track_end::update_queue_messages,
    messaging::message::ParrotMessage,
    utils::{create_response, queue::get_queue},
};
use serenity::{all::CommandInteraction, client::Context};

pub async fn stop(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let queue = get_queue(ctx, guild_id).await;

    verify(!queue.is_empty(), ParrotError::NothingPlaying)?;
    queue.stop();

    // refetch the queue after modification
    let queue = get_queue(ctx, guild_id).await;

    create_response(&ctx.http, interaction, ParrotMessage::Stop).await?;
    update_queue_messages(&ctx.http, &ctx.data, &queue.current_queue(), guild_id).await;
    Ok(())
}
