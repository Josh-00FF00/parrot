use parrot::{
    client::Client,
    utils::{REQWEST_CLIENT, load_cookie_jar_from_path},
};
use std::{error::Error, path::Path};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match reqwest::get("https://ifconfig.me/ip").await {
        Ok(resp) => match resp.text().await {
            Ok(ip) => info!("My IP address is: {ip}"),
            Err(err) => error!("Failed to read IP address: {err:?}"),
        },
        Err(err) => error!("Failed to fetch IP address: {err:?}"),
    }

    match std::env::var("CREDENTIALS_DIRECTORY") {
        Ok(cred_dir) => {
            info!("Loading creds from: {cred_dir}");
            dotenvy::from_path(Path::new(&cred_dir).join("app.env"))
                .expect("Failed to load from CREDENTIALS_DIRECTORY");

            info!("Loading ytdlp cookies");

            let builder = reqwest::Client::builder();
            let cookies = Path::new(&cred_dir).join("cookies.txt");

            let client = match load_cookie_jar_from_path(&cookies) {
                Ok(jar) => {
                    info!("Successfully loaded cookies into cookie jar");
                    builder.cookie_provider(jar).build()
                }
                Err(err) => {
                    info!("No cookies loaded, using default client: {err}");
                    builder.build()
                }
            }
            .expect("Failed to build reqwest client");

            REQWEST_CLIENT
                .set(client)
                .expect("Failed to store global client object");
        }
        Err(_) => {
            info!("Loading default env");
            let _ = dotenvy::dotenv();
        }
    }

    let mut parrot = Client::default().await?;
    if let Err(why) = parrot.start().await {
        error!("Fatality! Parrot crashed because: {:?}", why);
    };

    Ok(())
}
