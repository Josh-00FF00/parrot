use reqwest::cookie::Jar;
use serenity::{
    Error,
    all::{
        CommandInteraction, CreateEmbedAuthor, CreateEmbedFooter, CreateInteractionResponse,
        CreateInteractionResponseMessage, EditInteractionResponse,
    },
    builder::CreateEmbed,
    http::{Http, HttpError},
    model::channel::Message,
    model::id::GuildId,
};
use songbird::{input::AuxMetadata, tracks::TrackHandle};
use std::{
    fs::File,
    io::{self, BufReader},
    path::Path,
    sync::Arc,
    time::Duration,
};
use std::{io::BufRead, sync::OnceLock};
use tracing::error;
use url::Url;

use crate::{errors::ParrotError, messaging::message::ParrotMessage};

pub static REQWEST_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn get_reqwest_client() -> &'static reqwest::Client {
    REQWEST_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .expect("Failed to build global client")
    })
}

pub async fn create_response(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    message: ParrotMessage,
) -> Result<(), ParrotError> {
    let embed = CreateEmbed::default().description(format!("{message}"));
    create_embed_response(http, interaction, embed).await
}

pub async fn create_response_text(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    content: &str,
) -> Result<(), ParrotError> {
    let embed = CreateEmbed::default().description(content);
    create_embed_response(http, interaction, embed).await
}

pub async fn edit_response(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    message: ParrotMessage,
) -> Result<Message, ParrotError> {
    let embed = CreateEmbed::default().description(format!("{message}"));
    edit_embed_response(http, interaction, embed).await
}

pub async fn edit_response_text(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    content: &str,
) -> Result<Message, ParrotError> {
    let embed = CreateEmbed::default().description(content);
    edit_embed_response(http, interaction, embed).await
}

pub async fn create_embed_response(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    embed: CreateEmbed,
) -> Result<(), ParrotError> {
    if let Err(err) = interaction
        .create_response(
            &http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::default().add_embed(embed.clone()),
            ),
        )
        .await
        .map_err(Into::into)
    {
        if let ParrotError::Serenity(serenity_error) = &err
            && let Error::Http(HttpError::UnsuccessfulRequest(req)) = serenity_error.as_ref()
            && req.error.code == 40060
        {
            return edit_embed_response(http, interaction, embed)
                .await
                .map(|_| ());
        }

        return Err(err);
    }

    Ok(())
}

pub async fn edit_embed_response(
    http: &Arc<Http>,
    interaction: &mut CommandInteraction,
    embed: CreateEmbed,
) -> Result<Message, ParrotError> {
    interaction
        .edit_response(&http, EditInteractionResponse::new().add_embed(embed))
        .await
        .map_err(Into::into)
}

pub async fn create_now_playing_embed(track: &TrackHandle) -> CreateEmbed {
    let meta = track_to_meta(track);
    let mut embed = CreateEmbed::default()
        .author(CreateEmbedAuthor::new(
            ParrotMessage::NowPlaying.to_string(),
        ))
        .title(meta.title.clone().unwrap_or("No Title".to_string()));

    if let Some(url) = &meta.source_url {
        embed = embed.url(url);
    }

    let position = track
        .get_info()
        .await
        .ok()
        .map(|info| info.position)
        .map(|position| get_human_readable_timestamp(Some(position)))
        .unwrap_or_else(|| "∞".to_string());
    let duration = get_human_readable_timestamp(meta.duration);

    let embed = embed.field("Progress", format!(">>> {} / {}", position, duration), true);

    let embed = match meta.channel {
        Some(ref channel) => embed.field("Channel", format!(">>> {}", channel), true),
        None => embed.field("Channel", ">>> N/A", true),
    };

    match &meta.source_url {
        Some(source_url) => {
            let (footer_text, footer_icon_url) = get_footer_info(source_url);
            embed.footer(CreateEmbedFooter::new(footer_text).icon_url(footer_icon_url))
        }
        None => embed,
    }
}

pub fn get_footer_info(url: &str) -> (String, String) {
    let domain = Url::parse(url)
        .ok()
        .and_then(|url_data| url_data.host_str().map(|host| host.to_string()))
        .unwrap_or_else(|| url.to_string());

    // remove www prefix because it looks ugly
    let domain = domain.replace("www.", "");

    (
        format!("Streaming via {}", domain),
        format!("https://www.google.com/s2/favicons?domain={}", domain),
    )
}

pub fn get_human_readable_timestamp(duration: Option<Duration>) -> String {
    match duration {
        Some(duration) if duration == Duration::MAX => "∞".to_string(),
        Some(duration) => {
            let seconds = duration.as_secs() % 60;
            let minutes = (duration.as_secs() / 60) % 60;
            let hours = duration.as_secs() / 3600;

            if hours < 1 {
                format!("{:02}:{:02}", minutes, seconds)
            } else {
                format!("{}:{:02}:{:02}", hours, minutes, seconds)
            }
        }
        None => "∞".to_string(),
    }
}

pub fn parse_timestamp(timestamp: &str) -> Option<u64> {
    let parts: Vec<&str> = timestamp.split(':').collect();

    if parts.len() > 3 {
        return None;
    }

    let mut total = 0u64;

    for part in parts {
        total = total
            .checked_mul(60)
            .and_then(|total| total.checked_add(part.parse::<u64>().ok()?))?;
    }

    Some(total)
}

pub fn command_guild_id(interaction: &CommandInteraction) -> Result<GuildId, ParrotError> {
    interaction.guild_id.ok_or(ParrotError::Other(
        "This command can only be used in a server",
    ))
}

pub fn compare_domains(configured: &str, host: &str) -> bool {
    let configured = configured.trim_matches('.').to_ascii_lowercase();
    let host = host.trim_matches('.').to_ascii_lowercase();

    host == configured
        || host
            .strip_suffix(&configured)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

pub fn track_to_meta(track: &TrackHandle) -> Arc<AuxMetadata> {
    track.data::<AuxMetadata>()
}

pub struct NetscapeCookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub name: String,
    pub value: String,
}

pub fn parse_netscape_cookie_line(line: &str) -> Option<NetscapeCookie> {
    let parts: Vec<&str> = line.split('\t').collect();

    // Netscape format standard usually requires 7 columns
    if parts.len() < 7 {
        return None;
    }

    Some(NetscapeCookie {
        domain: parts[0].to_string(),
        path: parts[2].to_string(),
        secure: parts[3] == "TRUE",
        name: parts[5].to_string(),
        value: parts[6].to_string(),
    })
}

pub fn load_cookie_jar_from_path(path: &Path) -> io::Result<Arc<Jar>> {
    let jar = Arc::new(Jar::default());

    let reader = BufReader::new(File::open(path)?);

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some(cookie) = parse_netscape_cookie_line(trimmed) else {
            error!("Skipping invalid cookie line: '{}'", trimmed);
            continue;
        };

        let mut cookie_str = format!(
            "{}={}; Domain={}; Path={}",
            cookie.name, cookie.value, cookie.domain, cookie.path
        );

        if cookie.secure {
            cookie_str.push_str("; Secure");
        }

        // Determine the URL for the jar to associate the cookie with
        // We remove the leading '.' (e.g., .youtube.com -> youtube.com)
        let clean_domain = cookie.domain.trim_start_matches('.');

        let scheme = if cookie.secure { "https" } else { "http" };

        let url_str = format!("{}://{}", scheme, clean_domain);

        let Ok(url) = url_str.parse::<Url>() else {
            error!("Skipping cookie with invalid domain: '{}'", clean_domain);
            continue;
        };

        jar.add_cookie_str(&cookie_str, &url);
    }

    Ok(jar)
}
