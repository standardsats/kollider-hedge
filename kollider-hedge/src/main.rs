mod kollider;

#[cfg(test)]
#[macro_use] extern crate maplit;

use crate::kollider::hedge::api::{serve_api, hedge_api_specs};
use crate::kollider::hedge::db::{create_db_pool};
use clap::Parser;
use futures::StreamExt;
use kollider_api::kollider::{websocket::*, ChannelName};
use log::*;
use std::error::Error;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    #[clap(long, env = "KOLLIDER_API_KEY", hide_env_values = true)]
    api_key: String,
    #[clap(long, env = "KOLLIDER_API_SECRET", hide_env_values = true)]
    api_secret: String,
    #[clap(long, env = "KOLLIDER_API_PASSWORD", hide_env_values = true)]
    password: String,
    /// PostgreSQL connection string
    #[clap(long, short, default_value = "postgres://kollider:kollider@localhost/kollider_hedge", env = "KOLLIDER_HEDGE_POSTGRES")]
    dbconnect: String,
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser, Debug)]
enum SubCommand {
    /// Start listening incoming API requests
    Serve {
        /// Host name to bind the service to
        #[clap(
            long,
            short('a'),
            default_value = "0.0.0.0",
            env = "KOLLIDER_HEDGE_HOST"
        )]
        host: String,
        /// Port to bind the service to
        #[clap(long, short, default_value = "8081", env = "KOLLIDER_HEDGE_PORT")]
        port: u16,
    },
    /// Output swagger spec
    Swagger,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    env_logger::init();

    match args.subcmd {
        SubCommand::Serve { host, port } => {
            tokio::spawn(async move {
                if let Err(e) = listen_websocket(&args.api_secret, &args.api_key, &args.password).await {
                    error!("Websocket thread error: {}", e);
                }
            });

            let pool = create_db_pool(&args.dbconnect).await?;

            serve_api(&host, port, pool).await?;
        }
        SubCommand::Swagger => {
            let pool = create_db_pool(&args.dbconnect).await?;
            let specs = hedge_api_specs(pool).await?;
            let specs_str = serde_json::to_string_pretty(&specs)?;
            println!("{}", specs_str);
        }
    }
    Ok(())
}

async fn listen_websocket(
    api_secret: &str,
    api_key: &str,
    password: &str,
) -> Result<(), Box<dyn Error>> {
    let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
    let (msg_sender, msg_receiver) = futures_channel::mpsc::unbounded();
    let auth_msg = make_user_auth(api_secret, api_key, password)?;
    let channels = vec![ChannelName::IndexValues];
    let symbols = vec![".BTCUSD".to_owned()];
    stdin_tx.unbounded_send(auth_msg)?;
    stdin_tx.unbounded_send(KolliderMsg::Subscribe {
        _type: SubscribeTag::Tag,
        channels,
        symbols,
    })?;
    // if let Some(a) = action {
    //     stdin_tx.unbounded_send(a.to_message())?;
    // }
    tokio::spawn(kollider_websocket(stdin_rx, msg_sender));

    msg_receiver
        .for_each(|message| async move {
            // trace!("Received message: {:?}", message);
        })
        .await;

    Ok(())
}
