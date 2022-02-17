mod kollider;

#[cfg(test)]
#[macro_use]
extern crate maplit;

use crate::kollider::hedge::api::{hedge_api_specs, serve_api};
use crate::kollider::hedge::db::{create_db_pool, queries::query_state};
use clap::Parser;
use futures::future::{AbortHandle, Abortable, Aborted};
use futures::StreamExt;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use kollider_api::kollider::{websocket::*, ChannelName};
use kollider_api::kollider::{KolliderMsg, KolliderTaggedMsg};
use kollider_hedge_domain::state::{state_action_worker, HedgeConfig, State};
use log::*;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::sleep;

#[derive(Parser, Debug, Clone)]
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

#[derive(Parser, Debug, Clone)]
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
        /// leverage * 100, 100 means 1x, 200 means 2x. Defines the leverage of opened positions. If you hedge at 2x, you need 1/2 of sats to hedge all fixed USD value, but you will loose you money at 50% dropdowns.
        #[clap(long, default_value = "100", env = "KOLLIDER_HEDGE_LEVERAGE")]
        leverage: u64,
    },
    /// Output swagger spec
    Swagger,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    env_logger::init();

    match args.subcmd.clone() {
        SubCommand::Serve {
            host,
            port,
            spread_percent,
            leverage,
        } => {
            loop {
                let args = args.clone();

                info!("Connecting to database");
                let pool = create_db_pool(&args.dbconnect).await?;
                info!("Connected");
                let config = HedgeConfig {
                    spread_percent,
                    hedge_leverage: leverage,
                };

                info!("Reconstructing state from database");
                let state = query_state(&pool, config).await?;
                let state_mx = Arc::new(Mutex::new(state));
                let state_notify = Arc::new(Notify::new());
                let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
                let auth_notify = Arc::new(Notify::new());
                let (abort_ws_handle, abort_ws_reg) = AbortHandle::new_pair();
                let (abort_exe_handle, abort_exe_reg) = AbortHandle::new_pair();
                let (abort_api_handle, abort_api_reg) = AbortHandle::new_pair();
                info!("Spawning websocket control thread");
                tokio::spawn({
                    let state = state_mx.clone();
                    let state_notify = state_notify.clone();
                    let stdin_tx = stdin_tx.clone();
                    let auth_notify = auth_notify.clone();
                    let abort_api_handle = abort_api_handle.clone();
                    let future = async move {
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
                            error!("Websocket control thread error: {}", e);
                            abort_exe_handle.abort();
                            abort_api_handle.abort();
                        }
                    };
                    Abortable::new(future, abort_ws_reg)
                });
                info!("Spawning action executor thread");
                tokio::spawn({
                    let state_mx = state_mx.clone();
                    let state_notify = state_notify.clone();
                    let stdin_tx = stdin_tx.clone();
                    let auth_notify = auth_notify.clone();
                    let abort_api_handle = abort_api_handle.clone();
                    let future = async move {
                        auth_notify.notified().await;
                        let res = state_action_worker(state_mx, state_notify, |action| {
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
                        if res.is_err() {
                            error!("Aborting WS and API thread");
                            abort_ws_handle.abort();
                            abort_api_handle.abort();
                        }
                    };
                    Abortable::new(future, abort_exe_reg)
                });
                info!("Serving API");
                let api_future = serve_api(&host, port, pool, state_mx, state_notify);
                match Abortable::new(api_future, abort_api_reg).await {
                    Ok(mres) => mres?,
                    Err(Aborted) => {
                        error!("API thread aborted");
                    }
                }

                let restart_dt = Duration::from_secs(5);
                info!("Adding {:?} delay before restarting logic", restart_dt);
                sleep(restart_dt).await;
            }
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
    trace!("Sending Auth message to websocket");
    stdin_tx.unbounded_send(auth_msg)?;

    let (abort_handle, abort_reg) = AbortHandle::new_pair();
    tokio::spawn(async move {
        let res = kollider_websocket(stdin_rx, msg_sender).await;
        if let Err(e) = res {
            error!("Websocket thread failed: {}", e);
        }
        abort_handle.abort();
    });

    let mut counter = 0;
    let listen_future = msg_receiver.for_each(|message| {
        let state_mx = state_mx.clone();
        let state_notify = state_notify.clone();
        let auth_notify = auth_notify.clone();
        let stdin_tx = stdin_tx.clone();
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
                info!(
                    "We passed authentification on Kollider, subscibing and getting current state"
                );
                auth_notify.notify_one();
                debug!("Notified state that auth is passed");

                let channels = vec![ChannelName::IndexValues];
                let symbols = vec![".BTCUSD".to_owned()];
                stdin_tx
                    .unbounded_send(KolliderMsg::Subscribe {
                        _type: SubscribeTag::Tag,
                        channels,
                        symbols,
                    })
                    .map_err(|e| {
                        error!("Failed to send subscribe message: {}", e);
                    })
                    .ok();
                stdin_tx
                    .unbounded_send(KolliderMsg::FetchOpenOrders {
                        _type: FetchOpenOrdersTag::Tag,
                    })
                    .map_err(|e| {
                        error!("Failed to send fetch orders message: {}", e);
                    })
                    .ok();
                stdin_tx
                    .unbounded_send(KolliderMsg::FetchPositions {
                        _type: FetchPositionsTag::Tag,
                    })
                    .map_err(|e| {
                        error!("Failed to send fetch positions message: {}", e);
                    })
                    .ok();
            }
        }
    });
    let abortable_listen = Abortable::new(listen_future, abort_reg);

    Ok(abortable_listen.await?)
}
