use serenity::model::gateway::GatewayIntents;
use songbird::serenity::SerenityInit;
use tracing::info;

use std::{collections::HashMap, env, error::Error};

use crate::{
    errors::ParrotError,
    global_settings::GlobalSettings,
    global_settings::GlobalSettingsMap,
    guild::{cache::GuildCacheMap, settings::GuildSettingsMap},
    handlers::SerenityHandler,
};

pub struct Client {
    client: serenity::Client,
}

impl Client {
    pub async fn default() -> Result<Client, Box<dyn Error>> {
        let token = env::var("DISCORD_TOKEN").expect("Fatality! DISCORD_TOKEN not set!");
        Client::new(token).await
    }

    pub async fn new(token: String) -> Result<Client, Box<dyn Error>> {
        let application_id = env::var("DISCORD_APP_ID")
            .expect("Fatality! DISCORD_APP_ID not set!")
            .parse()?;

        let gateway_intents = GatewayIntents::non_privileged();

        let client = serenity::Client::builder(token, gateway_intents)
            .event_handler(SerenityHandler)
            .application_id(application_id)
            .register_songbird()
            .await?;

        info!("Serenity initialised!");

        let mut data = client.data.write().await;
        data.insert::<GuildCacheMap>(HashMap::default());
        data.insert::<GuildSettingsMap>(HashMap::default());
        data.insert::<GlobalSettingsMap>(GlobalSettings::default());

        drop(data);

        Ok(Client { client })
    }

    pub async fn start(&mut self) -> Result<(), ParrotError> {
        self.client.start().await.map_err(Into::into)
    }
}
