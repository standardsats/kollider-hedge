use rweb::Schema;
use serde::{Serialize, Deserialize};
use super::update::*;

#[derive(Serialize, Deserialize, Schema)]
pub struct HtlcInfo {
    pub channel_id: String,
    pub sats: i64,
    pub rate: u64,
}

impl HtlcInfo {
    pub fn into_update(self) -> HtlcUpdate {
        HtlcUpdate {
            channel_id: self.channel_id,
            sats: self.sats,
            rate: self.rate as i64,
        }
    }
}
