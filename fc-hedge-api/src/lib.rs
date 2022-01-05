use rweb::openapi::Spec;
use rweb::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Schema)]
struct HtlcInfo {
    pub sats: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_spec() {
        println!(
            "{}",
            serde_json::to_string_pretty(&hedge_api_specs()).unwrap()
        );
        assert!(false);
    }
}
