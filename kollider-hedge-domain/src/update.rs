use chrono::prelude::*;
use rweb::Schema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// All database updates are collected to a single table that
/// allows to reconstruct current state of the system by replaying
/// all events until required timestamp.
#[derive(Debug, PartialEq, Clone)]
pub struct StateUpdate {
    pub created: NaiveDateTime,
    pub body: UpdateBody,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
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

    pub fn json(&self) -> serde_json::Result<serde_json::Value> {
        match self {
            UpdateBody::Htlc(v) => serde_json::to_value(v),
            UpdateBody::Snapshot(v) => serde_json::to_value(v),
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
    #[error("Failed to deserialize body with version {0} and tag {1}: {2}. Body: {3}")]
    Deserialize(u16, UpdateTag, serde_json::Error, serde_json::Value),
    #[error("Unknown version tag: {0}")]
    UnexpectedVersion(u16),
}

pub const CURRENT_BODY_VERSION: u16 = 0;

impl UpdateTag {
    pub fn from_tag(
        tag: &str,
        version: u16,
        value: serde_json::Value,
    ) -> Result<UpdateBody, UpdateBodyError> {
        let tag = <UpdateTag as FromStr>::from_str(tag)?;
        if version != CURRENT_BODY_VERSION {
            return Err(UpdateBodyError::UnexpectedVersion(version));
        }
        tag.deserialize(value.clone())
            .map_err(|e| UpdateBodyError::Deserialize(version, tag, e, value))
    }

    pub fn deserialize(&self, value: serde_json::Value) -> Result<UpdateBody, serde_json::Error> {
        match self {
            UpdateTag::Htlc => Ok(UpdateBody::Htlc(serde_json::from_value(value)?)),
            UpdateTag::Snapshot => Ok(UpdateBody::Snapshot(serde_json::from_value(value)?)),
        }
    }
}

/// Unique hash of channel
pub type ChannelId = String;
/// Amount of satoshis
pub type Sats = i64;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct HtlcUpdate {
    pub channel_id: ChannelId,
    pub sats: Sats,
    pub rate: Sats,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Schema, Clone)]
pub struct ChannelHedge {
    pub sats: Sats,
    pub rate: Sats,
}

#[derive(Error, Debug, PartialEq, Clone)]
pub enum HtlcUpdateErr {
    #[error("Balance in sats is lower than update value. Was {0}, update {1}, new {2}")]
    InsufficientSatsBalance(Sats, Sats, Sats),
    #[error("Balance in fiat is lower than update value. Was {0}/{1}, update {2}/{3}")]
    InsufficientFiatBalance(Sats, Sats, Sats, Sats),
    #[error("New rate cannot fin in the 64 bits. Was {0}/{1}, update {2}/{3}, new rate: {4}")]
    RateOverflow(Sats, Sats, Sats, Sats, i128),
}

impl ChannelHedge {
    pub fn combine(self, other: &ChannelHedge) -> Result<ChannelHedge, HtlcUpdateErr> {
        self.with_htlc(HtlcUpdate {
            channel_id: "".to_owned(),
            sats: other.sats,
            rate: other.rate,
        })
    }

    pub fn with_htlc(self, htlc: HtlcUpdate) -> Result<ChannelHedge, HtlcUpdateErr> {
        if self.sats + htlc.sats < 0 {
            return Err(HtlcUpdateErr::InsufficientSatsBalance(
                self.sats,
                htlc.sats,
                self.sats + htlc.sats,
            ));
        }

        let sats0 = self.sats as i128;
        let sats1 = htlc.sats as i128;
        let rate0 = self.rate as i128;
        let rate1 = htlc.rate as i128;
        let sats = sats0 + sats1;
        let weighted_sum = sats0 * rate1 + sats1 * rate0;
        let rate = if weighted_sum <= 0 {
            return Err(HtlcUpdateErr::InsufficientFiatBalance(
                self.sats, self.rate, htlc.sats, htlc.rate,
            ));
        } else {
            (sats * rate0 * rate1) / weighted_sum
        };

        if rate > i64::MAX as i128 {
            return Err(HtlcUpdateErr::RateOverflow(
                self.sats, self.rate, htlc.sats, htlc.rate, rate,
            ));
        }

        Ok(ChannelHedge {
            sats: sats as i64,
            rate: rate as i64,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct StateSnapshot {
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
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

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 150,
                rate: 1125,
            })
        );
    }

    #[test]
    fn test_weighted_summ_add_02() {
        let hedge = ChannelHedge {
            sats: 100,
            rate: 1000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: 50,
            rate: 1000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 150,
                rate: 1000,
            })
        );
    }

    #[test]
    fn test_weighted_summ_add_03() {
        let hedge = ChannelHedge {
            sats: 0,
            rate: 1000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: 50,
            rate: 2000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 50,
                rate: 2000,
            })
        );
    }

    #[test]
    fn test_weighted_summ_add_04() {
        let hedge = ChannelHedge {
            sats: 100,
            rate: 3000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: 100,
            rate: 1000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 200,
                rate: 1500,
            })
        );
    }

    #[test]
    fn test_weighted_summ_sub_01() {
        let hedge = ChannelHedge {
            sats: 300,
            rate: 3000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -300,
            rate: 4000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(new_hedge, Ok(ChannelHedge { sats: 0, rate: 0 }));
    }

    #[test]
    fn test_weighted_summ_sub_02() {
        let hedge = ChannelHedge {
            sats: 100,
            rate: 3000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -99,
            rate: 3000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 1,
                rate: 3000,
            })
        );
    }

    #[test]
    fn test_weighted_summ_sub_03() {
        let hedge = ChannelHedge {
            sats: 300,
            rate: 3000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -50,
            rate: 1000,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 250,
                rate: 5000,
            })
        );
    }

    #[test]
    fn test_weighted_summ_sub_04() {
        let hedge = ChannelHedge {
            sats: 2,
            rate: 20_000_000_000_000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -1,
            rate: 10_000_000_000_001,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Err(HtlcUpdateErr::RateOverflow(
                2,
                20000000000000,
                -1,
                10000000000001,
                100000000000010000000000000
            ))
        );
    }

    #[test]
    fn test_weighted_summ_sub_05() {
        let hedge = ChannelHedge {
            sats: 2_000_000_000_000,
            rate: 4000,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -1_999_999_999_999,
            rate: 4001,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(new_hedge, Ok(ChannelHedge { sats: 1, rate: 0 }));
    }

    #[test]
    fn test_weighted_summ_sub_06() {
        let hedge = ChannelHedge {
            sats: 2_000_000_000_000,
            rate: 1,
        };
        let upd = HtlcUpdate {
            channel_id: "".to_owned(),
            sats: -999_999_999_999,
            rate: 2,
        };

        let new_hedge = hedge.with_htlc(upd);
        assert_eq!(
            new_hedge,
            Ok(ChannelHedge {
                sats: 1000000000001,
                rate: 0,
            })
        );
    }
}
