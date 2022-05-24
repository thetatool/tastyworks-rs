//! Unofficial Tastyworks API for Rust.
//!
//! ## Example
//!
//! ```rust
//! use tastyworks::Context;
//! use num_traits::ToPrimitive;
//!
//! // Requests made by the API are asynchronous, so you must use a runtime such as `tokio`.
//! #[tokio::main]
//! async fn main() {
//!   // See section below for instructions on finding your API token
//!   let token = "your-token-here";
//!   let context = Context::from_token(token);
//!
//!   let accounts = tastyworks::accounts(&context)
//!       .await.expect("Failed to fetch accounts");
//!   let account = accounts.first().expect("No accounts found");
//!
//!   let positions = tastyworks::positions(account, &context)
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
//!
//! ## API Token
//!
//! Your API token can be found by logging in to https://trade.tastyworks.com/ while your browser developer tools are open on the `Network` tab.
//! Select one of the requests made to https://api.tastyworks.com/ and in the `Request Headers` section that appears, find the `Authorization` header item.
//! The value of this item can be used as your `token` in this API.

use chrono::{DateTime, TimeZone, Utc};
use futures::{stream, StreamExt};
use itertools::Itertools;

pub mod api;
pub mod common;
mod constants;
pub mod context;
pub mod csv;
pub mod errors;
pub mod request;
pub mod streamer;
pub mod symbol;

use crate::errors::*;
pub use crate::{api::*, context::Context, request::*};

const MAX_SYMBOL_SUMMARY_BATCH_SIZE: usize = 500;
const PARALLEL_REQUESTS: usize = 10;

pub async fn accounts(context: &Context) -> Result<Vec<accounts::Account>, ApiError> {
    let url = "customers/me/accounts";
    let response: api::Response<accounts::Response> =
        deserialize_response(request(url, "", context).await?).await?;
    Ok(response
        .data
        .items
        .into_iter()
        .map(|item| item.account)
        .collect())
}

pub async fn watchlists(context: &Context) -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "", context).await?).await?;
    Ok(response.data.items)
}

pub async fn public_watchlists(context: &Context) -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "public-watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "", context).await?).await?;
    Ok(response.data.items)
}

pub async fn balances(
    account: &accounts::Account,
    context: &Context,
) -> Result<balances::Data, ApiError> {
    let url = format!("accounts/{}/balances", account.account_number);
    let response: api::Response<balances::Data> =
        deserialize_response(request(&url, "", context).await?).await?;
    Ok(response.data)
}

pub async fn positions(
    account: &accounts::Account,
    context: &Context,
) -> Result<Vec<positions::Item>, ApiError> {
    let url = format!("accounts/{}/positions", account.account_number);
    let response: api::Response<positions::Response> =
        deserialize_response(request(&url, "", context).await?).await?;
    Ok(response.data.items)
}

pub async fn transactions<Tz: TimeZone>(
    account: &accounts::Account,
    start_date: DateTime<Tz>,
    end_date: DateTime<Tz>,
    prev_pagination: Option<Pagination>,
    context: &Context,
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
        deserialize_response(request(&url, &parameters, context).await?).await?;

    Ok(Some((response.data.items, response.pagination)))
}

pub async fn market_metrics(
    symbols: &[String],
    context: &Context,
) -> Result<Vec<market_metrics::Item>, ApiError> {
    let results = stream::iter(symbols.chunks(MAX_SYMBOL_SUMMARY_BATCH_SIZE).map(
        |batch| async move {
            let symbols = batch.iter().cloned().join(",");

            let url_path = "market-metrics";
            let params_string = &format!("symbols={}", symbols);
            let response: Result<api::Response<market_metrics::Response>, ApiError> =
                deserialize_response(request(url_path, params_string, context).await?).await;

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
    context: &Context,
) -> Result<Vec<option_chains::Item>, ApiError> {
    let url = format!("option-chains/{}/nested", symbol);
    let response: api::Response<option_chains::Response> =
        deserialize_response(request(&url, "", context).await?).await?;
    Ok(response.data.items)
}

async fn deserialize_response<T>(response: reqwest::Response) -> Result<T, ApiError>
where
    T: serde::de::DeserializeOwned,
{
    let url = response.url().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| RequestError::FailedRequest {
            e,
            url: request::obfuscate_account_url(&url),
        })?;

    let de = &mut serde_json::Deserializer::from_slice(&bytes);
    let result: Result<T, _> = serde_path_to_error::deserialize(de);
    result.map_err(|e| ApiError::Decode {
        e: Box::new(e),
        url: request::obfuscate_account_url(&url),
    })
}
