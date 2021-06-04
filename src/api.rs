use crate::{
    common::{
        deserialize_integer_or_string_as_decimal, optional_string_serialize, string_serialize,
        Decimal, ExpirationDate, OptionType,
    },
    csv,
    symbol::OptionSymbol,
};

use chrono::{DateTime, FixedOffset, NaiveDate};
use num_rational::Rational64;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

use std::cmp::Ordering;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Response<Data> {
    pub data: Data,
    pub pagination: Option<Pagination>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Pagination {
    pub page_offset: i32,
    pub total_pages: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
pub enum InstrumentType {
    Future,
    Equity,
    Index,
    Cryptocurrency,
    Unknown,
}

pub mod accounts {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Item {
        pub account: Account,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Account {
        pub account_number: String,
    }
}

pub mod watchlists {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Item {
        pub name: String,
        #[serde(rename = "watchlist-entries")]
        pub entries: Vec<Entry>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Entry {
        pub symbol: String,
        #[serde(alias = "instrument-type")] // appears as both kebab and snake case
        pub instrument_type: Option<InstrumentType>,
    }
}

pub mod market_metrics {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Item {
        pub symbol: String,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility_index: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility_index_5_day_change: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility_index_rank: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub tos_implied_volatility_index_rank: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub tw_implied_volatility_index_rank: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub tos_implied_volatility_index_rank_updated_at: Option<DateTime<FixedOffset>>,
        pub implied_volatility_index_rank_source: Option<String>,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility_percentile: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility_updated_at: Option<DateTime<FixedOffset>>,
        #[serde(default, with = "optional_string_serialize")]
        pub liquidity_value: Option<f64>,
        #[serde(default, with = "optional_string_serialize")]
        pub liquidity_rank: Option<f64>,
        pub liquidity_rating: Option<i32>,
        #[serde(with = "string_serialize")]
        pub updated_at: DateTime<FixedOffset>,
        pub option_expiration_implied_volatilities: Option<Vec<ExpirationImpliedVolatility>>,
        pub earnings: Option<Earnings>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct ExpirationImpliedVolatility {
        #[serde(with = "string_serialize")]
        pub expiration_date: ExpirationDate,
        #[serde(default, with = "optional_string_serialize")]
        pub implied_volatility: Option<f64>,
    }

    impl options_common::ExpirationImpliedVolatilityProvider for market_metrics::Item {
        fn find_iv_for_expiration_date(&self, date: ExpirationDate) -> Option<f64> {
            self.option_expiration_implied_volatilities
                .as_ref()?
                .iter()
                .find(|eiv| eiv.expiration_date == date)?
                .implied_volatility
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Earnings {
        pub expected_report_date: NaiveDate,
        pub estimated: bool,
        pub time_of_day: Option<EarningsTimeOfDay>,
    }

    impl PartialOrd for Earnings {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Earnings {
        fn cmp(&self, other: &Self) -> Ordering {
            self.expected_report_date
                .cmp(&other.expected_report_date)
                .then(self.time_of_day.cmp(&other.time_of_day))
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Hash)]
    pub enum EarningsTimeOfDay {
        BTO,
        AMC,
    }
}

pub mod positions {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Item {
        pub symbol: String,
        #[serde(
            deserialize_with = "deserialize_integer_or_string_as_decimal",
            serialize_with = "string_serialize::serialize"
        )]
        pub quantity: Decimal,
        pub quantity_direction: QuantityDirection,
        pub instrument_type: String,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
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

        pub fn expiration_date(&self) -> ExpirationDate {
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
                quantity: Decimal(Rational64::from_integer(csv.quantity.abs().into())),
                quantity_direction: QuantityDirection::from_signed_quantity(csv.quantity),
                instrument_type: match csv.instrument_type.as_ref() {
                    // TODO: handle futures and futures options
                    "OPTION" => "Equity Option".to_string(),
                    "STOCK" => "Equity".to_string(),
                    _ => unreachable!("Unhandled instrument type: {}", csv.instrument_type),
                },
            }
        }
    }
}

pub mod transactions {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
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

    #[derive(Clone, Debug, Serialize, Deserialize)]
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
        pub ext_global_order_number: Option<u32>, // not present for crypto trades
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

        pub fn expiration_date(&self) -> ExpirationDate {
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

    #[derive(Clone, Debug, Serialize, Deserialize)]
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
        value_effect: ValueEffect,
        #[serde(with = "string_serialize")]
        pub quantity: Decimal,
        #[serde(with = "optional_string_serialize")]
        clearing_fees: Option<Decimal>,
        clearing_fees_effect: Option<ValueEffect>,
        #[serde(with = "optional_string_serialize")]
        regulatory_fees: Option<Decimal>,
        regulatory_fees_effect: Option<ValueEffect>,
        #[serde(with = "optional_string_serialize")]
        proprietary_index_option_fees: Option<Decimal>,
        proprietary_index_option_fees_effect: Option<ValueEffect>,
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

        pub fn fees(&self) -> Rational64 {
            self.clearing_fees_effect
                .map(|v| v.apply(self.clearing_fees.unwrap().0))
                .unwrap_or_else(|| Rational64::zero())
                + self
                    .regulatory_fees_effect
                    .map(|v| v.apply(self.regulatory_fees.unwrap().0))
                    .unwrap_or_else(|| Rational64::zero())
                + self
                    .proprietary_index_option_fees_effect
                    .map(|v| v.apply(self.proprietary_index_option_fees.unwrap().0))
                    .unwrap_or_else(|| Rational64::zero())
        }

        pub fn expiration_date(&self) -> ExpirationDate {
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

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct OtherItem {
        pub id: u32,
        pub transaction_type: String,
        #[serde(with = "string_serialize")]
        pub executed_at: DateTime<FixedOffset>,
        #[serde(with = "string_serialize")]
        value: Decimal,
        value_effect: ValueEffect,
    }

    impl PartialEq for OtherItem {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for OtherItem {}

    impl OtherItem {
        pub fn value(&self) -> Rational64 {
            self.value_effect.apply(self.value.0)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
    pub enum TradeAction {
        Sell,
        Buy,
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
                TradeAction::Sell => TradeAction::Buy,
                TradeAction::Buy => TradeAction::Sell,
                TradeAction::SellToOpen => TradeAction::BuyToClose,
                TradeAction::BuyToOpen => TradeAction::SellToClose,
                TradeAction::SellToClose => TradeAction::BuyToOpen,
                TradeAction::BuyToClose => TradeAction::SellToOpen,
            }
        }

        pub fn opens(&self) -> bool {
            match self {
                TradeAction::Sell => true,
                TradeAction::Buy => true,
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

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
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
                    ext_global_order_number: Some(0),
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
                    clearing_fees: Some(split_fees),
                    clearing_fees_effect: Some(fees_effect),
                    regulatory_fees: Some(split_fees),
                    regulatory_fees_effect: Some(fees_effect),
                    proprietary_index_option_fees: Some(split_fees),
                    proprietary_index_option_fees_effect: Some(fees_effect),
                })
            } else {
                Item::Other(OtherItem {
                    id: 0,
                    transaction_type: csv.trade_type,
                    executed_at: csv.date,
                    value: csv.value.abs(),
                    value_effect: ValueEffect::from_value(csv.value.0),
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

pub mod option_chains {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Response {
        pub items: Vec<Item>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Item {
        pub underlying_symbol: String,
        pub root_symbol: String,
        pub option_chain_type: String,
        pub shares_per_contract: i32,
        pub deliverables: Vec<Deliverable>,
        pub expirations: Vec<Expiration>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct TickSize {
        #[serde(with = "string_serialize")]
        pub value: Decimal,
        #[serde(default, with = "optional_string_serialize")]
        pub threshold: Option<Decimal>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Deliverable {
        pub symbol: Option<String>,
        pub root_symbol: String,
        pub deliverable_type: String,
        pub description: String,
        #[serde(with = "string_serialize")]
        pub amount: Decimal,
        pub instrument_type: Option<InstrumentType>,
        #[serde(with = "string_serialize")]
        pub percent: i32,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct Expiration {
        pub expiration_type: ExpirationType,
        #[serde(with = "string_serialize")]
        pub expiration_date: ExpirationDate,
        pub days_to_expiration: i32,
        pub settlement_type: String,
        #[serde(default)] // strikes property not always present
        pub strikes: Vec<ExpirationStrike>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
    pub enum ExpirationType {
        Regular,
        Weekly,
        Quarterly,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub struct ExpirationStrike {
        #[serde(with = "string_serialize")]
        pub strike_price: Decimal,
        pub call: String,
        pub put: String,
    }
}
