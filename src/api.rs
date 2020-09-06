use crate::{csv, symbol::OptionSymbol, common::{optional_string_serialize, string_serialize, Date, Decimal, OptionType}};

use chrono::{DateTime, FixedOffset};
use num_rational::Rational64;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Response<Data> {
    pub data: Data,
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Pagination {
    pub page_offset: i32,
    pub total_pages: i32,
}

pub mod accounts {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Item {
        pub account: Account,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Account {
        pub account_number: String,
    }
}

pub mod watchlists {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Item {
        pub name: String,
        #[serde(rename = "watchlist-entries")]
        pub entries: Vec<Entry>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Entry {
        pub symbol: String,
        #[serde(alias = "instrument-type")]
        pub instrument_type: InstrumentType,
    }

    #[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
    pub enum InstrumentType {
        Future,
        Equity,
        Index,
        Unknown,
    }
}

pub mod market_metrics {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Item {
        pub symbol: String,
        implied_volatility_index: String,
        implied_volatility_index_5_day_change: String,
        implied_volatility_index_rank: Option<String>,
        tos_implied_volatility_index_rank: Option<String>,
        tw_implied_volatility_index_rank: String,
        tos_implied_volatility_index_rank_updated_at: Option<String>,
        implied_volatility_index_rank_source: Option<String>,
        implied_volatility_percentile: String,
        implied_volatility_updated_at: Option<String>,
        liquidity_value: Option<String>,
        liquidity_rank: Option<String>,
        liquidity_rating: i32,
        updated_at: String,
        pub option_expiration_implied_volatilities: Option<Vec<ExpirationImpliedVolatility>>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct ExpirationImpliedVolatility {
        #[serde(with = "string_serialize")]
        pub expiration_date: Date,
        option_chain_type: String,
        settlement_type: Option<String>,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility: Option<f64>,
    }
}

pub mod positions {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Item {
        pub symbol: String,
        pub quantity: i32,
        pub quantity_direction: QuantityDirection,
    }

    #[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
    pub enum QuantityDirection {
        Short,
        Long,
    }

    impl QuantityDirection {
        fn from_signed_quantity(quantity: i32) -> Self {
            if quantity > 0 {
                Self::Long
            } else {
                Self::Short
            }
        }
    }

    impl Item {
        pub fn quote_symbol(&self) -> String {
            OptionSymbol::from(&self.symbol).quote_symbol()
        }

        pub fn expiration_date(&self) -> Date {
            OptionSymbol::from(&self.symbol).expiration_date()
        }

        pub fn underlying_symbol(&self) -> &str {
            OptionSymbol::from(&self.symbol).underlying_symbol()
        }

        pub fn option_type(&self) -> OptionType {
            OptionSymbol::from(&self.symbol).option_type()
        }

        pub fn strike_price(&self) -> Rational64 {
            OptionSymbol::from(&self.symbol).strike_price()
        }
    }

    impl From<csv::Position> for Item {
        fn from(csv: csv::Position) -> Self {
            Self {
                symbol: csv.symbol,
                quantity: csv.quantity.abs(),
                quantity_direction: QuantityDirection::from_signed_quantity(csv.quantity),
            }
        }
    }
}

pub mod transactions {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum Item {
        Trade(TradeItem),
        ReceiveDeliver(ReceiveDeliverItem),
        Other(OtherItem),
    }

    impl Item {
        pub fn id_mut(&mut self) -> &mut u32 {
            match self {
                Self::Trade(item) => &mut item.id,
                Self::ReceiveDeliver(item) => &mut item.id,
                Self::Other(item) => &mut item.id,
            }
        }

        pub fn executed_at(&mut self) -> DateTime<FixedOffset> {
            match self {
                Self::Trade(item) => item.executed_at,
                Self::ReceiveDeliver(item) => item.executed_at,
                Self::Other(item) => item.executed_at,
            }
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct TradeItem {
        pub id: u32,
        pub symbol: String,
        pub instrument_type: String,
        pub transaction_type: String,
        #[serde(with = "string_serialize")]
        pub executed_at: DateTime<FixedOffset>,
        pub action: TradeAction,
        pub underlying_symbol: String,
        #[serde(with = "string_serialize")]
        value: Decimal,
        value_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        pub quantity: Decimal,
        #[serde(with = "string_serialize")]
        commission: Decimal,
        commission_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        clearing_fees: Decimal,
        clearing_fees_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        regulatory_fees: Decimal,
        regulatory_fees_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        proprietary_index_option_fees: Decimal,
        proprietary_index_option_fees_effect: ValueEffect,
        pub ext_global_order_number: u32,
    }

    impl PartialEq for TradeItem {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for TradeItem {}

    impl TradeItem {
        pub fn value(&self) -> Rational64 {
            self.value_effect.apply(self.value.0)
        }

        pub fn commission(&self) -> Rational64 {
            self.commission_effect.apply(self.commission.0)
        }

        pub fn fees(&self) -> Rational64 {
            self.clearing_fees_effect.apply(self.clearing_fees.0)
                + self.regulatory_fees_effect.apply(self.regulatory_fees.0)
                + self
                    .proprietary_index_option_fees_effect
                    .apply(self.proprietary_index_option_fees.0)
        }

        pub fn expiration_date(&self) -> Date {
            OptionSymbol::from(&self.symbol).expiration_date()
        }

        pub fn underlying_symbol(&self) -> &str {
            OptionSymbol::from(&self.symbol).underlying_symbol()
        }

        pub fn option_type(&self) -> OptionType {
            OptionSymbol::from(&self.symbol).option_type()
        }

        pub fn strike_price(&self) -> Rational64 {
            OptionSymbol::from(&self.symbol).strike_price()
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct ReceiveDeliverItem {
        pub id: u32,
        pub symbol: String,
        pub instrument_type: String,
        pub transaction_type: String,
        pub transaction_sub_type: String,
        #[serde(with = "string_serialize")]
        pub executed_at: DateTime<FixedOffset>,
        #[serde(default)]
        pub action: Option<TradeAction>, // defined for stock splits, missing for exercise
        pub underlying_symbol: String,
        #[serde(with = "string_serialize")]
        value: Decimal,
        pub value_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        pub quantity: Decimal,
    }

    impl PartialEq for ReceiveDeliverItem {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for ReceiveDeliverItem {}

    impl ReceiveDeliverItem {
        pub fn value(&self) -> Rational64 {
            self.value_effect.apply(self.value.0)
        }

        pub fn expiration_date(&self) -> Date {
            OptionSymbol::from(&self.symbol).expiration_date()
        }

        pub fn underlying_symbol(&self) -> &str {
            OptionSymbol::from(&self.symbol).underlying_symbol()
        }

        pub fn option_type(&self) -> OptionType {
            OptionSymbol::from(&self.symbol).option_type()
        }

        pub fn strike_price(&self) -> Rational64 {
            OptionSymbol::from(&self.symbol).strike_price()
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct OtherItem {
        pub id: u32,
        pub transaction_type: String,
        #[serde(with = "string_serialize")]
        pub executed_at: DateTime<FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
    pub enum TradeAction {
        #[serde(rename = "Sell to Open")]
        SellToOpen, // open credit
        #[serde(rename = "Buy to Open")]
        BuyToOpen, // open debit
        #[serde(rename = "Sell to Close")]
        SellToClose, // close credit
        #[serde(rename = "Buy to Close")]
        BuyToClose, // close debit
    }

    impl TradeAction {
        pub fn opposing_action(&self) -> Self {
            match self {
                TradeAction::SellToOpen => TradeAction::BuyToClose,
                TradeAction::BuyToOpen => TradeAction::SellToClose,
                TradeAction::SellToClose => TradeAction::BuyToOpen,
                TradeAction::BuyToClose => TradeAction::SellToOpen,
            }
        }

        pub fn opens(&self) -> bool {
            match self {
                TradeAction::SellToOpen => true,
                TradeAction::BuyToOpen => true,
                TradeAction::SellToClose => false,
                TradeAction::BuyToClose => false,
            }
        }

        pub fn closes(&self) -> bool {
            !self.opens()
        }
    }

    #[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
    pub enum ValueEffect {
        None,
        Debit,
        Credit,
    }

    impl ValueEffect {
        fn from_value(value: Rational64) -> Self {
            if value.is_positive() {
                Self::Credit
            } else if value.is_negative() {
                Self::Debit
            } else {
                Self::None
            }
        }

        fn apply(&self, v: Rational64) -> Rational64 {
            match self {
                Self::None => Rational64::zero(),
                Self::Debit => -v,
                Self::Credit => v,
            }
        }
    }

    impl From<csv::Transaction> for Item {
        fn from(csv: csv::Transaction) -> Self {
            let symbol = csv.symbol.clone().unwrap_or_default();
            let instrument_type = csv.instrument_type.clone().unwrap_or_default();
            let underlying_symbol = csv.underlying_symbol().unwrap_or_default().to_string();

            let split_fees = Decimal(csv.fees.abs().0 / 3);
            let fees_effect = ValueEffect::from_value(csv.fees.0);

            if csv.trade_type == "Trade" {
                let commission = csv.commissions.expect("Missing commissions").abs();
                Item::Trade(TradeItem {
                    id: 0,
                    symbol,
                    instrument_type,
                    transaction_type: csv.trade_type,
                    executed_at: csv.date,
                    action: csv.action.expect("Missing trade action").into(),
                    underlying_symbol,
                    value: csv.value.abs(),
                    value_effect: ValueEffect::from_value(csv.value.0),
                    quantity: csv.quantity,
                    commission,
                    commission_effect: ValueEffect::from_value(commission.0),
                    clearing_fees: split_fees,
                    clearing_fees_effect: fees_effect,
                    regulatory_fees: split_fees,
                    regulatory_fees_effect: fees_effect,
                    proprietary_index_option_fees: split_fees,
                    proprietary_index_option_fees_effect: fees_effect,
                    ext_global_order_number: 0,
                })
            } else if csv.trade_type == "Receive Deliver" {
                let description = csv.description.to_ascii_lowercase();
                let transaction_sub_type = if description.contains("exercise") {
                    "Exercise".to_string()
                } else if description.contains("expiration") {
                    "Expiration".to_string()
                } else if description.contains("assignment") {
                    "Assignment".to_string()
                } else if description.contains("forward split") {
                    "Forward Split".to_string()
                } else if description.contains("backwards split") {
                    "Backwards Split".to_string()
                } else {
                    description
                };

                Item::ReceiveDeliver(ReceiveDeliverItem {
                    id: 0,
                    symbol,
                    instrument_type,
                    transaction_type: csv.trade_type,
                    transaction_sub_type,
                    executed_at: csv.date,
                    action: csv.action.map(|action| action.into()),
                    underlying_symbol,
                    value: csv.value.abs(),
                    value_effect: ValueEffect::from_value(csv.value.0),
                    quantity: csv.quantity,
                })
            } else {
                Item::Other(OtherItem {
                    id: 0,
                    transaction_type: csv.trade_type,
                    executed_at: csv.date,
                })
            }
        }
    }

    impl From<csv::TradeAction> for TradeAction {
        fn from(csv: csv::TradeAction) -> Self {
            match csv {
                csv::TradeAction::SellToOpen => Self::SellToOpen,
                csv::TradeAction::BuyToOpen => Self::BuyToOpen,
                csv::TradeAction::SellToClose => Self::SellToClose,
                csv::TradeAction::BuyToClose => Self::BuyToClose,
            }
        }
    }
}