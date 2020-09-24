use crate::{
    common::{optional_string_serialize, string_serialize, Date, Decimal, OptionType},
    symbol::{self, OptionSymbol},
};

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;

use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct Position {
    #[serde(rename = "Symbol")]
    pub symbol: String,
    #[serde(rename = "Type")]
    pub instrument_type: String,
    #[serde(rename = "Quantity", with = "string_serialize")]
    pub quantity: i32,
    #[serde(rename = "Strike Price", with = "string_serialize")]
    pub strike_price: Decimal,
    #[serde(rename = "Call/Put")]
    pub call_or_put: OptionTypePascalCase,
    #[serde(rename = "D's Opn")]
    pub days_open: String,
    #[serde(rename = "NetLiq", with = "string_serialize")]
    pub net_liq: Decimal,
}

impl Position {
    pub fn expiration_date(&self) -> Date {
        OptionSymbol::from(&self.symbol).expiration_date()
    }

    pub fn underlying_symbol(&self) -> &str {
        OptionSymbol::from(&self.symbol).underlying_symbol()
    }

    pub fn days_open(&self) -> i32 {
        let idx = self.days_open.len() - 1;

        assert!(self.days_open.chars().nth(idx) == Some('d'));
        self.days_open
            .get(..idx)
            .map(|s| i32::from_str(s).ok())
            .flatten()
            .unwrap_or_else(|| panic!("Could not parse days open: {}", self.days_open))
    }
}

#[derive(Debug, Deserialize)]
pub struct Transaction {
    #[serde(rename = "Date", with = "string_serialize")]
    pub date: DateTime<FixedOffset>,
    #[serde(rename = "Type")]
    pub trade_type: String,
    #[serde(rename = "Action")]
    pub action: Option<TradeAction>,
    #[serde(rename = "Symbol")]
    pub symbol: Option<String>,
    #[serde(rename = "Instrument Type")]
    pub instrument_type: Option<String>,
    #[serde(rename = "Description")]
    pub description: String,
    #[serde(rename = "Value", with = "string_serialize")]
    pub value: Decimal,
    #[serde(rename = "Quantity", with = "string_serialize")]
    pub quantity: Decimal,
    #[serde(rename = "Average Price", with = "optional_string_serialize")]
    pub average_price: Option<Decimal>,
    #[serde(rename = "Commissions", with = "optional_string_serialize")]
    pub commissions: Option<Decimal>,
    #[serde(rename = "Fees", with = "string_serialize")]
    pub fees: Decimal,
    #[serde(rename = "Multiplier", with = "optional_string_serialize")]
    pub multiplier: Option<i32>,
    // pub underlying_symbol: Option<String>,
    #[serde(rename = "Expiration Date", with = "optional_string_serialize")]
    pub expiration_date: Option<TransactionExpiration>,
    #[serde(rename = "Strike Price", with = "optional_string_serialize")]
    pub strike_price: Option<Decimal>,
    #[serde(rename = "Call or Put")]
    pub call_or_put: Option<OptionTypeUpperCase>,
}

impl Transaction {
    pub fn underlying_symbol(&self) -> Option<&str> {
        self.symbol.as_ref().map(|symbol| {
            let underlying_symbol = symbol
                .split_whitespace()
                .next()
                .expect("Missing underlying symbol");
            symbol::strip_weekly(underlying_symbol)
        })
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradeAction {
    SellToOpen,  // open credit
    BuyToOpen,   // open debit
    SellToClose, // close credit
    BuyToClose,  // close debit
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionExpiration(pub NaiveDate);

impl FromStr for TransactionExpiration {
    type Err = chrono::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(TransactionExpiration(NaiveDate::parse_from_str(
            s,
            "%-m/%-d/%y",
        )?))
    }
}

impl From<TransactionExpiration> for Date {
    fn from(d: TransactionExpiration) -> Date {
        Date(d.0)
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OptionTypeUpperCase {
    Call,
    Put,
}

impl From<OptionTypeUpperCase> for OptionType {
    fn from(json: OptionTypeUpperCase) -> OptionType {
        match json {
            OptionTypeUpperCase::Call => OptionType::Call,
            OptionTypeUpperCase::Put => OptionType::Put,
        }
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum OptionTypePascalCase {
    Call,
    Put,
}

impl From<OptionTypePascalCase> for OptionType {
    fn from(json: OptionTypePascalCase) -> OptionType {
        match json {
            OptionTypePascalCase::Call => OptionType::Call,
            OptionTypePascalCase::Put => OptionType::Put,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_expiration_date_csv_from_str() {
        assert_eq!(
            TransactionExpiration::from_str("7/31/20").unwrap(),
            TransactionExpiration(NaiveDate::from_ymd(2020, 7, 31))
        );
    }
}
