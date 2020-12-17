use crate::common::{Date, OptionType};

use chrono::NaiveDate;
use num_rational::Rational64;

use std::fmt;
use std::str::FromStr;

pub struct OptionSymbol<'a>(&'a str);

impl<'a> OptionSymbol<'a> {
    pub fn from(s: &'a str) -> OptionSymbol<'a> {
        OptionSymbol(s)
    }

    pub fn quote_symbol(&self) -> String {
        let price = self.price_component();
        let integer = price[..5].trim_start_matches('0');
        let decimal = price[5..].trim_end_matches('0');
        format!(
            ".{}{}{}{}{}{}",
            self.underlying_symbol(),
            self.date_component(),
            self.option_type_component(),
            integer,
            if decimal.is_empty() { "" } else { "." },
            decimal,
        )
    }

    fn date_component(&self) -> &str {
        let component = self.0.split_whitespace().nth(1);
        let date = component.and_then(|c| c.get(..6));
        date.unwrap_or_else(|| panic!("Missing date component for symbol: {}", self.0))
    }

    fn option_type_component(&self) -> char {
        self.0
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.chars().nth(6))
            .unwrap_or_else(|| panic!("Missing option type component for symbol: {}", self.0))
    }

    fn price_component(&self) -> &str {
        let component = self.0.split_whitespace().nth(1);
        let price = component.and_then(|c| c.get(7..));
        price.unwrap_or_else(|| panic!("Missing price component for symbol: {}", self.0))
    }

    pub fn expiration_date(&self) -> Date {
        let date_str = self.date_component();
        let date = NaiveDate::parse_from_str(date_str, "%y%m%d").ok().map(Date);
        date.unwrap_or_else(|| panic!("Missing expiration date for symbol: {}", self.0))
    }

    pub fn underlying_symbol(&self) -> &'a str {
        let underlying_symbol = self
            .0
            .split_whitespace()
            .next()
            .unwrap_or_else(|| panic!("Missing underlying symbol for symbol: {}", self.0));
        strip_weekly(underlying_symbol)
    }

    pub fn option_type(&self) -> OptionType {
        match self.option_type_component() {
            'P' => OptionType::Put,
            'C' => OptionType::Call,
            _ => unreachable!("Missing option type for symbol: {}", self.0),
        }
    }

    pub fn strike_price(&self) -> Rational64 {
        let price_str = self.price_component();
        let price = i64::from_str(price_str)
            .ok()
            .map(|i| Rational64::new(i, 1000));
        price.unwrap_or_else(|| panic!("Missing strike price for symbol: {}", self.0))
    }
}

impl fmt::Display for OptionSymbol<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub struct QuoteSymbol<'a>(&'a str);

impl<'a> QuoteSymbol<'a> {
    pub fn from(s: &'a str) -> QuoteSymbol<'a> {
        QuoteSymbol(s)
    }

    pub fn matches_underlying_symbol(&self, underlying_symbol: &str) -> bool {
        self.0
            .get(1..)
            .filter(|s| s.starts_with(underlying_symbol))
            .is_some()
            && self
                .0
                .chars()
                .nth(underlying_symbol.len() + 1)
                .filter(|c| !c.is_numeric())
                .is_none()
    }
}

impl fmt::Display for QuoteSymbol<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub fn strip_weekly(underlying_symbol: &str) -> &str {
    if underlying_symbol == "SPXW" {
        &underlying_symbol[0..3]
    } else {
        underlying_symbol
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_option_symbol_quote_symbol() {
        let quote_symbol = OptionSymbol::from("IQ 200918P00017500").quote_symbol();
        assert_eq!(quote_symbol, ".IQ200918P17.5");
    }

    #[test]
    fn test_option_symbol_option_type() {
        let option_type = OptionSymbol::from("IQ 200918P00017500").option_type();
        assert_eq!(option_type, OptionType::Put);

        let option_type = OptionSymbol::from("IQ 200918C00017500").option_type();
        assert_eq!(option_type, OptionType::Call);
    }

    #[test]
    fn test_option_symbol_strike_price() {
        let strike_price = OptionSymbol::from("IQ 200918P00017500").strike_price();
        assert_eq!(strike_price, Rational64::new(175, 10));
    }

    #[test]
    fn test_option_symbol_strike_price2() {
        let strike_price = OptionSymbol::from("PENN  200821C00040500").strike_price();
        assert_eq!(strike_price, Rational64::new(405, 10));
    }

    #[test]
    fn test_quote_symbol_matches_underlying_symbol() {
        let quote_symbol = QuoteSymbol::from(".IQ200918P17.5");
        assert!(quote_symbol.matches_underlying_symbol("IQ"));
    }
}
