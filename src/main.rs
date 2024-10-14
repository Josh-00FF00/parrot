use parrot::client::Client;
use std::error::Error;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut parrot = Client::default().await?;
    if let Err(why) = parrot.start().await {
        println!("Fatality! Parrot crashed because: {:?}", why);
    };

    Ok(())
}
