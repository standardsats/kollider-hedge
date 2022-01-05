use clap::Parser;
use std::error::Error;
use futures::StreamExt;
use kollider_api::kollider::{ChannelName, websocket::*};

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    #[clap(long, env = "KOLLIDER_API_KEY", hide_env_values = true)]
    api_key: String,
    #[clap(long, env = "KOLLIDER_API_SECRET", hide_env_values = true)]
    api_secret: String,
    #[clap(long, env = "KOLLIDER_API_PASSWORD", hide_env_values = true)]
    password: String,
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser, Debug)]
enum SubCommand {
    /// Start listening incoming API requests
    Serve {
        /// Host name to bind the service to
        #[clap(long, short('a'), default_value="0.0.0.0", env = "KOLLIDER_HEDGE_HOST")]
        host: String,
        /// Port to bind the service to
        #[clap(long, short, default_value="8080", env = "KOLLIDER_HEDGE_PORT")]
        port: u16,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    env_logger::init();

    match args.subcmd {
        SubCommand::Serve { host, port } => {
            let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
            let (msg_sender, msg_receiver) = futures_channel::mpsc::unbounded();
            let auth_msg = make_user_auth(&args.api_secret, &args.api_key, &args.password)?;
            let channels = vec![ChannelName::Matches];
            let symbols = vec![".BTCUSD.PERP".to_owned()];
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
                    println!("Received message: {:?}", message);
                })
                .await
        }
    }
    Ok(())
}
