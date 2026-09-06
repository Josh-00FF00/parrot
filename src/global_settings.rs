use serde::{Deserialize, Serialize};
use serenity::prelude::TypeMapKey;
use std::{
    fs::{OpenOptions, create_dir_all},
    io::{BufReader, BufWriter},
    path::Path,
};
use tracing::info;

use crate::{SETTINGS_PATH, errors::ParrotError};

#[derive(Deserialize, Serialize, Debug)]
pub struct SpotifySettings {
    pub spotify_refresh_token: String,
    pub spotify_access_token: String,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct GlobalSettings {
    pub spotify: Option<SpotifySettings>,
}

impl GlobalSettings {
    pub fn path() -> String {
        format!("{}/global.json", SETTINGS_PATH.as_str())
    }

    pub fn load_if_exists(&mut self) -> Result<(), ParrotError> {
        let path = GlobalSettings::path();
        if !Path::new(&path).exists() {
            return Ok(());
        }
        self.load()
    }

    pub fn load(&mut self) -> Result<(), ParrotError> {
        let path = GlobalSettings::path();
        info!("Loading from: {path}");
        let file = OpenOptions::new().read(true).open(path)?;
        let reader = BufReader::new(file);
        *self = serde_json::from_reader::<_, GlobalSettings>(reader)?;
        Ok(())
    }

    pub fn save(&self) -> Result<(), ParrotError> {
        create_dir_all(SETTINGS_PATH.as_str())?;

        let path = GlobalSettings::path();

        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = file.metadata()?.permissions();
            permissions.set_mode(0o600);
            file.set_permissions(permissions)?;
        }

        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self)?;
        Ok(())
    }

    pub fn set_tokens(&mut self, refresh_token: &str, access_token: &str) {
        self.spotify = Some(SpotifySettings {
            spotify_refresh_token: refresh_token.to_string(),
            spotify_access_token: access_token.to_string(),
        });
    }

    pub fn update_refresh_token(&mut self, refresh_token: &str) {
        let access_token = self
            .spotify
            .as_ref()
            .map(|settings| settings.spotify_access_token.clone())
            .unwrap_or_default();

        self.set_tokens(refresh_token, &access_token);
    }
}

#[derive(Debug)]
pub struct GlobalSettingsMap;

impl TypeMapKey for GlobalSettingsMap {
    type Value = GlobalSettings;
}
