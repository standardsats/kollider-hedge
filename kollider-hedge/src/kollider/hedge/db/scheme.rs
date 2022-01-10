use chrono::prelude::*;
use std::collections::HashMap;
use std::str::FromStr;
use std::fmt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// All database updates are collected to a single table that
/// allows to reconstruct current state of the system by replaying
/// all events until required timestamp.
pub struct StateUpdate {
    pub created: NaiveDateTime,
    pub body: UpdateBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum UpdateBody {
    /// Updating state of required hedging for given channel
    Htlc(HtlcUpdate),
    /// Caching current state to database for speeding startup time
    Snapshot(StateSnapshot),
}

impl UpdateBody {
    pub fn tag(&self) -> UpdateTag {
        match self {
            UpdateBody::Htlc(_) => UpdateTag::Htlc,
            UpdateBody::Snapshot(_) => UpdateTag::Snapshot,
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum UpdateTag {
    Htlc,
    Snapshot,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub struct UnknownUpdateTag(String);

impl std::error::Error for UnknownUpdateTag {}

impl fmt::Display for UnknownUpdateTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Given UpdateTag '{}' is unknown, valid are: Htlc, Snapshot",
            self.0
        )
    }
}

impl fmt::Display for UpdateTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateTag::Htlc => write!(f, "htlc"),
            UpdateTag::Snapshot => write!(f, "snapshot"),
        }
    }
}

impl FromStr for UpdateTag {
    type Err = UnknownUpdateTag;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "htlc" => Ok(UpdateTag::Htlc),
            "snapshot" => Ok(UpdateTag::Snapshot),
            _ => Err(UnknownUpdateTag(s.to_owned())),
        }
    }
}

#[derive(Error, Debug)]
pub enum UpdateBodyError {
    #[error("Unknown update tag: {0}")]
    UnknownTag(#[from] UnknownUpdateTag),
    #[error("Failed to deserialize body with version {0} and tag {1}: {2}")]
    Deserialize(u16, UpdateTag, serde_json::Error),
    #[error("Unknown version tag: {0}")]
    UnexpectedVersion(u16),
}

pub const CURRENT_BODY_VERSION: u16 = 0;

impl UpdateTag {
    pub fn from_tag(tag: &str, version: u16, value: serde_json::Value) -> Result<UpdateBody, UpdateBodyError> {
        let tag = <UpdateTag as FromStr>::from_str(tag)?;
        if version != CURRENT_BODY_VERSION {
            return Err(UpdateBodyError::UnexpectedVersion(version));
        }
        tag.deserialize(value).map_err(|e| UpdateBodyError::Deserialize(version, tag, e))
    }

    pub fn deserialize(&self, value: serde_json::Value) -> Result<UpdateBody, serde_json::Error> {
        match self {
            Htlc => Ok(UpdateBody::Htlc(serde_json::from_value(value)?)),
            Snapshot => Ok(UpdateBody::Snapshot(serde_json::from_value(value)?)),
        }
    }
}

/// Unique hash of channel
pub type ChannelId = String;
/// Amount of satoshis
pub type Sats = i64;

#[derive(Serialize, Deserialize, Debug)]
pub struct HtlcUpdate {
    pub sats: Sats,
    pub channel_id: ChannelId,
    pub rate: Sats,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct ChannelHedge {
    pub sats: Sats,
    pub rate: Sats,
}

impl ChannelHedge {
    pub fn add_htlc(self, htlc: HtlcUpdate) -> ChannelHedge {
        let sats = (self.sats + htlc.sats).max(0);
        let weighted_sum = self.sats * htlc.rate + htlc.sats * self.rate;
        let rate = if weighted_sum <= 0 {
            0
        } else {
            (sats * self.rate * htlc.rate) / weighted_sum
        };

        ChannelHedge {
            sats,
            rate
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StateSnapshot {
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
}

pub struct State {
    pub last_changed: NaiveDateTime,
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
}

impl State {
    pub fn new() -> Self {
        State {
            last_changed: Utc::now().naive_utc(),
            channels_hedge: HashMap::new(),
        }
    }

    pub fn apply_update(&mut self, update: StateUpdate) {
        match update.body {
            UpdateBody::Htlc(htlc) => {
                self.add_htlc(htlc);
                self.last_changed = update.created;
            }
            UpdateBody::Snapshot(snaphsot) => {
                self.channels_hedge = snaphsot.channels_hedge;
                self.last_changed = update.created;
            }
        }
    }

    fn add_htlc(&mut self, htlc: HtlcUpdate) {
        let chan_id = htlc.channel_id.clone();
        let new_chan = if let Some(chan) = self.channels_hedge.get(&chan_id) {
            chan.clone().add_htlc(htlc)
        } else {
            ChannelHedge {
                sats: htlc.sats,
                rate: htlc.rate,
            }
        };
        self.channels_hedge.insert(chan_id, new_chan);
    }

    pub fn collect<I>(updates: I) -> Self
    where
        I: IntoIterator<Item = StateUpdate>,
    {
        let mut updates_iter = updates.into_iter();
        let state = State::new();
        loop {
            if let Some(upd) = updates_iter.next() {
                unimplemented!();
            } else {
                break;
            }
        }
        return state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weighted_summ_add_01() {
        let hedge = ChannelHedge {
            sats: 100,
            rate: 1000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: 50,
            rate: 1500,
        };

        let new_hedge = hedge.add_htlc(upd);
        assert_eq!(new_hedge, ChannelHedge {
            sats: 150,
            rate: 1125,
        });
    }
}
