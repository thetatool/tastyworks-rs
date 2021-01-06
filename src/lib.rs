use chrono::{DateTime, Utc};
use futures::{stream, StreamExt};
use itertools::Itertools;

use std::error::Error;

pub mod api;
pub mod common;
pub mod csv;
pub mod request;
pub mod streamer;
pub mod symbol;

pub use crate::{api::*, common::*, request::*};

const MAX_SYMBOL_SUMMARY_BATCH_SIZE: usize = 500;
const PARALLEL_REQUESTS: usize = 10;

pub async fn accounts() -> Result<Vec<accounts::Item>, Box<dyn Error>> {
    let url = "customers/me/accounts";
    let response: api::Response<accounts::Response> =
        request(url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            e
        })?;
    Ok(response.data.items)
}

pub async fn watchlists() -> Result<Vec<watchlists::Item>, Box<dyn Error>> {
    let url = "watchlists";
    let response: api::Response<watchlists::Response> =
        request(url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            e
        })?;
    Ok(response.data.items)
}

pub async fn public_watchlists() -> Result<Vec<watchlists::Item>, Box<dyn Error>> {
    let url = "public-watchlists";
    let response: api::Response<watchlists::Response> =
        request(url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            e
        })?;
    Ok(response.data.items)
}

pub async fn positions(
    account: &accounts::Account,
) -> Result<Vec<positions::Item>, Box<dyn Error>> {
    let url = format!("accounts/{}/positions", account.account_number);
    let response: api::Response<positions::Response> =
        request(&url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            e
        })?;
    Ok(response.data.items)
}

pub async fn transactions(
    account: &accounts::Account,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    prev_pagination: Option<Pagination>,
) -> Result<Option<(Vec<transactions::Item>, Option<Pagination>)>, Box<dyn Error>> {
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
        .map_err(|e| {
            log::error!("Error deserializing {}?{}: {:?}", url, parameters, e);
            e
        })?;

    Ok(Some((response.data.items, response.pagination)))
}

pub async fn market_metrics(
    symbols: &[String],
) -> Result<Vec<market_metrics::Item>, Box<dyn Error>> {
    let mut results = stream::iter(symbols.chunks(MAX_SYMBOL_SUMMARY_BATCH_SIZE).map(
        |batch| async move {
            let symbols = batch.iter().cloned().join(",");

            let url_path = "market-metrics";
            let params_string = &format!("symbols={}", symbols);
            let response = request(url_path, params_string).await?;

            let json_string = response.text().await?;
            let result: Result<api::Response<market_metrics::Response>, Box<dyn Error>> =
                serde_json::from_str(&json_string).map_err(|e| {
                    log::error!(
                        "Error deserializing {}?{}: {:?}",
                        url_path,
                        params_string,
                        e,
                    );
                    e.into()
                });

            result
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

pub async fn option_chains(symbol: &str) -> Result<Vec<option_chains::Item>, Box<dyn Error>> {
    let url = format!("option-chains/{}/nested", symbol);
    let response: api::Response<option_chains::Response> =
        request(&url, "").await?.json().await.map_err(|e| {
            log::error!("Error deserializing {}: {:?}", url, e);
            e
        })?;
    Ok(response.data.items)
}
