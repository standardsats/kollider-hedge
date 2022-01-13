use crate::kollider::hedge::db::queries::{self, insert_update};
use crate::kollider::hedge::db::scheme::{HtlcUpdate, UpdateBody};
use crate::kollider::hedge::db::Pool;
use rweb::openapi::Spec;
use rweb::*;
use serde::{Deserialize, Serialize};
use std::convert::From;
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;

impl rweb::reject::Reject for queries::Error {}

#[derive(Serialize, Deserialize, Schema)]
struct HtlcInfo {
    pub channel_id: String,
    pub sats: i64,
    pub rate: u64,
}

impl HtlcInfo {
    fn into_update(self) -> HtlcUpdate {
        HtlcUpdate {
            channel_id: self.channel_id,
            sats: self.sats,
            rate: self.rate as i64,
        }
    }
}

#[post("/hedge/htlc")]
#[openapi(
    tags("node"),
    summary = "Update state of position to adjust to the new HTLC incoming or outcoming from a fiat channel.",
    description = "When Eclar node receives a new HTLC to a fiat channel the endpoint is called with positive amount. If the HTLC is outcoming from the channel, the provided amount has to be negative."
)]
async fn hedge_htlc(#[data] pool: Pool, body: Json<HtlcInfo>) -> Result<Json<()>, Rejection> {
    let htlc = body.into_inner();
    insert_update(&pool, UpdateBody::Htlc(htlc.into_update())).await?;
    Ok(Json::from(()))
}

pub async fn hedge_api_specs(pool: Pool) -> Result<Spec, Box<dyn Error>> {
    let (_spec, _) = openapi::spec().build(|| hedge_htlc(pool));
    Ok(_spec)
}

pub async fn serve_api(host: &str, port: u16, pool: Pool) -> Result<(), Box<dyn Error>> {
    let filter = hedge_htlc(pool);
    serve(filter).run((IpAddr::from_str(host)?, port)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use futures_util::future::TryFutureExt;
    use std::panic::{UnwindSafe, AssertUnwindSafe};
    use kollider_hedge_client::client::{self, HedgeClient};

    const SERVICE_TEST_PORT: u16 = 8098;
    const SERVICE_TEST_HOST: &str = "127.0.0.1";

    async fn run_api_test<Fn, Fut>(pool: Pool, test_body: Fn)
    where
        Fn: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let serve_task = serve_api(SERVICE_TEST_HOST, SERVICE_TEST_PORT, pool);
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
            let client = HedgeClient::new(&format!("http://{}:{}", SERVICE_TEST_HOST, SERVICE_TEST_PORT));
            client.hedge_htlc(client::HtlcInfo {
                channel_id: "aboba".to_owned(),
                sats: 100,
                rate: 2500,
            }).await.unwrap();
        }).await;
    }
}
