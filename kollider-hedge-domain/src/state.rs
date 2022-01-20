use super::update::*;
use chrono::prelude::*;
use futures::Future;
use kollider_api::kollider::api::OrderSide;
use kollider_api::kollider::websocket::data::*;
use log::*;
use rweb::Schema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, Notify};

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema, Clone)]
pub struct State {
    pub last_changed: NaiveDateTime,
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
    pub opened_orders: Vec<KolliderOrder>,
    pub opened_position: Option<KolliderPosition>,
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

impl KolliderOrder {
    pub fn required_margin(&self) -> u64 {
        let real_leverage = self.leverage as f64 / 100.0;
        let real_price = self.price as f64 / 10.0;
        let margin = self.quantity as f64 * (100_000_000.0 / real_price) / real_leverage;
        margin.ceil() as u64
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema, Clone)]
pub struct KolliderPosition {
    liquidation_price: f64,
    leverage: u64,
    entry_value: u64,
    entry_price: u64,
    quantity: u64,
    rpnl: f64,
}

impl std::convert::From<Position> for KolliderPosition {
    fn from(pos: Position) -> Self {
        KolliderPosition {
            liquidation_price: pos.bankruptcy_price,
            leverage: pos.leverage as u64,
            entry_value: pos.entry_value.floor() as u64,
            entry_price: pos.entry_price as u64,
            quantity: pos.quantity as u64,
            rpnl: pos.rpnl,
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
/// How much satoshis we can have unhedged or overhedged. That allows to avoid
/// frequent order opening when channels balances changes by small amount.
pub const ALLOWED_POSITION_GAP: i64 = 100;

impl State {
    pub fn new() -> Self {
        State {
            last_changed: Utc::now().naive_utc(),
            channels_hedge: HashMap::new(),
            opened_orders: vec![],
            opened_position: None,
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

    /// Save information from Kollider WS API, return true fi the state is modified
    pub fn apply_kollider_message(&mut self, msg: KolliderMsg) -> bool {
        if let KolliderMsg::Tagged(tmsg) = msg {
            match tmsg {
                KolliderTaggedMsg::OpenOrders { open_orders } => {
                    self.opened_orders.clear();
                    if let Some(orders) = open_orders.get(HEDGING_SYMBOL) {
                        orders
                            .iter()
                            .for_each(|o| self.opened_orders.push(o.clone().into()));
                        return true;
                    }
                }
                KolliderTaggedMsg::Positions { positions } => {
                    self.opened_position = None;
                    if let Some(position) = positions.get(HEDGING_SYMBOL) {
                        self.opened_position = Some(position.clone().into());
                        return true;
                    }
                }
                _ => (),
            }
        }
        false
    }

    /// Calculate total hedge position across all channels
    pub fn total_hedge(&self) -> Result<ChannelHedge, HtlcUpdateErr> {
        let init_hedge = ChannelHedge { sats: 0, rate: 1 };
        self.channels_hedge
            .iter()
            .try_fold(init_hedge, |acc, (_, h)| acc.combine(h))
    }

    /// Get total amount of sats that we need to hedge at the moment
    pub fn hedge_capacity(&self) -> u64 {
        self.channels_hedge.iter().map(|(_, v)| v.sats as u64).sum()
    }

    /// Get average weighted price over all hedged channels
    pub fn hedge_avg_price(&self) -> Result<u64, HtlcUpdateErr> {
        let final_hedge = self.total_hedge()?;
        assert!(
            final_hedge.rate > 0,
            "Total rate is negative! Rate: {}",
            final_hedge.rate
        );
        Ok(final_hedge.rate as u64)
    }

    /// Get total amount of sats that we request for short positions (buying stables)
    pub fn short_orders(&self) -> u64 {
        self.opened_orders
            .iter()
            .filter(|o| o.side == OrderSide::Ask)
            .map(|o| o.required_margin())
            .sum()
    }

    /// Get total amount of sats that we request for long positions (selling stables)
    pub fn long_orders(&self) -> u64 {
        self.opened_orders
            .iter()
            .filter(|o| o.side == OrderSide::Bid)
            .map(|o| o.required_margin())
            .sum()
    }

    /// Get amount of sats locked in the position
    pub fn position_volume(&self) -> u64 {
        self.opened_position.as_ref().map_or(0, |p| p.entry_value)
    }

    /// Return actions that we need to execute based on current state of service
    ///
    /// TODO: React to situation when we have Bid and Ask orders that negate each other.
    pub fn get_nex_action(&self) -> Result<Vec<StateAction>, NextActionError> {
        let mut actions = vec![];

        trace!("Calculation if we need to open new order");
        let hcap = self.hedge_capacity() as i64;
        let short_orders = self.short_orders() as i64;
        let long_orders = self.long_orders() as i64;
        let avg_price = self.hedge_avg_price()?;
        let pos_volume: i64 = self.position_volume() as i64;
        let pos_short = pos_volume + short_orders;
        let pos_long = pos_volume - long_orders;
        trace!("hcap {} > pos_short {} + gap {}", hcap, pos_short, ALLOWED_POSITION_GAP);
        trace!("hcap {} < pos_long {} - gap {}", hcap, pos_long, ALLOWED_POSITION_GAP);
        if hcap > pos_short + ALLOWED_POSITION_GAP {
            assert!(
                pos_short <= hcap,
                "Sats overflow in order opening: {} <= {}",
                pos_short,
                hcap
            );
            let action = StateAction::OpenOrder {
                sats: (hcap - pos_short) as u64,
                price: avg_price,
                side: OrderSide::Bid,
            };
            actions.push(action);
        } else if hcap < pos_long - ALLOWED_POSITION_GAP {
            assert!(
                hcap <= pos_long,
                "Sats overflow in order opening: {} <= {}",
                hcap,
                pos_long
            );
            let action = StateAction::OpenOrder {
                sats: (pos_long - hcap) as u64,
                price: avg_price,
                side: OrderSide::Ask,
            };
            actions.push(action);
        }

        Ok(actions)
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum StateAction {
    OpenOrder {
        sats: u64,
        price: u64,
        /// Bid for selling sats, Ask for buying sats back
        side: OrderSide,
    },
    CloseOrder {
        order_id: u64,
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum NextActionError {
    #[error("Total hedge position calculation error: {0}")]
    TotalHedge(#[from] HtlcUpdateErr),
}

/// Recalculate actions when state is changed
pub async fn state_action_worker<F, Fut>(state_mx: Arc<Mutex<State>>, state_notify: Arc<Notify>, execute_action: F)
where
    F: Fn(StateAction) -> Fut,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
{
    loop {
        let mactions = {
            let state = state_mx.lock().await;
            state.get_nex_action()
        };
        match mactions {
            Ok(actions) => {
                for action in actions {
                    let res = execute_action(action).await;
                    if let Err(e) = res {
                        log::error!("State action worker failed: {}", e);
                    }
                }
            }
            Err(e) => log::error!("Failed to calculate next state action: {}", e),
        }
        state_notify.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_margin_order() {
        let order = KolliderOrder {
            id: 0,
            leverage: 100,
            price: 500000,
            quantity: 1,
            side: OrderSide::Ask,
        };
        assert_eq!(order.required_margin(), 2000);

        let order = KolliderOrder {
            id: 0,
            leverage: 200,
            price: 500000,
            quantity: 1,
            side: OrderSide::Ask,
        };
        assert_eq!(order.required_margin(), 1000);
    }
}
