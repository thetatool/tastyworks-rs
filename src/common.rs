use chrono::{Duration, NaiveDate, Utc};
use num_rational::Rational64;
use num_traits::{Signed, Zero};

use std::error::Error;
use std::fmt;
use std::str::FromStr;

const MAX_DECIMAL_POINTS: i64 = 4;
const DECIMAL_MULTIPLIER: i64 = 10_000; // 10^MAX_DECIMAL_POINTS

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Decimal(pub Rational64);

impl Decimal {
    pub fn abs(&self) -> Decimal {
        Decimal(self.0.abs())
    }
}

#[derive(Debug, Clone)]
struct PriceFromStrError(String);

impl Error for PriceFromStrError {}

impl fmt::Display for PriceFromStrError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "'{}' could not be parsed as price", self.0)
    }
}

impl FromStr for Decimal {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let is_negative = s.starts_with('-');

        let mut components = s.split('.');
        let integer = components
            .next()
            .map(|s| i64::from_str(&s.replace(',', "")))
            .transpose()?
            .ok_or_else(|| PriceFromStrError(s.to_string()))?
            .abs();

        let decimal = components
            .next()
            .map(|s| {
                let padded = format!("{:0<4}", s);
                if padded.len() != MAX_DECIMAL_POINTS as usize {
                    let boxed: Box<dyn Error> = PriceFromStrError(s.to_string()).into();
                    return Err(boxed);
                }

                i64::from_str(&padded).map_err(|e| {
                    let boxed: Box<dyn Error> = e.into();
                    boxed
                })
            })
            .transpose()?
            .unwrap_or(0);

        let mut numerator = integer * DECIMAL_MULTIPLIER + decimal;
        if is_negative {
            numerator *= -1;
        }

        let denominator = DECIMAL_MULTIPLIER;

        Ok(Decimal(Rational64::new(numerator, denominator)))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let integer = self.0.to_integer().abs();

        let decimal = format!(
            "{:0>4}",
            (self.0.fract() * DECIMAL_MULTIPLIER).to_integer().abs()
        );
        let decimal_trimmed = decimal.trim_end_matches('0');

        write!(
            f,
            "{}{}.{}",
            if self.0.is_negative() { "-" } else { "" },
            integer,
            decimal_trimmed
        )
    }
}

impl Default for Decimal {
    fn default() -> Self {
        Decimal(Rational64::zero())
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Date(pub NaiveDate);

impl Date {
    pub fn time_to_expiration(&self) -> Duration {
        let date_now = Utc::now().naive_utc().date();
        self.0 - date_now
    }
}

impl FromStr for Date {
    type Err = chrono::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Date(NaiveDate::from_str(s)?))
    }
}

impl Default for Date {
    fn default() -> Self {
        Date(NaiveDate::from_ymd(1, 1, 1))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum OptionType {
    Call,
    Put,
}

impl Default for OptionType {
    fn default() -> Self {
        OptionType::Call
    }
}

pub mod string_serialize {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Display,
        S: Serializer,
    {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

pub mod optional_string_serialize {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Display,
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let string: Option<String> = Option::deserialize(deserializer)?;

        string
            .map(|s| s.parse().map_err(de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decimal_from_str() {
        assert_eq!(
            Decimal::from_str("0.3").unwrap(),
            Decimal(Rational64::new(3, 10))
        );
        assert_eq!(
            Decimal::from_str("-0.3").unwrap(),
            Decimal(Rational64::new(-3, 10))
        );
        assert_eq!(
            Decimal::from_str("9.12").unwrap(),
            Decimal(Rational64::new(912, 100))
        );
        assert_eq!(
            Decimal::from_str("-9.12").unwrap(),
            Decimal(Rational64::new(-912, 100))
        );
        assert_eq!(
            Decimal::from_str("23.012").unwrap(),
            Decimal(Rational64::new(23012, 1000))
        );
        assert_eq!(
            Decimal::from_str("1.0001").unwrap(),
            Decimal(Rational64::new(10001, 10000))
        );
        assert_eq!(
            Decimal::from_str("12,345.4321").unwrap(),
            Decimal(Rational64::new(123454321, 10000))
        );
    }

    #[test]
    fn test_decimal_to_str() {
        assert_eq!(Decimal(Rational64::new(3, 10)).to_string(), "0.3",);
        assert_eq!(Decimal(Rational64::new(-3, 10)).to_string(), "-0.3",);
        assert_eq!(Decimal(Rational64::new(912, 100)).to_string(), "9.12",);
        assert_eq!(Decimal(Rational64::new(-912, 100)).to_string(), "-9.12",);
        assert_eq!(Decimal(Rational64::new(23012, 1000)).to_string(), "23.012",);
        assert_eq!(Decimal(Rational64::new(10001, 10000)).to_string(), "1.0001",);
    }
}
