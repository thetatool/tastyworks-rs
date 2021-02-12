use chrono::{DateTime, Utc};
use futures::{stream, StreamExt};
use itertools::Itertools;

pub mod api;
pub mod common;
mod constants;
pub mod csv;
pub mod errors;
pub mod request;
pub mod streamer;
pub mod symbol;

use crate::errors::*;
pub use crate::{api::*, request::*};

const MAX_SYMBOL_SUMMARY_BATCH_SIZE: usize = 500;
const PARALLEL_REQUESTS: usize = 10;

pub async fn accounts() -> Result<Vec<accounts::Item>, ApiError> {
    let url = "customers/me/accounts";
    let response: api::Response<accounts::Response> =
        deserialize_response(request(url, "").await?).await?;
    Ok(response.data.items)
}

pub async fn watchlists() -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "").await?).await?;
    Ok(response.data.items)
}

pub async fn public_watchlists() -> Result<Vec<watchlists::Item>, ApiError> {
    let url = "public-watchlists";
    let response: api::Response<watchlists::Response> =
        deserialize_response(request(url, "").await?).await?;
    Ok(response.data.items)
}

pub async fn positions(account: &accounts::Account) -> Result<Vec<positions::Item>, ApiError> {
    let url = format!("accounts/{}/positions", account.account_number);
    let response: api::Response<positions::Response> =
        deserialize_response(request(&url, "").await?).await?;
    Ok(response.data.items)
}

pub async fn transactions(
    account: &accounts::Account,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    prev_pagination: Option<Pagination>,
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
        start_date, end_date, page_offset
    );
    let response: api::Response<transactions::Response> =
        deserialize_response(request(&url, &parameters).await?).await?;

    Ok(Some((response.data.items, response.pagination)))
}

pub async fn market_metrics(symbols: &[String]) -> Result<Vec<market_metrics::Item>, ApiError> {
    let mut results = stream::iter(symbols.chunks(MAX_SYMBOL_SUMMARY_BATCH_SIZE).map(
        |batch| async move {
            let symbols = batch.iter().cloned().join(",");

            let url_path = "market-metrics";
            let params_string = &format!("symbols={}", symbols);
            let response: Result<api::Response<market_metrics::Response>, ApiError> =
                deserialize_response(request(url_path, params_string).await?).await;

            response
        },
    ))
    .buffered(PARALLEL_REQUESTS)
    .collect::<Vec<_>>()
    .await;

    let mut json = vec![];
    for result in results.drain(..) {
        json.append(&mut result?.data.items);
    }

    Ok(json)
}

pub async fn option_chains(symbol: &str) -> Result<Vec<option_chains::Item>, ApiError> {
    let url = format!("option-chains/{}/nested", symbol);
    let response: api::Response<option_chains::Response> =
        deserialize_response(request(&url, "").await?).await?;
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
