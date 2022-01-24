mod kollider;

#[cfg(test)]
#[macro_use]
extern crate maplit;

use crate::kollider::hedge::api::{hedge_api_specs, serve_api};
use crate::kollider::hedge::db::{create_db_pool, queries::query_state};
use clap::Parser;
use futures::StreamExt;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use kollider_api::kollider::{websocket::*, ChannelName};
use kollider_api::kollider::{KolliderMsg, KolliderTaggedMsg};
use kollider_hedge_domain::state::{state_action_worker, HedgeConfig, State};
use log::*;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

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
    #[clap(
        long,
        short,
        default_value = "postgres://kollider:kollider@localhost/kollider_hedge",
        env = "KOLLIDER_HEDGE_POSTGRES"
    )]
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
        /// That percent is added and subtructed from current price to ensure that order is executed
        #[clap(long, default_value = "0.1", env = "KOLLIDER_HEDGE_SPREAD")]
        spread_percent: f64,
    },
    /// Output swagger spec
    Swagger,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    env_logger::init();

    match args.subcmd {
        SubCommand::Serve {
            host,
            port,
            spread_percent,
        } => {
            let pool = create_db_pool(&args.dbconnect).await?;
            let config = HedgeConfig { spread_percent };

            let state = query_state(&pool, config).await?;
            let state_mx = Arc::new(Mutex::new(state));
            let state_notify = Arc::new(Notify::new());
            let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
            let auth_notify = Arc::new(Notify::new());
            tokio::spawn({
                let state = state_mx.clone();
                let state_notify = state_notify.clone();
                let stdin_tx = stdin_tx.clone();
                let auth_notify = auth_notify.clone();
                async move {
                    let ws_auth = WebsocketAuth {
                        api_secret: &args.api_secret,
                        api_key: &args.api_key,
                        password: &args.password,
                    };
                    if let Err(e) = listen_websocket(
                        stdin_tx,
                        stdin_rx,
                        state,
                        state_notify,
                        auth_notify,
                        ws_auth,
                    )
                    .await
                    {
                        error!("Websocket thread error: {}", e);
                    }
                }
            });
            tokio::spawn({
                let state_mx = state_mx.clone();
                let state_notify = state_notify.clone();
                let stdin_tx = stdin_tx.clone();
                let auth_notify = auth_notify.clone();
                async move {
                    auth_notify.notified().await;
                    state_action_worker(state_mx, state_notify, |action| {
                        let stdin_tx = stdin_tx.clone();
                        async move {
                            log::info!("Executing action: {:?}", action);
                            for msg in action.to_kollider_messages() {
                                stdin_tx.unbounded_send(msg)?;
                            }
                            Ok(())
                        }
                    })
                    .await;
                }
            });

            serve_api(&host, port, pool, state_mx, state_notify).await?;
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

struct WebsocketAuth<'a> {
    api_secret: &'a str,
    api_key: &'a str,
    password: &'a str,
}

async fn listen_websocket(
    stdin_tx: UnboundedSender<KolliderMsg>,
    stdin_rx: UnboundedReceiver<KolliderMsg>,
    state_mx: Arc<Mutex<State>>,
    state_notify: Arc<Notify>,
    auth_notify: Arc<Notify>,
    ws_auth: WebsocketAuth<'_>,
) -> Result<(), Box<dyn Error>> {
    let (msg_sender, msg_receiver) = futures_channel::mpsc::unbounded();
    let auth_msg = make_user_auth(ws_auth.api_secret, ws_auth.api_key, ws_auth.password)?;
    let channels = vec![ChannelName::IndexValues];
    let symbols = vec![".BTCUSD".to_owned()];
    stdin_tx.unbounded_send(auth_msg)?;
    stdin_tx.unbounded_send(KolliderMsg::Subscribe {
        _type: SubscribeTag::Tag,
        channels,
        symbols,
    })?;
    stdin_tx.unbounded_send(KolliderMsg::FetchOpenOrders {
        _type: FetchOpenOrdersTag::Tag,
    })?;
    stdin_tx.unbounded_send(KolliderMsg::FetchPositions {
        _type: FetchPositionsTag::Tag,
    })?;
    tokio::spawn(kollider_websocket(stdin_rx, msg_sender));

    let mut counter = 0;
    msg_receiver
        .for_each(|message| {
            let state_mx = state_mx.clone();
            let state_notify = state_notify.clone();
            let auth_notify = auth_notify.clone();
            async move {
                if let KolliderMsg::Tagged(KolliderTaggedMsg::IndexValues(v)) = &message {
                    counter += 1;
                    if counter % 10 == 0 {
                        info!("Received index: {:?}", v);
                    }
                } else {
                    info!("Received message: {:?}", message);
                }
                let mut state = state_mx.lock().await;
                let changed = state.apply_kollider_message(message.clone());
                if changed {
                    state_notify.notify_one();
                }

                if let KolliderMsg::Tagged(KolliderTaggedMsg::Authenticate { .. }) = message {
                    auth_notify.notify_one();
                }
            }
        })
        .await;

    Ok(())
}
