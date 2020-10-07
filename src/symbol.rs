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
        let (integer_str, decimal_str) = self.price_components();
        format!(
            ".{}{}{}{}{}{}{}{}",
            if self.is_future_option_symbol() {
                "/"
            } else {
                ""
            },
            self.underlying_symbol(),
            self.date_component(),
            self.option_type_component(),
            integer_str,
            if decimal_str.is_none() { "" } else { "." },
            decimal_str.unwrap_or(""),
            self.future_exchange()
                .map(|ex| format!(":{}", ex))
                .unwrap_or_else(|| "".to_string()),
        )
    }

    fn date_component(&self) -> &str {
        if self.is_future_option_symbol() {
            return "";
        }

        let component = self.0.split_whitespace().nth(1);
        let date = component.and_then(|c| c.get(..6));
        date.unwrap_or_else(|| panic!("Missing date component for symbol: {}", self.0))
    }

    fn option_type_component(&self) -> char {
        self.common_slice()
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.chars().nth(6))
            .unwrap_or_else(|| panic!("Missing option type component for symbol: {}", self.0))
    }

    fn price_components(&self) -> (&str, Option<&str>) {
        let component = self.common_slice().split_whitespace().nth(1);
        let price = component
            .and_then(|c| c.get(7..))
            .unwrap_or_else(|| panic!("Missing price component for symbol: {}", self.0));

        let (integer_str, decimal_str) = if self.is_future_option_symbol() {
            let mut iter = price.split('.');
            let integer = iter
                .next()
                .unwrap_or_else(|| panic!("Missing price separator for symbol: {}", self.0));
            let decimal = iter.next().unwrap_or("");
            (integer, decimal)
        } else {
            let integer = &price[..5];
            let decimal = &price[5..];
            (integer, decimal)
        };

        let decimal_str = decimal_str.trim_end_matches('0');

        (
            integer_str.trim_start_matches('0'),
            if decimal_str.is_empty() {
                None
            } else {
                Some(decimal_str)
            },
        )
    }

    pub fn expiration_date(&self) -> Date {
        let date_str = self.date_component();
        let date = NaiveDate::parse_from_str(date_str, "%y%m%d").ok().map(Date);
        date.unwrap_or_else(|| panic!("Missing expiration date for symbol: {}", self.0))
    }

    pub fn underlying_symbol(&self) -> &'a str {
        let underlying_symbol = self
            .common_slice()
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
        let (integer_str, decimal_str) = self.price_components();
        let integer =
            i64::from_str(&format!("{}{:0<3}", integer_str, decimal_str.unwrap_or(""))).unwrap();
        Rational64::new(integer, 1000)
    }

    fn common_slice(&self) -> &'a str {
        if self.is_future_option_symbol() {
            self.0.splitn(2, ' ').nth(1).unwrap().trim_start()
        } else {
            self.0
        }
    }

    fn future_symbol(&self) -> Option<&str> {
        if self.is_future_option_symbol() {
            self.0.split_whitespace().next().and_then(|c| {
                let expiry_chars_byte_len = 2; // e.g. Z0
                c.get(1..c.len() - expiry_chars_byte_len)
            })
        } else {
            None
        }
    }

    fn is_future_option_symbol(&self) -> bool {
        self.0.chars().nth(1).filter(|c| *c == '/').is_some()
    }

    fn future_exchange(&self) -> Option<&str> {
        let ex = match self.future_symbol()? {
            "/ZB" => "XCBT",
            "/ZN" => "XCBT",
            "/ZF" => "XCBT",
            "/ZT" => "XCBT",
            "/UB" => "XCBT",
            "/GE" => "XCME",
            "/6A" => "XCME",
            "/6B" => "XCME",
            "/6C" => "XCME",
            "/6E" => "XCME",
            "/6J" => "XCME",
            "/6M" => "XCME",
            "/ZC" => "XCBT",
            "/ZS" => "XCBT",
            "/ZW" => "XCBT",
            "/HE" => "XCBT",
            "/CL" => "XNYM",
            "/NG" => "XNYM",
            "/GC" => "XCEC",
            "/SI" => "XCEC",
            "/HG" => "XCEC",
            "/ES" => "XCME",
            "/NQ" => "XCME",
            "/YM" => "XCBT",
            "/RTY" => "XCME",
            "/VX" => "XCBF",
            "/VXM" => "XCBF",
            "/BTC" => "XCME",
            symbol => panic!("Unhandled future: {}", symbol),
        };

        Some(ex)
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
    fn test_futures_option_symbol_quote_symbol() {
        let quote_symbol = OptionSymbol::from("./NGZ0 LNEZ0 201124C4.5").quote_symbol();
        assert_eq!(quote_symbol, "./LNEZ20C4.5:XNYM");
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
