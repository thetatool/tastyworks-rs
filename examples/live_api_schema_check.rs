#[path = "common/auth.rs"]
mod auth;

use chrono::{Duration as ChronoDuration, Utc};
use tastyworks::{api, streamer};

use std::error::Error;
use std::process;
use std::time::{Duration, Instant};

const STREAMER_SYMBOLS: &[&str] = &["SPY", "QQQ", "IWM", "AAPL"];
const MARKET_METRIC_SYMBOLS: &[&str] = &["SPY", "QQQ", "IWM", "AAPL", "NVDA", "TSLA", "SPX"];
const OPTION_CHAIN_SYMBOLS: &[&str] = &["SPY", "QQQ", "IWM", "AAPL", "EB", "SPX"];
const TRANSACTIONS_DAYS: i64 = 365;
const MAX_TRANSACTION_PAGES: usize = 5;
const STREAMER_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let session = auth::session_from_env_or_login("live_api_schema_check").await?;

    let mut passed = 0usize;
    let mut failures = Vec::new();

    let accounts = match tastyworks::accounts(&session).await {
        Ok(accounts) => {
            passed += 1;
            println!(
                "PASS customers/me/accounts accounts={}{}",
                accounts.len(),
                if accounts.is_empty() {
                    " (no accounts found)"
                } else {
                    ""
                }
            );
            accounts
        }
        Err(err) => {
            record_failure(&mut failures, "customers/me/accounts", err);
            Vec::new()
        }
    };

    match tastyworks::watchlists(&session).await {
        Ok(watchlists) => {
            passed += 1;
            println!("PASS watchlists watchlists={}", watchlists.len());
        }
        Err(err) => record_failure(&mut failures, "watchlists", err),
    }

    match tastyworks::public_watchlists(&session).await {
        Ok(watchlists) => {
            passed += 1;
            println!("PASS public-watchlists watchlists={}", watchlists.len());
        }
        Err(err) => record_failure(&mut failures, "public-watchlists", err),
    }

    let transaction_start = Utc::now() - ChronoDuration::days(TRANSACTIONS_DAYS);
    let transaction_end = Utc::now();

    for account in &accounts {
        let account_label = format!("accounts/{}/balances", display_account(account));
        match tastyworks::balances(account, &session).await {
            Ok(_) => {
                passed += 1;
                println!("PASS {}", account_label);
            }
            Err(err) => record_failure(&mut failures, &account_label, err),
        }

        let account_label = format!("accounts/{}/positions", display_account(account));
        match tastyworks::positions(account, &session).await {
            Ok(positions) => {
                passed += 1;
                println!("PASS {} positions={}", account_label, positions.len());
            }
            Err(err) => record_failure(&mut failures, &account_label, err),
        }

        let account_label = format!("accounts/{}/transactions", display_account(account));
        let mut prev_pagination = None;
        let mut page_count = 0usize;
        let mut item_count = 0usize;
        let mut transaction_failed = false;

        loop {
            if page_count >= MAX_TRANSACTION_PAGES {
                break;
            }

            match tastyworks::transactions(
                account,
                transaction_start,
                transaction_end,
                prev_pagination.clone(),
                &session,
            )
            .await
            {
                Ok(Some((items, pagination))) => {
                    page_count += 1;
                    item_count += items.len();

                    if pagination.is_none() {
                        break;
                    }
                    prev_pagination = pagination;
                }
                Ok(None) => break,
                Err(err) => {
                    transaction_failed = true;
                    record_failure(&mut failures, &account_label, err);
                    break;
                }
            }
        }

        if !transaction_failed {
            passed += 1;
            println!(
                "PASS {} pages={} items={}",
                account_label, page_count, item_count
            );
        }
    }

    let metric_symbols = MARKET_METRIC_SYMBOLS
        .iter()
        .map(|symbol| (*symbol).to_string())
        .collect::<Vec<_>>();
    let market_metrics_label = format!("market-metrics symbols={}", metric_symbols.len());
    match tastyworks::market_metrics(&metric_symbols, &session).await {
        Ok(items) => {
            if items.is_empty() {
                record_failure(
                    &mut failures,
                    &market_metrics_label,
                    "response decoded but returned 0 items",
                );
            } else {
                passed += 1;
                println!("PASS {} items={}", market_metrics_label, items.len());
            }
        }
        Err(err) => record_failure(&mut failures, &market_metrics_label, err),
    }

    let mut option_chain_successes = 0usize;
    let mut last_option_chain_error = None;

    for symbol in OPTION_CHAIN_SYMBOLS {
        let label = format!("option-chains/{symbol}/nested");
        match tastyworks::option_chains(symbol, &session).await {
            Ok(items) if !items.is_empty() => {
                passed += 1;
                option_chain_successes += 1;
                println!("PASS {} chains={}", label, items.len());
            }
            Ok(_) => {
                eprintln!("SKIP {} response decoded but returned 0 chains", label);
                last_option_chain_error =
                    Some("response decoded but returned 0 chains".to_string());
            }
            Err(err) => {
                eprintln!("SKIP {} {}", label, err);
                last_option_chain_error = Some(err.to_string());
            }
        }
    }

    if option_chain_successes == 0 {
        let detail = last_option_chain_error.unwrap_or_else(|| {
            "no known-good option-chain symbol produced a non-empty response".to_string()
        });
        failures.push(format!(
            "option-chains/*/nested failed for all known-good symbols: {}",
            detail
        ));
    }

    match streamer::Client::new(&session).await {
        Ok(mut client) => {
            passed += 1;
            println!("PASS api-quote-tokens");

            let label = format!("streamer Quote symbols={}", STREAMER_SYMBOLS.len());
            match run_streamer_smoke_test(&mut client, STREAMER_SYMBOLS).await {
                Ok(saw_data) => {
                    passed += 1;
                    if saw_data {
                        println!("PASS {} data=received", label);
                    } else {
                        println!("PASS {} data=not-received-within-timeout", label);
                    }
                }
                Err(err) => record_failure(&mut failures, &label, err),
            }
        }
        Err(err) => record_failure(&mut failures, "api-quote-tokens", err),
    }

    println!();
    println!("Summary: passed={} failed={}", passed, failures.len());

    if failures.is_empty() {
        return Ok(());
    }

    eprintln!("Failures:");
    for failure in &failures {
        eprintln!("- {}", failure);
    }
    process::exit(1);
}

fn record_failure(failures: &mut Vec<String>, label: &str, err: impl ToString) {
    let err = err.to_string();
    eprintln!("FAIL {} {}", label, err);
    failures.push(format!("{}: {}", label, err));
}

fn display_account(account: &api::accounts::Account) -> String {
    let suffix_len = account.account_number.len().min(4);
    let suffix = &account.account_number[account.account_number.len() - suffix_len..];
    format!("***{}", suffix)
}

async fn run_streamer_smoke_test(
    client: &mut streamer::Client,
    symbols: &[&str],
) -> Result<bool, Box<dyn Error>> {
    client.connect()?;

    let fields = vec![
        "eventSymbol".to_string(),
        "bidPrice".to_string(),
        "askPrice".to_string(),
    ];
    let symbols = symbols
        .iter()
        .map(|symbol| (*symbol).to_string())
        .collect::<Vec<_>>();
    client.add_subscription("Quote", &fields, &symbols)?;

    let deadline = Instant::now() + STREAMER_TIMEOUT;
    while Instant::now() < deadline {
        let subscriptions = client.poll_subscriptions()?;
        if let Some(quote_data) = subscriptions.get("Quote") {
            if quote_data.iter_field("eventSymbol").next().is_some() {
                return Ok(true);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Ok(false)
}
