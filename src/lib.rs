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

use lazy_static::lazy_static;
use std::env;

const DEFAULT_SETTINGS_PATH: &str = "data/settings";

lazy_static! {
    pub static ref SETTINGS_PATH: String =
        env::var("STATE_DIRECTORY").unwrap_or(DEFAULT_SETTINGS_PATH.to_string());
}

#[cfg(test)]
pub mod test;
