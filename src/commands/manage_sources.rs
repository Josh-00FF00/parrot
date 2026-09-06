use crate::{
    errors::ParrotError,
    guild::settings::{GuildSettings, GuildSettingsMap},
    messaging::messages::{
        DOMAIN_FORM_ALLOWED_PLACEHOLDER, DOMAIN_FORM_ALLOWED_TITLE, DOMAIN_FORM_BANNED_PLACEHOLDER,
        DOMAIN_FORM_BANNED_TITLE, DOMAIN_FORM_TITLE,
    },
    utils::command_guild_id,
};
use serenity::{
    all::{
        ActionRowComponent, CommandInteraction, CreateActionRow, CreateInteractionResponse,
        CreateInteractionResponseMessage, CreateModal, InputTextStyle,
    },
    builder::CreateInputText,
    client::Context,
    collector::ModalInteractionCollector,
    futures::StreamExt,
};
use tracing::{error, instrument};

#[instrument(level = "info", skip_all)]
pub async fn allow(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = command_guild_id(interaction)?;

    let mut data = ctx.data.write().await;
    let settings = data.get_mut::<GuildSettingsMap>().unwrap();

    let guild_settings = settings
        .entry(guild_id)
        .or_insert_with(|| GuildSettings::new(guild_id));

    // transform the domain sets from the settings into a string
    let allowed_str = guild_settings
        .allowed_domains
        .clone()
        .into_iter()
        .collect::<Vec<String>>()
        .join(";");

    let banned_str = guild_settings
        .banned_domains
        .clone()
        .into_iter()
        .collect::<Vec<String>>()
        .join(";");

    drop(data);

    let allowed_input = CreateInputText::new(
        InputTextStyle::Paragraph,
        DOMAIN_FORM_ALLOWED_TITLE,
        "allowed_domains",
    )
    .placeholder(DOMAIN_FORM_ALLOWED_PLACEHOLDER)
    .value(allowed_str)
    .required(false);

    let banned_input = CreateInputText::new(
        InputTextStyle::Paragraph,
        DOMAIN_FORM_BANNED_TITLE,
        "banned_domains",
    )
    .placeholder(DOMAIN_FORM_BANNED_PLACEHOLDER)
    .value(banned_str)
    .required(false);

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new("manage_domains", DOMAIN_FORM_TITLE).components(vec![
                    CreateActionRow::InputText(allowed_input),
                    CreateActionRow::InputText(banned_input),
                ]),
            ),
        )
        .await?;

    // collect the submitted data
    let collector = ModalInteractionCollector::new(ctx)
        .filter(|int| int.data.custom_id == "manage_domains")
        .stream();

    collector
        .then(|int| async move {
            let mut data = ctx.data.write().await;
            let Some(settings) = data.get_mut::<GuildSettingsMap>() else {
                return;
            };

            let Some(guild_settings) = settings.get_mut(&guild_id) else {
                return;
            };

            let inputs: Vec<_> = int
                .data
                .components
                .iter()
                .flat_map(|r| r.components.iter())
                .collect();

            for input in inputs.iter() {
                if let ActionRowComponent::InputText(it) = input {
                    let value = it.value.clone().unwrap_or_default();

                    if it.custom_id == "allowed_domains" {
                        guild_settings.set_allowed_domains(&value);
                    }

                    if it.custom_id == "banned_domains" {
                        guild_settings.set_banned_domains(&value);
                    }
                }
            }

            guild_settings.update_domains();

            if let Err(err) = guild_settings.save() {
                error!("Failed to save guild settings: {err:?}");
            }

            // it's now safe to close the modal, so send a response to it
            int.create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
            )
            .await
            .ok();
        })
        .collect::<Vec<_>>()
        .await;

    Ok(())
}
