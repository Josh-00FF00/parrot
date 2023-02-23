use crate::{
    errors::ParrotError,
    guild::cache::GuildCacheMap,
    handlers::track_end::ModifyQueueHandler,
    messaging::messages::{
        QUEUE_EXPIRED, QUEUE_NOTHING_IS_PLAYING, QUEUE_NOW_PLAYING, QUEUE_NO_SONGS, QUEUE_PAGE,
        QUEUE_PAGE_OF, QUEUE_UP_NEXT,
    },
    utils::{
        get_human_readable_timestamp,
        queue::{get_queue, Queued},
    },
};
use log::info;
use serenity::{
    all::{
        ButtonStyle, CommandInteraction, CreateActionRow, CreateEmbedFooter,
        CreateInteractionResponse, CreateInteractionResponseMessage, EditMessage,
    },
    builder::{CreateButton, CreateEmbed},
    client::Context,
    futures::StreamExt,
    model::{channel::Message, id::GuildId},
    prelude::{RwLock, TypeMap},
};
use songbird::{Event, TrackEvent};
use std::{
    cmp::{max, min},
    fmt::Write,
    ops::Add,
    sync::Arc,
    time::Duration,
};

const EMBED_PAGE_SIZE: usize = 6;
const EMBED_TIMEOUT: u64 = 3600;

pub async fn queue(ctx: &Context, interaction: &mut CommandInteraction) -> Result<(), ParrotError> {
    let guild_id = interaction.guild_id.unwrap();
    let manager = songbird::get(ctx).await.unwrap();
    let call = manager.get(guild_id).unwrap();

    let queue = get_queue(ctx, guild_id).await;

    info!("Making interaction response");
    let num_pages = calculate_num_pages(&queue.current_queue());

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .add_embed(create_queue_embed(&queue.current_queue(), 0))
                    .components(build_nav_btns(0, num_pages)),
            ),
        )
        .await?;

    info!("Sent interaction response");
    let mut message = interaction.get_response(&ctx.http).await?;
    let page: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
    info!("Got interaction response");

    // store this interaction to context.data for later edits
    let mut data = ctx.data.write().await;
    let cache_map = data.get_mut::<GuildCacheMap>().unwrap();

    let cache = cache_map.entry(guild_id).or_default();
    cache.queue_messages.push((message.clone(), page.clone()));
    drop(data);

    // refresh the queue interaction whenever a track ends
    call.lock().await.add_global_event(
        Event::Track(TrackEvent::End),
        ModifyQueueHandler {
            http: ctx.http.clone(),
            ctx_data: ctx.data.clone(),
            queue,
            guild_id,
        },
    );

    let mut cib = message
        .await_component_interactions(ctx)
        .timeout(Duration::from_secs(EMBED_TIMEOUT))
        .stream();

    while let Some(mci) = cib.next().await {
        let btn_id = &mci.data.custom_id;

        // refetch the queue in case it changed
        let queue = get_queue(ctx, guild_id).await;

        let num_pages = calculate_num_pages(&queue.current_queue());
        let mut page_wlock = page.write().await;

        *page_wlock = match btn_id.as_str() {
            "<<" => 0,
            "<" => min(page_wlock.saturating_sub(1), num_pages - 1),
            ">" => min(page_wlock.add(1), num_pages - 1),
            ">>" => num_pages - 1,
            _ => continue,
        };

        interaction
            .create_response(
                &ctx.http,
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .add_embed(create_queue_embed(&queue.current_queue(), *page_wlock))
                        .components(build_nav_btns(*page_wlock, num_pages)),
                ),
            )
            .await?;
    }

    message
        .edit(
            &ctx.http,
            EditMessage::new().add_embed(CreateEmbed::new().description(QUEUE_EXPIRED)),
        )
        .await
        .unwrap();

    forget_queue_message(&ctx.data, &mut message, guild_id)
        .await
        .ok();

    Ok(())
}

pub fn create_queue_embed(tracks: &[Queued], page: usize) -> CreateEmbed {
    let embed: CreateEmbed =
        CreateEmbed::default().field(QUEUE_UP_NEXT, build_queue_page(tracks, page), false);

    let (embed, description) = if !tracks.is_empty() {
        let metadata = tracks[0].clone().1;
        (
            embed.thumbnail(metadata.thumbnail.as_ref().unwrap()),
            format!(
                "[{}]({}) • `{}`",
                metadata.title.as_ref().unwrap(),
                metadata.source_url.as_ref().unwrap(),
                get_human_readable_timestamp(metadata.duration)
            ),
        )
    } else {
        (embed, String::from(QUEUE_NOTHING_IS_PLAYING))
    };

    embed
        .field(QUEUE_NOW_PLAYING, description, false)
        .footer(CreateEmbedFooter::new(format!(
            "{} {} {} {}",
            QUEUE_PAGE,
            page + 1,
            QUEUE_PAGE_OF,
            calculate_num_pages(tracks),
        )))
}

fn build_single_nav_btn(label: &str, is_disabled: bool) -> CreateButton {
    CreateButton::new(label.to_string().to_ascii_lowercase())
        .label(label)
        .style(ButtonStyle::Primary)
        .disabled(is_disabled)
        .to_owned()
}

pub fn build_nav_btns(page: usize, num_pages: usize) -> Vec<CreateActionRow> {
    let (cant_left, cant_right) = (page < 1, page >= num_pages - 1);

    vec![CreateActionRow::Buttons(vec![
        build_single_nav_btn("<<", cant_left),
        build_single_nav_btn("<", cant_left),
        build_single_nav_btn(">", cant_right),
        build_single_nav_btn(">>", cant_right),
    ])]
}

fn build_queue_page(tracks: &[Queued], page: usize) -> String {
    let start_idx = EMBED_PAGE_SIZE * page;
    let queue: Vec<&Queued> = tracks
        .iter()
        .skip(start_idx + 1)
        .take(EMBED_PAGE_SIZE)
        .collect();

    if queue.is_empty() {
        return String::from(QUEUE_NO_SONGS);
    }

    let mut description = String::new();

    for (i, queued) in queue.iter().enumerate() {
        let metadata = queued.1.clone();
        let title = metadata.title.as_ref().unwrap();
        let url = metadata.source_url.as_ref().unwrap();
        let duration = get_human_readable_timestamp(metadata.duration);

        let _ = writeln!(
            description,
            "`{}.` [{}]({}) • `{}`",
            i + start_idx + 1,
            title,
            url,
            duration
        );
    }

    description
}

pub fn calculate_num_pages<T>(tracks: &[T]) -> usize {
    let num_pages = ((tracks.len() as f64 - 1.0) / EMBED_PAGE_SIZE as f64).ceil() as usize;
    max(1, num_pages)
}

pub async fn forget_queue_message(
    data: &Arc<RwLock<TypeMap>>,
    message: &mut Message,
    guild_id: GuildId,
) -> Result<(), ()> {
    let mut data_wlock = data.write().await;
    let cache_map = data_wlock.get_mut::<GuildCacheMap>().ok_or(())?;

    let cache = cache_map.get_mut(&guild_id).ok_or(())?;
    cache.queue_messages.retain(|(m, _)| m.id != message.id);

    Ok(())
}
