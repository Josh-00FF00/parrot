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

    let ip = reqwest::get("https://ifconfig.me/ip").await?.text().await?;
    println!("My IP address is: {}", ip);

    match std::env::var("CREDENTIALS_DIRECTORY") {
        Ok(cred_dir) => {
            info!("Loading creds from: {cred_dir}");
            dotenv::from_path(Path::new(&cred_dir).join("app.env"))
                .expect("Failed to load from CREDENTIALS_DIRECTORY");

            info!("Loading ytdlp cookies");

            let builder = reqwest::Client::builder();
            let cookies = Path::new(&cred_dir).join("cookies.txt");

            if let Ok(_jar) = load_cookie_jar_from_path(&cookies) {
                info!("Successfully loaded cookies into cookie jar");
                // builder = builder.cookie_provider(jar);
            }

            let client = builder.build().expect("Failed to build reqwest client");

            REQWEST_CLIENT
                .set(client)
                .expect("Failed to store global client object");
        }
        Err(_) => {
            info!("Loading default env");
            let _ = dotenv::dotenv();
        }
    }

    let mut parrot = Client::default().await?;
    if let Err(why) = parrot.start().await {
        error!("Fatality! Parrot crashed because: {:?}", why);
    };

    Ok(())
}
