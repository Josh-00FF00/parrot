use parrot::client::Client;
use std::error::Error;
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut parrot = Client::default().await?;
    if let Err(why) = parrot.start().await {
        error!("Fatality! Parrot crashed because: {:?}", why);
    };

    Ok(())
}
