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
    /// Balance in BTC on Kollider
    pub balance: Option<f64>,
    /// Price of BTC/USD reported by Kollider
    pub ticker: Option<f64>,
    pub channels_hedge: HashMap<ChannelId, ChannelHedge>,
    pub opened_orders: Option<Vec<KolliderOrder>>,
    pub opened_position: Option<KolliderPosition>,
    // TODO: put orders in progress of opening here
    /// Cache actions that we need to execute to avoid replaying them before they are completed
    pub scheduled_actions: Vec<StateAction>,
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
pub const INDEX_BTC_USD: &str = ".BTCUSD";

/// How much USD we can have unhedged or overhedged. That allows to avoid
/// frequent order opening when channels balances changes by small amount.
pub const ALLOWED_POSITION_GAP: i64 = 1;

impl State {
    pub fn new() -> Self {
        State {
            last_changed: Utc::now().naive_utc(),
            balance: None,
            ticker: None,
            channels_hedge: HashMap::new(),
            opened_orders: None,
            opened_position: None,
            scheduled_actions: vec![],
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
                    if let Some(orders) = open_orders.get(HEDGING_SYMBOL) {
                        let mut res = vec![];
                        orders.iter().for_each(|o| res.push(o.clone().into()));

                        self.opened_orders = Some(res);
                        return true;
                    } else {
                        self.opened_orders = Some(vec![]);
                    }
                }
                KolliderTaggedMsg::Positions { positions } => {
                    if let Some(position) = positions.get(HEDGING_SYMBOL) {
                        self.opened_position = Some(position.clone().into());
                        return true;
                    } else {
                        self.opened_position = Some(KolliderPosition {
                            liquidation_price: 0.0,
                            leverage: 100,
                            entry_value: 0,
                            entry_price: 0,
                            quantity: 0,
                            rpnl: 0.0,
                        });
                        return true;
                    }
                }
                KolliderTaggedMsg::Open {
                    symbol,
                    order_id,
                    leverage,
                    price,
                    quantity,
                    side,
                    ..
                } if symbol == HEDGING_SYMBOL => {
                    let order = KolliderOrder {
                        id: order_id,
                        leverage,
                        price,
                        quantity,
                        side,
                    };
                    if let Some(orders) = &mut self.opened_orders {
                        orders.push(order);
                    } else {
                        self.opened_orders = Some(vec![order]);
                    }

                    return true;
                }
                KolliderTaggedMsg::Balances { cash, .. } => {
                    self.balance = Some(cash);
                    return true;
                }
                KolliderTaggedMsg::IndexValues(IndexValue { symbol, value, .. })
                    if symbol == INDEX_BTC_USD =>
                {
                    self.ticker = Some(value);
                    return true;
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
    pub fn short_orders(&self) -> Option<u64> {
        self.opened_orders.as_ref().map(|orders| {
            orders
                .iter()
                .filter(|o| o.side == OrderSide::Ask)
                .map(|o| o.required_margin())
                .sum()
        })
    }

    /// Get total amount of sats that we request for long positions (selling stables)
    pub fn long_orders(&self) -> Option<u64> {
        self.opened_orders.as_ref().map(|orders| {
            orders
                .iter()
                .filter(|o| o.side == OrderSide::Bid)
                .map(|o| o.required_margin())
                .sum()
        })
    }

    /// Get total amount of sats we are going to place into short position (buying stable)
    pub fn scheduled_shorts(&self) -> u64 {
        self.scheduled_actions
            .iter()
            .filter(|a| a.is_short_order())
            .filter_map(|a| a.order_sats())
            .sum()
    }

    /// Get total amount of sats we are going to place into long position (selling stables)
    pub fn scheduled_longs(&self) -> u64 {
        self.scheduled_actions
            .iter()
            .filter(|a| a.is_long_order())
            .filter_map(|a| a.order_sats())
            .sum()
    }

    /// Get amount of sats locked in the position
    pub fn position_volume(&self) -> u64 {
        self.opened_position.as_ref().map_or(0, |p| p.entry_value)
    }

    /// Return actions that we need to execute based on current state of service
    ///
    /// TODO: React to situation when we have Bid and Ask orders that negate each other.
    pub fn calculate_next_actions(&mut self) -> Result<(), NextActionError> {
        trace!("Calculation if we need to open new order");
        if let (Some(short_orders), Some(long_orders)) = (self.short_orders(), self.long_orders()) {
            let hcap = self.hedge_capacity() as i64;
            let scheduled_shorts = self.scheduled_shorts() as i64;
            let scheduled_longs = self.scheduled_longs() as i64;
            let avg_price = self.hedge_avg_price()?;
            let pos_volume: i64 = self.position_volume() as i64;
            let pos_short = pos_volume + short_orders as i64 + scheduled_shorts;
            let pos_long = pos_volume - long_orders as i64 - scheduled_longs;
            let gap = ALLOWED_POSITION_GAP * avg_price as i64;
            trace!("hcap {} > pos_short {} + gap {}", hcap, pos_short, gap);
            trace!("hcap {} < pos_long {} - gap {}", hcap, pos_long, gap);
            if hcap > pos_short + gap {
                debug!(
                    "Decided to open short position as hcap {} > pos_short {} + gap {}",
                    hcap, pos_short, gap
                );
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
                self.scheduled_actions.push(action);
            } else if hcap < pos_long - gap {
                debug!(
                    "Decided to close position as hcap {} < pos_long {} - gap {}",
                    hcap, pos_long, gap
                );
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
                self.scheduled_actions.push(action);
            }
        }

        Ok(())
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

#[derive(Debug, Serialize, Deserialize, Schema, PartialEq, Clone)]
pub enum StateAction {
    OpenOrder {
        sats: u64,
        price: u64,
        /// Bid for selling sats, Ask for buying sats back
        side: OrderSide,
    },
    CloseOrder {
        order_id: u64,
    },
}

impl StateAction {
    /// Buying stable, selling sats
    pub fn is_short_order(&self) -> bool {
        match self {
            StateAction::OpenOrder { side, .. } => *side == OrderSide::Bid,
            _ => false,
        }
    }

    /// Selling stable, buying sats
    pub fn is_long_order(&self) -> bool {
        match self {
            StateAction::OpenOrder { side, .. } => *side == OrderSide::Ask,
            _ => false,
        }
    }

    /// Get amount of sats in opening order if the action is open order
    pub fn order_sats(&self) -> Option<u64> {
        match self {
            StateAction::OpenOrder { sats, .. } => Some(*sats),
            _ => None,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum NextActionError {
    #[error("Total hedge position calculation error: {0}")]
    TotalHedge(#[from] HtlcUpdateErr),
}

/// Recalculate actions when state is changed
pub async fn state_action_worker<F, Fut>(
    state_mx: Arc<Mutex<State>>,
    state_notify: Arc<Notify>,
    execute_action: F,
) where
    F: Fn(StateAction) -> Fut,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
{
    loop {
        {
            let mut state = state_mx.lock().await;
            let mactions = state
                .calculate_next_actions();
            debug!("Scheduled actions {:?}", state.scheduled_actions);
            match mactions {
                Ok(_) => {
                    for action in state.scheduled_actions.iter() {
                        let res = execute_action(action.clone()).await;
                        if let Err(e) = res {
                            log::error!("State action worker failed: {}", e);
                        }
                    }
                    state.scheduled_actions = vec![];
                }
                Err(e) => log::error!("Failed to calculate next state action: {}", e),
            }
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
