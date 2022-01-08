use std::error::Error;
use rweb::openapi::Spec;
use rweb::*;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;
use crate::kollider::hedge::db::Pool;

#[derive(Serialize, Deserialize, Schema)]
struct HtlcInfo {
    pub sats: i64,
    pub channel_id: String,
}

#[post("/hedge/htlc")]
#[openapi(
    tags("node"),
    summary = "Update state of position to adjust to the new HTLC incoming or outcoming from a fiat channel.",
    description = "When Eclar node receives a new HTLC to a fiat channel the endpoint is called with positive amount. If the HTLC is outcoming from the channel, the provided amount has to be negative."
)]
fn hedge_htlc(htlc: Json<HtlcInfo>) -> Json<()> {


    return Json::from(());
}

pub fn hedge_api_specs() -> Spec {
    let (_spec, _) = openapi::spec().build(|| hedge_htlc());
    return _spec;
}

pub async fn serve_api(host: &str, port: u16, pool: Pool) -> Result<(), Box<dyn Error>> {
    let filter = hedge_htlc();
    serve(filter).run((IpAddr::from_str(host)?, port)).await;
    Ok(())
}