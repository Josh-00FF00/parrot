use crate::{
    errors::ParrotError,
    messaging::{message::ParrotMessage, messages::FAIL_LOOP},
    utils::{create_response, queue::get_queue},
};
use serenity::{all::CommandInteraction, client::Context};
use songbird::tracks::{LoopState, TrackHandle};

pub async fn repeat(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let track = get_queue(ctx, guild_id).await.current().unwrap();

    let was_looping = track.get_info().await.unwrap().loops == LoopState::Infinite;
    let toggler = if was_looping {
        TrackHandle::disable_loop
    } else {
        TrackHandle::enable_loop
    };

    match toggler(&track) {
        Ok(_) if was_looping => {
            create_response(&ctx.http, interaction, ParrotMessage::LoopDisable).await
        }
        Ok(_) if !was_looping => {
            create_response(&ctx.http, interaction, ParrotMessage::LoopEnable).await
        }
        _ => Err(ParrotError::Other(FAIL_LOOP)),
    }
}
