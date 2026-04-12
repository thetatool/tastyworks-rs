#[path = "common/auth.rs"]
mod auth;

use num_rational::Rational64;
use num_traits::ToPrimitive;
use tastyworks::{
    api::{self, InstrumentType},
    streamer::{self, SubscriptionValue, SymbolSubscriptionKind},
    symbol,
};

use std::collections::HashMap;
use std::error::Error;
use std::io::{Write, stdout};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let session = auth::session_from_env_or_login("live_positions").await?;

    let account = tastyworks::accounts(&session)
        .await?
        .into_iter()
        .next()
        .expect("No accounts found");

    let positions = tastyworks::positions(&account, &session).await?;
    if positions.is_empty() {
        println!("No open positions.");
        return Ok(());
    }

    let quote_symbols = quote_symbols_for_positions(&positions);
    let quote_fields = vec![
        "eventSymbol".to_string(),
        "bidPrice".to_string(),
        "askPrice".to_string(),
    ];

    let mut streamer = streamer::Client::new(&session).await?;
    streamer.connect()?;
    streamer.add_symbol_subscriptions(
        SymbolSubscriptionKind::Quote,
        &quote_fields,
        &quote_symbols,
    )?;

    let mut quotes = HashMap::new();
    render_positions(&positions, &quotes);

    loop {
        let subscription_data = match streamer.poll_subscriptions() {
            Ok(subscription_data) => subscription_data,
            Err(err) if err.is_disconnect() => {
                eprintln!("streamer disconnected: {}", err);
                tokio::time::sleep(Duration::from_secs(1)).await;
                streamer.connect()?;
                streamer.add_symbol_subscriptions(
                    SymbolSubscriptionKind::Quote,
                    &quote_fields,
                    &quote_symbols,
                )?;
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        if let Some(quote_data) = subscription_data.get("Quote") {
            for ((event_symbol, bid), ask) in quote_data
                .iter_field("eventSymbol")
                .zip(quote_data.iter_field("bidPrice"))
                .zip(quote_data.iter_field("askPrice"))
            {
                let Some(event_symbol) = event_symbol.as_str() else {
                    continue;
                };

                quotes.insert(
                    event_symbol.to_string(),
                    Quote {
                        bid: bid.to_price(),
                        ask: ask.to_price(),
                    },
                );
            }

            render_positions(&positions, &quotes);
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn quote_symbols_for_positions(positions: &[api::positions::Item]) -> Vec<String> {
    let mut quote_symbols = positions
        .iter()
        .map(quote_symbol_for_position)
        .collect::<Vec<_>>();
    quote_symbols.sort();
    quote_symbols.dedup();
    quote_symbols
}

fn quote_symbol_for_position(position: &api::positions::Item) -> String {
    match position.instrument_type {
        InstrumentType::EquityOption => symbol::OptionSymbol::from(&position.symbol).quote_symbol(),
        _ => position.symbol.clone(),
    }
}

fn render_positions(positions: &[api::positions::Item], quotes: &HashMap<String, Quote>) {
    // Clear the screen and move the cursor to the top-left so each refresh redraws in place.
    print!("\x1B[2J\x1B[H");
    println!("{:<24} {:>10} {:>12} {:>12}", "symbol", "qty", "bid", "ask");
    println!("{}", "-".repeat(64));

    for position in positions {
        let stream_symbol = quote_symbol_for_position(position);
        let quote = quotes.get(&stream_symbol);
        let bid = quote.and_then(|quote| quote.bid);
        let ask = quote.and_then(|quote| quote.ask);

        println!(
            "{:<24} {:>10} {:>12} {:>12}",
            position.symbol,
            format_quantity(position.signed_quantity()),
            format_price(bid),
            format_price(ask),
        );
    }

    let _ = stdout().flush();
}

fn format_quantity(value: Rational64) -> String {
    if value.is_integer() {
        value.to_integer().to_string()
    } else {
        format!("{:.4}", value.to_f64().expect("quantity should fit in f64"))
    }
}

fn format_price(value: Option<Rational64>) -> String {
    match value {
        Some(value) => format!("{:.4}", value.to_f64().expect("price should fit in f64")),
        None => "-".to_string(),
    }
}

#[derive(Clone, Copy, Debug)]
struct Quote {
    bid: Option<Rational64>,
    ask: Option<Rational64>,
}
