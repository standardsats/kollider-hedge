use super::update::*;
use chrono::prelude::*;
use kollider_api::kollider::websocket::data::*;
use kollider_api::kollider::api::OrderSide;
use rweb::Schema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema, Clone)]
pub struct State {
    pub last_changed: NaiveDateTime,
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
    pub opened_orders: Vec<KolliderOrder>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema, Clone)]
pub struct KolliderOrder {
    id: u64,
    leverage: u64,
    price: u64,
    quantity: u64,
    side: OrderSide,
}

impl std::convert::From<OpenOrder> for KolliderOrder {
    fn from(order: OpenOrder) -> Self {
        KolliderOrder {
            id: order.order_id,
            leverage: order.leverage,
            price: order.price,
            quantity: order.quantity,
            side: order.side,
        }
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum StateUpdateErr {
    #[error("State update error: {0}")]
    Htlc(#[from] HtlcUpdateErr),
}

impl rweb::reject::Reject for StateUpdateErr {}

pub const HEDGING_SYMBOL: &str = "BTCUSD.PERP";

impl State {
    pub fn new() -> Self {
        State {
            last_changed: Utc::now().naive_utc(),
            channels_hedge: HashMap::new(),
            opened_orders: vec![],
        }
    }

    pub fn apply_update(&mut self, update: StateUpdate) -> Result<(), StateUpdateErr> {
        match update.body {
            UpdateBody::Htlc(htlc) => {
                self.with_htlc(htlc)?;
                self.last_changed = update.created;
                Ok(())
            }
            UpdateBody::Snapshot(snaphsot) => {
                self.channels_hedge = snaphsot.channels_hedge;
                self.last_changed = update.created;
                Ok(())
            }
        }
    }

    fn with_htlc(&mut self, htlc: HtlcUpdate) -> Result<(), HtlcUpdateErr> {
        let chan_id = htlc.channel_id.clone();
        let new_chan = if let Some(chan) = self.channels_hedge.get(&chan_id) {
            chan.clone().with_htlc(htlc)?
        } else {
            ChannelHedge {
                sats: htlc.sats,
                rate: htlc.rate,
            }
        };
        self.channels_hedge.insert(chan_id, new_chan);

        Ok(())
    }

    /// Take ordered chain of updates and collect the accumulated state.
    /// Order should be from the earliest to the latest.
    pub fn collect<I>(updates: I) -> Result<Self, StateUpdateErr>
    where
        I: IntoIterator<Item = StateUpdate>,
    {
        let mut state = State::new();
        for upd in updates.into_iter() {
            state.apply_update(upd)?;
        }
        Ok(state)
    }

    /// Save information from Kollider WS API
    pub fn apply_kollider_message(&mut self, msg: KolliderMsg) {
        if let KolliderMsg::Tagged(tmsg) = msg {
            match tmsg {
                KolliderTaggedMsg::OpenOrders { open_orders } => {
                    if let Some(orders) = open_orders.get(HEDGING_SYMBOL) {
                        orders.iter().for_each(|o| self.appy_open_order(o));
                    }
                }
                KolliderTaggedMsg::Positions { positions } => {
                    if let Some(position) = positions.get(HEDGING_SYMBOL) {
                        self.appy_open_position(position);
                    }
                }
                _ => (),
            }
        }
    }

    fn appy_open_order(&mut self, order: &OpenOrder) {

    }

    fn appy_open_position(&mut self, position: &Position) {

    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}
