use crate::{
    commands::{
        autopause::*, clear::*, leave::*, manage_sources::*, now_playing::*, pause::*, play::*,
        queue::*, remove::*, repeat::*, resume::*, seek::*, shuffle::*, skip::*, stop::*,
        summon::*, version::*, voteskip::*,
    },
    connection::{Connection, check_voice_connections},
    errors::ParrotError,
    global_settings::{GlobalSettings, GlobalSettingsMap},
    guild::settings::{GuildSettings, GuildSettingsMap},
    sources::librespot::{login, refresh_session, set_refresh_token},
    utils::create_response_text,
};
use serenity::{
    all::{
        ActivityData, Command, CommandInteraction, CommandOptionType, CreateCommand,
        CreateCommandOption, EditMember, Interaction,
    },
    async_trait,
    client::{Context, EventHandler},
    model::{gateway::Ready, id::GuildId, prelude::VoiceState},
    prelude::Mentionable,
};
use tracing::{Instrument, error, info, info_span};

use super::track_end::update_queue_messages;

pub struct SerenityHandler;

#[async_trait]
impl EventHandler for SerenityHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("🦜 {} is connected!", ready.user.name);

        // sets parrot activity status message to /play
        let activity = ActivityData::listening("/play");
        ctx.set_activity(Some(activity));

        // loads serialized guild settings
        self.load_guilds_settings(&ctx, &ready).await;
        info!("Loading Settings!");
        self.load_global_settings(&ctx).await;

        // creates the global application commands
        info!("Creating Commands");
        for guild in ready.guilds.iter() {
            info!("Creating commands for guild: {0}", guild.id);
            self.create_commands(&ctx, guild.id).await;
        }

        let data = ctx.data.read().await;
        let global = data.get::<GlobalSettingsMap>().unwrap();

        info!("Got global: {:?}", global);
        if let Some(settings) = &global.spotify {
            info!("Found saved refresh token, reauthing...");
            set_refresh_token(&settings.spotify_refresh_token);

            match refresh_session().await {
                Ok(_) => info!("Spotify reauth success!"),
                Err(e) => error!("Failed to auth spotify: {e:?}"),
            }
        }

        info!("Load guild settings");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(mut command) = interaction else {
            return;
        };

        if let Err(err) = self.run_command(&ctx, &mut command).await {
            error!("Got serenity error: {err:?}");
            self.handle_error(&ctx, &mut command, err).await
        }
    }

    async fn voice_state_update(&self, ctx: Context, _old: Option<VoiceState>, new: VoiceState) {
        // do nothing if this is a voice update event for a user, not a bot
        if new.user_id != ctx.cache.current_user().id {
            return;
        }

        if new.channel_id.is_some() {
            return self.self_deafen(&ctx, new.guild_id, new).await;
        }

        let Some(guild_id) = new.guild_id else {
            return;
        };

        let manager = songbird::get(&ctx).await.unwrap();

        if manager.get(guild_id).is_some() {
            manager.remove(guild_id).await.ok();
        }

        update_queue_messages(&ctx.http, &ctx.data, &[], guild_id).await;
    }
}

impl SerenityHandler {
    async fn create_commands(&self, ctx: &Context, guild: GuildId) -> Vec<Command> {
        let x = vec![
            CreateCommand::new("autopause")
                .description("Toggles whether to pause after a song ends"),
            CreateCommand::new("clear").description("Clears the queue"),
            CreateCommand::new("leave")
                .description("Leave the voice channel the bot is connected to"),
            CreateCommand::new("managesources")
                .description("Manage streaming from different sources"),
            CreateCommand::new("np").description("Displays information about the current track"),
            CreateCommand::new("pause").description("Pauses the current track"),
            CreateCommand::new("play")
                .description("Add a track to the queue")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "The media to play",
                    )
                    .required(true),
                ),
            CreateCommand::new("superplay")
                .description("Add a track to the queue in a special way")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "next",
                        "Add a track to be played up next",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "query",
                            "The media to play",
                        )
                        .required(true),
                    ),
                )
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "jump",
                        "Instantly plays a track, skipping the current one",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "query",
                            "The media to play",
                        )
                        .required(true),
                    ),
                )
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "all",
                        "Add all tracks if the URL refers to a video and a playlist",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "query",
                            "The media to play",
                        )
                        .required(true),
                    ),
                )
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "reverse",
                        "Add a playlist to the queue in reverse order",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "query",
                            "The media to play",
                        )
                        .required(true),
                    ),
                )
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "shuffle",
                        "Add a playlist to the queue in random order",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "query",
                            "The media to play",
                        )
                        .required(true),
                    ),
                ),
            CreateCommand::new("queue").description("Shows the queue"),
            CreateCommand::new("remove")
                .description("Removes a track from the queue")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "index",
                        "Position of the track in the queue (1 is the next track to be played)",
                    )
                    .required(true)
                    .min_int_value(1),
                )
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "until",
                        "Upper range track position to remove a range of tracks",
                    )
                    .required(false)
                    .min_int_value(1),
                ),
            CreateCommand::new("repeat").description("Toggles looping for the current track"),
            CreateCommand::new("resume").description("Resumes the current track"),
            CreateCommand::new("seek")
                .description("Seeks current track to the given position")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "timestamp",
                        "Timestamp in the format HH:MM:SS",
                    )
                    .required(true),
                ),
            CreateCommand::new("shuffle").description("Shuffles the queue"),
            CreateCommand::new("skip")
                .description("Skips the current track")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "to",
                        "Track index to skip to",
                    )
                    .required(false)
                    .min_int_value(1),
                ),
            CreateCommand::new("stop").description("Stops the bot and clears the queue"),
            CreateCommand::new("summon").description("Summons the bot in your voice channel"),
            CreateCommand::new("version").description("Displays the current version"),
            CreateCommand::new("voteskip").description("Starts a vote to skip the current track"),
            CreateCommand::new("login")
                .description("Initiate flow to authorise spotify")
                .add_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "redirect_url",
                        "URL to redirect to for auth flow",
                    )
                    .required(true),
                ),
        ];

        guild
            .set_commands(&ctx.http, x)
            .await
            .unwrap_or_else(|err| {
                error!("Failed to create commands for guild {guild}: {err:?}");
                Vec::new()
            })
    }

    async fn load_global_settings(&self, ctx: &Context) {
        let mut data = ctx.data.write().await;

        let settings = data.get_mut::<GlobalSettingsMap>().unwrap();

        if let Err(err) = settings.load_if_exists() {
            error!("Failed to load global settings due to {:?}", err);
        } else {
            info!(
                "Successfully loaded the settings! {}",
                GlobalSettings::path()
            );
        }
    }
    async fn load_guilds_settings(&self, ctx: &Context, ready: &Ready) {
        info!("Loading guilds' settings");
        let mut data = ctx.data.write().await;

        for guild in &ready.guilds {
            info!("Loading guild settings for {:?}", guild);
            let settings = data.get_mut::<GuildSettingsMap>().unwrap();

            let guild_settings = settings
                .entry(guild.id)
                .or_insert_with(|| GuildSettings::new(guild.id));

            if let Err(err) = guild_settings.load_if_exists() {
                error!(
                    "[ERROR] Failed to load guild {} settings due to {}",
                    guild.id, err
                );
            }
        }
    }

    async fn run_command(
        &self,
        ctx: &Context,
        command: &mut CommandInteraction,
    ) -> Result<(), ParrotError> {
        let command_name = command.data.name.clone();
        let guild_id = command.guild_id.ok_or(ParrotError::Other(
            "This command can only be used in a server",
        ))?;

        // get songbird voice client
        let manager = songbird::get(ctx).await.unwrap();

        // parrot might have been disconnected manually
        if let Some(call) = manager.get(guild_id) {
            let mut handler = call.lock().await;
            if handler.current_connection().is_none() {
                handler.leave().await.unwrap();
            }
        }

        // fetch the user and the bot's user IDs
        let user_id = command.user.id;
        let bot_id = ctx.cache.current_user().id;
        {
            let Some(guild) = ctx.cache.guild(guild_id) else {
                return Err(ParrotError::Other("Could not fetch this server's data"));
            };

            match command_name.as_str() {
                "autopause" | "clear" | "leave" | "pause" | "remove" | "repeat" | "resume"
                | "seek" | "shuffle" | "skip" | "stop" | "voteskip" => {
                    match check_voice_connections(&guild, &user_id, &bot_id) {
                        Connection::User(_) | Connection::Neither => Err(ParrotError::NotConnected),
                        Connection::Bot(bot_channel_id) => {
                            Err(ParrotError::AuthorDisconnected(bot_channel_id.mention()))
                        }
                        Connection::Separate(_, _) => Err(ParrotError::WrongVoiceChannel),
                        _ => Ok(()),
                    }
                }
                "play" | "superplay" | "summon" => {
                    match check_voice_connections(&guild, &user_id, &bot_id) {
                        Connection::User(_) => Ok(()),
                        Connection::Bot(_) if command_name == "summon" => {
                            Err(ParrotError::AuthorNotFound)
                        }
                        Connection::Bot(_) if command_name != "summon" => {
                            Err(ParrotError::WrongVoiceChannel)
                        }
                        Connection::Separate(bot_channel_id, _) => {
                            Err(ParrotError::AlreadyConnected(bot_channel_id.mention()))
                        }
                        Connection::Neither => Err(ParrotError::AuthorNotFound),
                        _ => Ok(()),
                    }
                }
                "np" | "queue" => match check_voice_connections(&guild, &user_id, &bot_id) {
                    Connection::User(_) | Connection::Neither => Err(ParrotError::NotConnected),
                    _ => Ok(()),
                },
                _ => Ok(()),
            }?;
        }
        info!("Running command: {command_name}");
        async move {
            let res = match command_name.as_str() {
                "autopause" => autopause(ctx, command).await,
                "clear" => clear(ctx, command).await,
                "leave" => leave(ctx, command).await,
                "managesources" => allow(ctx, command).await,
                "np" => now_playing(ctx, command).await,
                "pause" => pause(ctx, command).await,
                "play" | "superplay" => play(ctx, command).await,
                "queue" => queue(ctx, command).await,
                "remove" => remove(ctx, command).await,
                "repeat" => repeat(ctx, command).await,
                "resume" => resume(ctx, command).await,
                "seek" => seek(ctx, command).await,
                "shuffle" => shuffle(ctx, command).await,
                "skip" => skip(ctx, command).await,
                "stop" => stop(ctx, command).await,
                "summon" => summon(ctx, command, true).await,
                "version" => version(ctx, command).await,
                "voteskip" => voteskip(ctx, command).await,
                "login" => login(ctx, command).await,
                _ => unreachable!(),
            };
            info!("Finished Command {}", &command_name);
            res
        }
        .instrument(info_span!("Running Command"))
        .await
    }

    async fn self_deafen(&self, ctx: &Context, guild: Option<GuildId>, new: VoiceState) {
        let current_user_id = ctx.cache.current_user().id;

        if current_user_id == new.user_id && !new.deaf {
            let Some(guild) = guild else {
                return;
            };

            if let Err(err) = guild
                .edit_member(&ctx.http, new.user_id, EditMember::new().deafen(true))
                .await
            {
                error!("Failed to self-deafen: {err:?}");
            }
        }
    }

    async fn handle_error(
        &self,
        ctx: &Context,
        interaction: &mut CommandInteraction,
        err: ParrotError,
    ) {
        error!("Error in serenity: {:?}", err);
        create_response_text(&ctx.http, interaction, &format!("{err}"))
            .await
            .expect("failed to create response");
    }
}
