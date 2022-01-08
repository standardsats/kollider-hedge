use chrono::prelude::*;
use std::collections::HashMap;

/// All database updates are collected to a single table that
/// allows to reconstruct current state of the system by replaying
/// all events until required timestamp.
pub enum StateUpdate {
    /// Updating state of required hedging for given channel
    Htlc(HtlcUpdate),
    /// Caching current state to database for speeding startup time
    Snapshot(StateSnapshot),
}

/// Unique hash of channel
pub type ChannelId = String;
/// Amount of satoshis
pub type Sats = i64;

pub struct HtlcUpdate {
    pub sats: Sats,
    pub channel_id: ChannelId,
    pub timestamp: DateTime<Utc>,
}

pub struct StateSnapshot {
    pub channels_hedge: HashMap<ChannelId, Sats>,
}