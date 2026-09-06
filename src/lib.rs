pub mod client;
pub mod commands;
pub mod connection;
pub mod errors;
pub mod global_settings;
pub mod guild;
pub mod handlers;
pub mod messaging;
pub mod sources;
pub mod utils;

use std::{env, sync::LazyLock};

const DEFAULT_SETTINGS_PATH: &str = "data/settings";

pub static SETTINGS_PATH: LazyLock<String> =
    LazyLock::new(|| env::var("STATE_DIRECTORY").unwrap_or(DEFAULT_SETTINGS_PATH.to_string()));

#[cfg(test)]
pub mod test;
