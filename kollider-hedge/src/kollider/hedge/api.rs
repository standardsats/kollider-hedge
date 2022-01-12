use std::error::Error;
use rweb::openapi::Spec;
use rweb::*;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;
use crate::kollider::hedge::db::Pool;
use crate::kollider::hedge::db::scheme::{UpdateBody, HtlcUpdate};
use crate::kollider::hedge::db::queries::{self, insert_update};
use std::convert::From;

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