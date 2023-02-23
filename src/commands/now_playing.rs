use crate::{
    errors::ParrotError,
    utils::{create_embed_response, create_now_playing_embed, queue::get_queue},
};
use log::info;
use serenity::{all::CommandInteraction, client::Context};

pub async fn now_playing(
    ctx: &Context,
    interaction: &mut CommandInteraction,
) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let track = get_queue(ctx, guild_id)
        .await
        .current()
        .ok_or(ParrotError::NothingPlaying)?;

    info!("Got track: {:?}", track);

    let embed = create_now_playing_embed(&track).await;
    create_embed_response(&ctx.http, interaction, embed).await
}
