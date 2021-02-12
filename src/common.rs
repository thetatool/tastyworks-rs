pub use options_common::{Decimal, ExpirationDate, OptionType};

use num_rational::Rational64;
use serde::{de, Deserialize, Deserializer, Serializer};

use std::convert::TryInto;
use std::fmt::{self, Display};
use std::str::FromStr;

pub mod string_serialize {
    use super::*;

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
    use super::*;

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

pub fn deserialize_integer_or_string_as_decimal<'de, D>(
    deserializer: D,
) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(DeserializeIntegerOrStringAsDecimal)
}

struct DeserializeIntegerOrStringAsDecimal;

impl<'de> de::Visitor<'de> for DeserializeIntegerOrStringAsDecimal {
    type Value = Decimal;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an integer or a string")
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Decimal(Rational64::from_integer(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_i64(v.try_into().map_err(de::Error::custom)?)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Decimal::from_str(v).map_err(de::Error::custom)
    }
}
