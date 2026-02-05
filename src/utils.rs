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
    match interaction
        .create_response(
            &http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::default().add_embed(embed.clone()),
            ),
        )
        .await
        .map_err(Into::into)
    {
        Ok(val) => Ok(val),
        Err(err) => match err {
            ParrotError::Serenity(Error::Http(HttpError::UnsuccessfulRequest(ref req))) => {
                match req.error.code {
                    40060 => edit_embed_response(http, interaction, embed)
                        .await
                        .map(|_| ()),
                    _ => Err(err),
                }
            }
            _ => Err(err),
        },
    }
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

    let position = get_human_readable_timestamp(Some(track.get_info().await.unwrap().position));
    let duration = get_human_readable_timestamp(meta.duration);

    let embed = embed.field("Progress", format!(">>> {} / {}", position, duration), true);

    let embed = match meta.channel {
        Some(ref channel) => embed.field("Channel", format!(">>> {}", channel), true),
        None => embed.field("Channel", ">>> N/A", true),
    };

    let source_url = meta.source_url.as_ref().unwrap();

    let (footer_text, footer_icon_url) = get_footer_info(source_url);
    embed.footer(CreateEmbedFooter::new(footer_text).icon_url(footer_icon_url))
}

pub fn get_footer_info(url: &str) -> (String, String) {
    let url_data = Url::parse(url).unwrap();
    let domain = url_data.host_str().unwrap();

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

pub fn compare_domains(domain: &str, subdomain: &str) -> bool {
    subdomain == domain || subdomain.ends_with(domain)
}

pub fn track_to_meta(track: &TrackHandle) -> Arc<AuxMetadata> {
    track.data::<AuxMetadata>()
}

pub fn load_cookie_jar_from_path(path: &Path) -> io::Result<Arc<Jar>> {
    let jar = Arc::new(Jar::default());

    let reader = BufReader::new(File::open(path)?);

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            // Skip comments (often header info) and empty lines
            continue;
        }

        // Attempt to parse and add the cookie
        if let Err(e) = add_netscape_cookie(&jar, trimmed) {
            error!("Skipping invalid line: '{}'. Error: {:?}", trimmed, e);
        }
    }

    Ok(jar)
}

fn add_netscape_cookie(jar: &Arc<Jar>, line: &str) -> Result<(), String> {
    let parts: Vec<&str> = line.split('\t').collect();

    // Netscape format standard usually requires 7 columns
    if parts.len() < 7 {
        return Err("Line has fewer than 7 columns".to_string());
    }

    let domain = parts[0];
    let path = parts[2];
    let secure = parts[3];
    let _expiration = parts[4]; // Ignoring expiration for session-based scraping
    let name = parts[5];
    let value = parts[6];

    // Build the cookie string: "Name=Value; Domain=...; Path=..."
    let mut cookie_str = format!("{}={}; Domain={}; Path={}", name, value, domain, path);

    if secure == "TRUE" {
        cookie_str.push_str("; Secure");
    }

    // Determine the URL for the jar to associate the cookie with
    // We remove the leading '.' (e.g., .youtube.com -> youtube.com)
    let clean_domain = domain.trim_start_matches('.');

    let scheme = if secure == "TRUE" { "https" } else { "http" };

    let url_str = format!("{}://{}", scheme, clean_domain);

    let url = url_str
        .parse::<Url>()
        .map_err(|_| format!("Could not parse URL from domain: {}", clean_domain))?;

    jar.add_cookie_str(&cookie_str, &url);

    Ok(())
}
