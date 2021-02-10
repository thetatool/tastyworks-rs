use chrono::{DateTime, Utc};
use futures::{stream, StreamExt};
use itertools::Itertools;

pub mod api;
pub mod common;
pub mod csv;
pub mod request;
pub mod streamer;
pub mod symbol;

pub use crate::{api::*, common::*, request::*};

const MAX_SYMBOL_SUMMARY_BATCH_SIZE: usize = 500;
const PARALLEL_REQUESTS: usize = 10;

pub async fn accounts() -> Result<Vec<accounts::Item>, RequestError> {
    let url = "customers/me/accounts";
    let response: api::Response<accounts::Response> = request(url, "")
        .await?
        .json()
        .await
        .map_err(|e| map_decode_err(e, url, None))?;
    Ok(response.data.items)
}

pub async fn watchlists() -> Result<Vec<watchlists::Item>, RequestError> {
    let url = "watchlists";
    let response: api::Response<watchlists::Response> = request(url, "")
        .await?
        .json()
        .await
        .map_err(|e| map_decode_err(e, url, None))?;
    Ok(response.data.items)
}

pub async fn public_watchlists() -> Result<Vec<watchlists::Item>, RequestError> {
    let url = "public-watchlists";
    let response: api::Response<watchlists::Response> = request(url, "")
        .await?
        .json()
        .await
        .map_err(|e| map_decode_err(e, url, None))?;
    Ok(response.data.items)
}

pub async fn positions(account: &accounts::Account) -> Result<Vec<positions::Item>, RequestError> {
    let url = format!("accounts/{}/positions", account.account_number);
    let response: api::Response<positions::Response> =
        request(&url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            RequestError::FailedRequest {
                e,
                url: request::obfuscate_account_url(&url),
            }
        })?;
    Ok(response.data.items)
}

pub async fn transactions(
    account: &accounts::Account,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    prev_pagination: Option<Pagination>,
) -> Result<Option<(Vec<transactions::Item>, Option<Pagination>)>, RequestError> {
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
    let response: api::Response<transactions::Response> = request(&url, &parameters)
        .await?
        .json()
        .await
        .map_err(|e| map_decode_err(e, &request::obfuscate_account_url(&url), Some(&parameters)))?;

    Ok(Some((response.data.items, response.pagination)))
}

pub async fn market_metrics(symbols: &[String]) -> Result<Vec<market_metrics::Item>, RequestError> {
    let mut results = stream::iter(symbols.chunks(MAX_SYMBOL_SUMMARY_BATCH_SIZE).map(
        |batch| async move {
            let symbols = batch.iter().cloned().join(",");

            let url_path = "market-metrics";
            let params_string = &format!("symbols={}", symbols);
            let response: Result<api::Response<market_metrics::Response>, RequestError> =
                request(url_path, params_string)
                    .await?
                    .json()
                    .await
                    .map_err(|e| map_decode_err(e, url_path, Some(params_string)));

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

pub async fn option_chains(symbol: &str) -> Result<Vec<option_chains::Item>, RequestError> {
    let url = format!("option-chains/{}/nested", symbol);
    let response: api::Response<option_chains::Response> = request(&url, "")
        .await?
        .json()
        .await
        .map_err(|e| map_decode_err(e, &url, None))?;
    Ok(response.data.items)
}

fn map_decode_err(e: reqwest::Error, url: &str, params: Option<&str>) -> RequestError {
    let url_string = if let Some(params) = params {
        format!("{}?{}", url, params)
    } else {
        url.to_string()
    };

    RequestError::Decode { e, url: url_string }
}
