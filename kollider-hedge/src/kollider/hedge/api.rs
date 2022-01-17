use crate::kollider::hedge::db::queries::{self, insert_update};
use crate::kollider::hedge::db::Pool;
use ::log::*;
use chrono::prelude::*;
use kollider_hedge_domain::api::*;
use kollider_hedge_domain::state::*;
use kollider_hedge_domain::update::*;
use rweb::openapi::Spec;
use rweb::*;
use std::convert::From;
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

impl rweb::reject::Reject for queries::Error {}

#[post("/hedge/htlc")]
#[openapi(
    tags("node"),
    summary = "Update state of position to adjust to the new HTLC incoming or outcoming from a fiat channel.",
    description = "When Eclar node receives a new HTLC to a fiat channel the endpoint is called with positive amount. If the HTLC is outcoming from the channel, the provided amount has to be negative."
)]
async fn hedge_htlc(
    #[data] pool: Pool,
    #[data] state_mx: Arc<Mutex<State>>,
    body: Json<HtlcInfo>,
) -> Result<Json<()>, Rejection> {
    let htlc = body.into_inner();
    let update = StateUpdate {
        created: Utc::now().naive_utc(),
        body: UpdateBody::Htlc(htlc.into_update()),
    };
    debug!("Calling hedge_htlc");
    {
        let mut state = state_mx.lock().await;
        insert_update(&pool, update.body.clone()).await?;
        state.apply_update(update)?;
        debug!("New state {:?}", state);
    }

    Ok(Json::from(()))
}

#[get("/state")]
#[openapi(
    tags("management"),
    summary = "Return current state of the plugin",
    description = "The full state of the server that can be quite slow. The en"
)]
async fn query_state(#[data] state_mx: Arc<Mutex<State>>) -> Result<Json<State>, Rejection> {
    let state = state_mx.lock().await;
    Ok(Json::from(state.clone()))
}

pub async fn hedge_api_specs(pool: Pool) -> Result<Spec, Box<dyn Error>> {
    let state = Arc::new(Mutex::new(State::default()));
    let (_spec, _) =
        openapi::spec().build(|| hedge_htlc(pool, state.clone()).or(query_state(state)));
    Ok(_spec)
}

pub async fn serve_api(host: &str, port: u16, pool: Pool, state: Arc<Mutex<State>>) -> Result<(), Box<dyn Error>> {
    let filter = hedge_htlc(pool, state.clone()).or(query_state(state));
    serve(filter).run((IpAddr::from_str(host)?, port)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use futures_util::future::TryFutureExt;
    use kollider_hedge_client::client::HedgeClient;
    use kollider_hedge_domain::api::HtlcInfo;
    use std::panic::AssertUnwindSafe;

    const SERVICE_TEST_PORT: u16 = 8098;
    const SERVICE_TEST_HOST: &str = "127.0.0.1";

    async fn run_api_test<Fn, Fut>(pool: Pool, test_body: Fn)
    where
        Fn: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _ = env_logger::builder().is_test(true).try_init();

        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let state = Arc::new(Mutex::new(State::default()));
            let serve_task = serve_api(SERVICE_TEST_HOST, SERVICE_TEST_PORT, pool, state);
            futures::pin_mut!(serve_task);
            futures::future::select(serve_task, receiver.map_err(drop)).await;
        });

        let res = AssertUnwindSafe(test_body()).catch_unwind().await;

        sender.send(()).unwrap();

        assert!(res.is_ok());
    }

    #[sqlx_database_tester::test(pool(variable = "pool"))]
    async fn test_api_hedge() {
        run_api_test(pool, || async {
            let client = HedgeClient::new(&format!(
                "http://{}:{}",
                SERVICE_TEST_HOST, SERVICE_TEST_PORT
            ));
            client
                .hedge_htlc(HtlcInfo {
                    channel_id: "aboba".to_owned(),
                    sats: 100,
                    rate: 2500,
                })
                .await
                .unwrap();

            let state = client.query_state().await.unwrap();
            assert_eq!(
                state.channels_hedge,
                hashmap! {
                    "aboba".to_owned() => ChannelHedge {
                        sats: 100,
                        rate: 2500,
                    }
                }
            );
        })
        .await;
    }
}
