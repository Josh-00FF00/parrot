use crate::{
    commands::{skip::force_skip_top_track, summon::summon},
    errors::{ParrotError, verify},
    guild::settings::{GuildSettings, GuildSettingsMap},
    handlers::track_end::update_queue_messages,
    messaging::{
        message::ParrotMessage,
        messages::{PLAY_QUEUE, PLAY_TOP, TRACK_DURATION, TRACK_TIME_TO_PLAY},
    },
    sources::{
        librespot::{Respot, SpotifyError},
        youtube::{YouTube, YouTubeTrack},
    },
    utils::{
        command_guild_id, compare_domains, create_now_playing_embed, create_response,
        edit_embed_response, edit_response, get_human_readable_timestamp, track_to_meta,
        yt_dlp_cookie_args,
    },
};
use librespot::core::SpotifyUri;
use serenity::{
    all::{CommandInteraction, CreateEmbedFooter},
    builder::CreateEmbed,
    client::Context,
};
use songbird::{
    Call,
    input::{Input, YoutubeDl},
    tracks::{Track, TrackHandle, TrackQueue},
};
use std::{cmp::Ordering, error::Error as StdError, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tracing::{info, instrument};
use url::Url;

#[derive(Clone, Copy)]
pub enum Mode {
    End,
    Next,
    All,
    Reverse,
    Shuffle,
    Jump,
}

#[derive(Clone)]
pub enum QueryType {
    Keywords(String),
    VideoLink(String),
    VideoMeta {
        url: String,
        title: Option<String>,
        duration: Option<f64>,
    },
    PlaylistLink(String),
    SpotifyUri(SpotifyUri),
}

#[instrument(level = "info", skip_all)]
pub async fn play(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let args = interaction.data.options.clone();
    let first_arg = args.first().ok_or(ParrotError::Other("Missing arg"))?;

    info!("Got play args: {:#?}", args);

    let mode = match first_arg.name.as_str() {
        "next" => Mode::Next,
        "all" => Mode::All,
        "reverse" => Mode::Reverse,
        "shuffle" => Mode::Shuffle,
        "jump" => Mode::Jump,
        _ => Mode::End,
    };

    let url = first_arg
        .value
        .as_str()
        .ok_or(ParrotError::Other("Missing arg"))?;

    let guild_id = command_guild_id(interaction)?;

    // try to join a voice channel if not in one just yet
    summon(ctx, interaction, false).await?;
    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).ok_or(ParrotError::NotConnected)?;

    // determine whether this is a link or a query string
    let query_type = match Url::parse(url) {
        Ok(url_data) => match url_data.host_str() {
            Some("open.spotify.com") => {
                let uri = Respot::from_share_link(url)?;
                Some(Respot::extract(uri))
            }
            Some(other) => {
                let mut data = ctx.data.write().await;
                let settings = data.get_mut::<GuildSettingsMap>().unwrap();
                let guild_settings = settings
                    .entry(guild_id)
                    .or_insert_with(|| GuildSettings::new(guild_id));

                let is_allowed = guild_settings
                    .allowed_domains
                    .iter()
                    .any(|d| compare_domains(d, other));

                let is_banned = guild_settings
                    .banned_domains
                    .iter()
                    .any(|d| compare_domains(d, other));

                if is_banned || (guild_settings.banned_domains.is_empty() && !is_allowed) {
                    return create_response(
                        &ctx.http,
                        interaction,
                        ParrotMessage::PlayDomainBanned {
                            domain: other.to_string(),
                        },
                    )
                    .await;
                }

                YouTube::extract(url)
            }
            None => None,
        },
        Err(_) => {
            let mut data = ctx.data.write().await;
            let settings = data.get_mut::<GuildSettingsMap>().unwrap();
            let guild_settings = settings
                .entry(guild_id)
                .or_insert_with(|| GuildSettings::new(guild_id));

            if guild_settings.banned_domains.contains("youtube.com")
                || (guild_settings.banned_domains.is_empty()
                    && !guild_settings.allowed_domains.contains("youtube.com"))
            {
                return create_response(
                    &ctx.http,
                    interaction,
                    ParrotMessage::PlayDomainBanned {
                        domain: "youtube.com".to_string(),
                    },
                )
                .await;
            }

            Some(QueryType::Keywords(url.to_string()))
        }
    };

    info!("Handling cmd: {}", url);

    let query_type = verify(
        query_type,
        ParrotError::Other("Something went wrong while parsing your query!"),
    )?;

    // reply with a temporary message while we fetch the source
    // needed because interactions must be replied within 3s and queueing takes longer
    create_response(&ctx.http, interaction, ParrotMessage::Search).await?;

    info!("Replied to play req");

    let queue_was_empty = call.lock().await.queue().is_empty();

    match mode {
        Mode::End => match query_type.clone() {
            QueryType::Keywords(_) | QueryType::VideoLink(_) => {
                let queue = enqueue_track(&call, &query_type).await?;
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            QueryType::PlaylistLink(url) => {
                let entries = YouTube::ytdl_playlist(&url, mode)
                    .await
                    .ok_or(ParrotError::Other("failed to fetch playlist"))?;

                for entry in entries {
                    let Ok(_) = enqueue_track(&call, &QueryType::from(entry)).await else {
                        continue;
                    };
                }

                let queue = call.lock().await.queue().current_queue();
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            QueryType::SpotifyUri(spotify_uri) => match spotify_uri {
                SpotifyUri::Track { id: _ } => {
                    let queue = enqueue_track(&call, &query_type).await?;
                    update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
                }
                SpotifyUri::Album { id: _ } => {
                    return Err(ParrotError::Spotify(SpotifyError::Todo));
                }
                SpotifyUri::Playlist { user: _, id: _ } => {
                    return Err(ParrotError::Spotify(SpotifyError::Todo));
                }
                _ => return Err(ParrotError::Spotify(SpotifyError::Todo)),
            },
            _ => return Ok(()),
        },
        Mode::Next => match query_type.clone() {
            QueryType::Keywords(_)
            | QueryType::VideoLink(_)
            | QueryType::SpotifyUri(SpotifyUri::Track { id: _ }) => {
                let queue = insert_track(&call, &query_type, 1).await?;
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            QueryType::PlaylistLink(url) => {
                let entries = YouTube::ytdl_playlist(&url, mode)
                    .await
                    .ok_or(ParrotError::Other("failed to fetch playlist"))?;

                for (idx, entry) in entries.into_iter().enumerate() {
                    let Ok(_) = insert_track(&call, &QueryType::from(entry), idx + 1).await else {
                        continue;
                    };
                }

                let queue = call.lock().await.queue().current_queue();
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            QueryType::SpotifyUri(_) => return Err(ParrotError::Spotify(SpotifyError::Todo)),
            _ => return Ok(()),
        },
        Mode::Jump => match query_type.clone() {
            QueryType::Keywords(_)
            | QueryType::VideoLink(_)
            | QueryType::SpotifyUri(SpotifyUri::Track { id: _ }) => {
                let mut queued = enqueue_track(&call, &query_type).await?;

                if !queue_was_empty {
                    let handler = call.lock().await;
                    rotate_tracks(handler.queue(), 1).await.ok();
                    queued = force_skip_top_track(handler.queue()).await?;
                }

                update_queue_messages(&ctx.http, &ctx.data, &queued, guild_id).await;
            }
            QueryType::PlaylistLink(url) => {
                let entries = YouTube::ytdl_playlist(&url, mode)
                    .await
                    .ok_or(ParrotError::Other("failed to fetch playlist"))?;

                let mut insert_idx = 1;

                for (i, entry) in entries.into_iter().enumerate() {
                    let Ok(_) = insert_track(&call, &QueryType::from(entry), insert_idx).await
                    else {
                        continue;
                    };

                    if i == 0 && !queue_was_empty {
                        let handler = call.lock().await;
                        force_skip_top_track(handler.queue()).await?;
                    } else {
                        insert_idx += 1;
                    }
                }

                let queue = call.lock().await.queue().current_queue();
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            QueryType::SpotifyUri(_) => return Err(ParrotError::Spotify(SpotifyError::Todo)),
            _ => return Ok(()),
        },
        Mode::All | Mode::Reverse | Mode::Shuffle => match query_type.clone() {
            QueryType::VideoLink(url) | QueryType::PlaylistLink(url) => {
                let entries = YouTube::ytdl_playlist(&url, mode)
                    .await
                    .ok_or(ParrotError::Other("failed to fetch playlist"))?;

                for entry in entries {
                    let Ok(_) = enqueue_track(&call, &QueryType::from(entry)).await else {
                        continue;
                    };
                }

                let queue = call.lock().await.queue().current_queue();
                update_queue_messages(&ctx.http, &ctx.data, &queue, guild_id).await;
            }
            _ => {
                edit_response(&ctx.http, interaction, ParrotMessage::PlayAllFailed).await?;
                return Ok(());
            }
        },
    }

    // refetch the queue after modification
    let snapshot = call.lock().await.queue().current_queue();

    match snapshot.len().cmp(&1) {
        Ordering::Greater => {
            if let Some(estimated_time) = calculate_time_until_play(&snapshot, mode).await {
                match (query_type, mode) {
                    (
                        QueryType::SpotifyUri(_)
                        | QueryType::VideoLink(_)
                        | QueryType::VideoMeta { .. }
                        | QueryType::Keywords(_),
                        Mode::Next,
                    ) => {
                        let Some(track) = snapshot.get(1) else {
                            return Ok(());
                        };
                        let embed = create_queued_embed(PLAY_TOP, track, estimated_time).await;

                        edit_embed_response(&ctx.http, interaction, embed).await?;
                    }
                    (
                        QueryType::SpotifyUri(_)
                        | QueryType::VideoLink(_)
                        | QueryType::VideoMeta { .. }
                        | QueryType::Keywords(_),
                        Mode::End,
                    ) => {
                        let Some(track) = snapshot.last() else {
                            return Ok(());
                        };
                        let embed = create_queued_embed(PLAY_QUEUE, track, estimated_time).await;

                        edit_embed_response(&ctx.http, interaction, embed).await?;
                    }
                    (QueryType::PlaylistLink(_), _) => {
                        edit_response(&ctx.http, interaction, ParrotMessage::PlaylistQueued)
                            .await?;
                    }
                    (_, _) => {}
                }
            }
        }
        Ordering::Equal => {
            let Some(track) = snapshot.first() else {
                return Ok(());
            };
            info!("Got track: {:?}", track);
            let embed = create_now_playing_embed(track).await;

            edit_embed_response(&ctx.http, interaction, embed).await?;
        }
        _ => return Ok(()),
    }

    Ok(())
}

async fn calculate_time_until_play(queue: &[TrackHandle], mode: Mode) -> Option<Duration> {
    if queue.is_empty() {
        return None;
    }

    let top_track = queue.first()?;
    let top_track_elapsed = top_track.get_info().await.ok()?.position;

    let top_track_duration = match track_to_meta(top_track).duration {
        Some(duration) => duration,
        None => return Some(Duration::MAX),
    };

    match mode {
        Mode::Next => Some(top_track_duration.saturating_sub(top_track_elapsed)),
        _ => {
            let center = &queue[1..queue.len() - 1];
            let livestreams = center.len()
                - center
                    .iter()
                    .filter_map(|t| track_to_meta(t).duration)
                    .count();

            // if any of the tracks before are livestreams, the new track will never play
            if livestreams > 0 {
                return Some(Duration::MAX);
            }

            let durations = center.iter().fold(Duration::ZERO, |acc, track| {
                acc + track_to_meta(track).duration.unwrap_or_default()
            });

            Some(durations + top_track_duration.saturating_sub(top_track_elapsed))
        }
    }
}

async fn create_queued_embed(
    title: &str,
    track: &TrackHandle,
    estimated_time: Duration,
) -> CreateEmbed {
    let metadata = track_to_meta(track);
    let footer_text = format!(
        "{}{}\n{}{}",
        TRACK_DURATION,
        get_human_readable_timestamp(metadata.duration),
        TRACK_TIME_TO_PLAY,
        get_human_readable_timestamp(Some(estimated_time))
    );

    CreateEmbed::default()
        // .thumbnail(metadata.thumbnail.unwrap())
        .field(
            title,
            format!(
                "[**{}**]({})",
                metadata
                    .title
                    .clone()
                    .unwrap_or("Missing title".to_string()),
                metadata
                    .source_url
                    .clone()
                    .unwrap_or("Missing url".to_string())
            ),
            false,
        )
        .footer(CreateEmbedFooter::new(footer_text))
}

async fn get_track_source(query_type: QueryType) -> Result<Input, ParrotError> {
    // Fine to clone since the client uses an Arc internally
    let client = crate::utils::get_reqwest_client().clone();

    Ok(match query_type {
        QueryType::VideoLink(query) => YoutubeDl::new(client, query)
            .user_args(yt_dlp_cookie_args())
            .into(),
        QueryType::VideoMeta {
            url,
            title,
            duration,
        } => YouTubeTrack::new(url, title, duration).into(),
        QueryType::Keywords(search) => YoutubeDl::new_search(client, search)
            .user_args(yt_dlp_cookie_args())
            .into(),
        QueryType::SpotifyUri(id) => Respot::new_track(id).await,
        QueryType::PlaylistLink(_) => {
            return Err(ParrotError::Other("cannot play a playlist directly"));
        }
    })
}

async fn make_track(query_type: &QueryType) -> Result<Track, ParrotError> {
    let (source, meta) = match query_type {
        QueryType::VideoMeta {
            url,
            title,
            duration,
        } => {
            let track = YouTubeTrack::new(url.clone(), title.clone(), *duration);
            let meta = track.metadata();
            (track.into(), meta)
        }
        _ => {
            let mut source = get_track_source(query_type.clone()).await?;
            let meta = source
                .aux_metadata()
                .await
                .map_err(|e| ParrotError::OtherS(format!("Failed to read aux_metadata() {e:?}")))?;
            (source, meta)
        }
    };

    let mut track: Track = source.into();
    track.user_data = Arc::new(meta);

    Ok(track.volume(0.10))
}

async fn enqueue_track(
    call: &Arc<Mutex<Call>>,
    query_type: &QueryType,
) -> Result<Vec<TrackHandle>, ParrotError> {
    let track = make_track(query_type).await?;

    let mut handler = call.lock().await;
    handler.enqueue(track).await;

    Ok(handler.queue().current_queue())
}

async fn insert_track(
    call: &Arc<Mutex<Call>>,
    query_type: &QueryType,
    idx: usize,
) -> Result<Vec<TrackHandle>, ParrotError> {
    let queue_size = call.lock().await.queue().current_queue().len();

    if queue_size <= 1 {
        return enqueue_track(call, query_type).await;
    }

    verify(
        idx > 0 && idx <= queue_size,
        ParrotError::NotInRange("index", idx as isize, 1, queue_size as isize),
    )?;

    let track = make_track(query_type).await?;

    let mut handler = call.lock().await;
    handler.enqueue(track).await;

    handler.queue().modify_queue(|queue| {
        if let Some(back) = queue.pop_back() {
            queue.insert(idx.min(queue.len()), back);
        }
    });

    Ok(handler.queue().current_queue())
}

async fn rotate_tracks(
    queue: &TrackQueue,
    n: usize,
) -> Result<Vec<TrackHandle>, Box<dyn StdError>> {
    verify(
        queue.len() > 2,
        ParrotError::Other("cannot rotate queues smaller than 3 tracks"),
    )?;

    queue.modify_queue(|queue| {
        let mut not_playing = queue.split_off(1);
        not_playing.rotate_right(n);
        queue.append(&mut not_playing);
    });

    Ok(queue.current_queue())
}
