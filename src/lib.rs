//! Unofficial tastyworks/tastytrade API for Rust. Requires [API access to be enabled](https://support.tastytrade.com/support/s/solutions/articles/43000700385) for your account.
//!
//! ## Example
//!
//! ```rust
//! use tastyworks::Session;
//! use num_traits::ToPrimitive;
//!
//! // Requests made by the API are asynchronous, so you must use a runtime such as `tokio`.
//! #[tokio::main]
//! async fn main() {
//!   let login = "username"; // or email
//!   let password = "password";
//!   let otp = Some("123456"); // 2FA code, may be None::<String>
//!   let session = Session::from_credentials(login, password, otp)
//!       .await.expect("Failed to login");
//!
//!   let accounts = tastyworks::accounts(&session)
//!       .await.expect("Failed to fetch accounts");
//!   let account = accounts.first().expect("No accounts found");
//!
//!   let positions = tastyworks::positions(account, &session)
//!       .await.expect("Failed to fetch positions");
//!
//!   println!("Your active positions:");
//!   for position in &positions {
//!       let signed_quantity = position.signed_quantity();
//!
//!       // Quantities in the API that could potentially be decimal values are stored as
//!       // `num_rational::Rational64`. To convert these to floats include the `num-traits` crate
//!       // in your project and use the `ToPrimitive` trait. To convert these to integers no
//!       // additional crate is required.
//!       println!(
//!           "{:>10} x {}",
//!           if signed_quantity.is_integer() {
//!               signed_quantity.to_integer().to_string()
//!           } else {
//!               signed_quantity.to_f64().unwrap().to_string()
//!           },
//!           position.symbol
//!       );
//!   }
//! }
//! ```

use chrono::{DateTime, TimeZone, Utc};
use futures::{stream, StreamExt};
use itertools::Itertools;

pub mod api;
pub mod common;
pub mod csv;
pub mod errors;
pub mod request;
pub mod session;
pub mod streamer;
pub mod symbol;

use crate::errors::*;
pub use crate::{api::*, request::*, session::Session};

const MAX_SYMBOL_SUMMARY_BATCH_SIZE: usize = 500;
const PARALLEL_REQUESTS: usize = 10;

pub async fn accounts(session: &Session) -> Result<Vec<accounts::Account>, ApiError> {
    let url = "customers/me/accounts";
    let response: api::Response<accounts::Response> =
        deserialize_response(request(url, "", session).await?).await?;
    Ok(response
        .data
        .items
        .into_iter()
        .map(|item| item.account)
        .collect())
}

pub async fn watchlists(session: &Session) -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "", session).await?).await?;
    Ok(response.data.items)
}

pub async fn public_watchlists(session: &Session) -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "public-watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "", session).await?).await?;
    Ok(response.data.items)
}

pub async fn balances(
    account: &accounts::Account,
    session: &Session,
) -> Result<balances::Data, ApiError> {
    let url = format!("accounts/{}/balances", account.account_number);
    let response: api::Response<balances::Data> =
        deserialize_response(request(&url, "", session).await?).await?;
    Ok(response.data)
}

pub async fn positions(
    account: &accounts::Account,
    session: &Session,
) -> Result<Vec<positions::Item>, ApiError> {
    let url = format!("accounts/{}/positions", account.account_number);
    let response: api::Response<positions::Response> =
        deserialize_response(request(&url, "", session).await?).await?;
    Ok(response.data.items)
}

pub async fn transactions<Tz: TimeZone>(
    account: &accounts::Account,
    start_date: DateTime<Tz>,
    end_date: DateTime<Tz>,
    prev_pagination: Option<Pagination>,
    session: &Session,
) -> Result<Option<(Vec<transactions::Item>, Option<Pagination>)>, ApiError> {
    let page_offset = if let Some(api::Pagination {
        page_offset,
        total_pages,
        ..
    }) = prev_pagination
    {
        if page_offset + 1 >= total_pages {
            return Ok(None);
        }
        page_offset + 1
    } else {
        0
    };

    let url = format!("accounts/{}/transactions", account.account_number);
    let parameters = format!(
        "start-date={}&end-date={}&page-offset={}",
        start_date.with_timezone(&Utc),
        end_date.with_timezone(&Utc),
        page_offset
    );
    let response: api::Response<transactions::Response> =
        deserialize_response(request(&url, &parameters, session).await?).await?;

    Ok(Some((response.data.items, response.pagination)))
}

pub async fn market_metrics(
    symbols: &[String],
    session: &Session,
) -> Result<Vec<market_metrics::Item>, ApiError> {
    let results = stream::iter(symbols.chunks(MAX_SYMBOL_SUMMARY_BATCH_SIZE).map(
        |batch| async move {
            let symbols = batch.iter().cloned().join(",");

            let url_path = "market-metrics";
            let params_string = &format!("symbols={}", symbols);
            let response: Result<api::Response<market_metrics::Response>, ApiError> =
                deserialize_response(request(url_path, params_string, session).await?).await;

            response
        },
    ))
    .buffered(PARALLEL_REQUESTS)
    .collect::<Vec<_>>()
    .await;

    let mut json = vec![];
    for result in results.into_iter() {
        json.append(&mut result?.data.items);
    }

    Ok(json)
}

pub async fn option_chains(
    symbol: &str,
    session: &Session,
) -> Result<Vec<option_chains::Item>, ApiError> {
    let url = format!("option-chains/{}/nested", symbol);
    let response: api::Response<option_chains::Response> =
        deserialize_response(request(&url, "", session).await?).await?;
    Ok(response.data.items)
}
