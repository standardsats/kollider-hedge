mod kollider;

#[cfg(test)]
#[macro_use]
extern crate maplit;

use kollider_api::kollider::{KolliderMsg, KolliderTaggedMsg};
use crate::kollider::hedge::api::{hedge_api_specs, serve_api};
use crate::kollider::hedge::db::{create_db_pool, queries::query_state};
use clap::Parser;
use futures::StreamExt;
use kollider_api::kollider::{websocket::*, ChannelName, MarginType, OrderType, SettlementType};
use kollider_hedge_domain::state::{State, StateAction, state_action_worker, HEDGING_SYMBOL};
use log::*;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use futures_channel::mpsc::{UnboundedSender, UnboundedReceiver};
use uuid::Uuid;

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
            let pool = create_db_pool(&args.dbconnect).await?;

            let state = query_state(&pool).await?;
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
                    if let Err(e) =
                        listen_websocket(stdin_tx, stdin_rx, state, state_notify, auth_notify, ws_auth)
                            .await
                    {
                        error!("Websocket thread error: {}", e);
                    }
                }
            });
            tokio::spawn({
                let state = state_mx.clone();
                let state_notify = state_notify.clone();
                let stdin_tx = stdin_tx.clone();
                let auth_notify = auth_notify.clone();
                async move {
                    auth_notify.notified().await;
                    state_action_worker(state, state_notify, |action| {
                        let stdin_tx = stdin_tx.clone();
                        async move {
                            log::info!("Executing action: {:?}", action);
                            match action {
                                StateAction::OpenOrder { sats, price, side } => {
                                    let usd_price = 10 * 100_000_000 / price;
                                    let quantity = (sats as f64 / price as f64).ceil() as u64;
                                    log::info!("Price {} 10*USD/BTC", usd_price);
                                    log::info!("Quantity {}", quantity);
                                    stdin_tx.unbounded_send(KolliderMsg::Order {
                                        _type: OrderTag::Tag,
                                        price: usd_price,
                                        quantity,
                                        symbol: HEDGING_SYMBOL.to_owned(),
                                        leverage: 100,
                                        side: side.inverse(),
                                        margin_type: MarginType::Isolated,
                                        order_type: OrderType::Limit,
                                        settlement_type: SettlementType::Delayed,
                                        ext_order_id: Uuid::new_v4()
                                            .to_hyphenated()
                                            .encode_lower(&mut Uuid::encode_buffer())
                                            .to_owned(),
                                    })?
                                },
                                StateAction::CloseOrder { order_id } => {
                                    stdin_tx.unbounded_send(KolliderMsg::CancelOrder {
                                        _type: CancelOrderTag::Tag,
                                        order_id,
                                        symbol: HEDGING_SYMBOL.to_owned(),
                                        settlement_type: SettlementType::Delayed,
                                    })?
                                }
                            }
                            Ok(())
                        }
                    }).await;
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

                if let KolliderMsg::Tagged(KolliderTaggedMsg::Authenticate{..}) = message {
                    auth_notify.notify_one();
                }
            }
        })
        .await;

    Ok(())
}
