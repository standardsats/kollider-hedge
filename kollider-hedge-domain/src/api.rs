use super::update::*;
use rweb::Schema;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Schema)]
pub struct Stats {
    pub channels_sats: u64,
    pub channels_usd: f64,

    pub position_sats: u64,
    pub position_usd: u64,

    pub account_balance: f64,
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            channels_sats: 0,
            channels_usd: 0.0,
            position_sats: 0,
            position_usd: 0,
            account_balance: 0.,
        }
    }
}

impl Default for Stats {
    fn default() -> Stats {
        Stats::new()
    }
}