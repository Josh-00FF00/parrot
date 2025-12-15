use parrot::client::Client;
use std::{error::Error, path::Path};
use tracing::{error, info};
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match std::env::var("CREDENTIALS_DIRECTORY") {
        Ok(cred_dir) => {
            info!("Loading creds from: {cred_dir}");
            dotenv::from_path(Path::new(&cred_dir).join("app.env"))
                .expect("Failed to load from CREDENTIALS_DIRECTORY");
        }
        Err(_) => {
            info!("Loading default env");
            dotenv::dotenv().expect("Failed to load default dotenv");
        }
    }

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut parrot = Client::default().await?;
    if let Err(why) = parrot.start().await {
        error!("Fatality! Parrot crashed because: {:?}", why);
    };

    Ok(())
}
