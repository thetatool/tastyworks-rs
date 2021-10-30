# tastyworks-rs

[![Crates.io][crates_img]][crates_link]

[crates_img]: https://img.shields.io/crates/v/tastyworks.svg
[crates_link]: https://crates.io/crates/tastyworks

Unofficial Tastyworks API for Rust.

## Example

```rust
use tastyworks::Context;
use num_traits::ToPrimitive;

// Requests made by the API are asynchronous, so you must use a runtime such as `tokio`.
#[tokio::main]
async fn main() {
   // See section below for instructions on finding your API token
  let token = "your-token-here";
  let context = Context::from_token(token);

  let accounts = tastyworks::accounts(&context)
      .await.expect("Failed to fetch accounts");
  let account = accounts.first().expect("No accounts found");

  let positions = tastyworks::positions(account, &context)
      .await.expect("Failed to fetch positions");

  println!("Your active positions:");
  for position in &positions {
      let signed_quantity = position.signed_quantity();

      // Quantities in the API that could potentially be decimal values are stored as
      // `num_rational::Rational64`. To convert these to floats include the `num-traits` crate
      // in your project and use the `ToPrimitive` trait. To convert these to integers no
      // additional crate is required.
      println!(
          "{:>10} x {}",
          if signed_quantity.is_integer() {
              signed_quantity.to_integer().to_string()
          } else {
              signed_quantity.to_f64().unwrap().to_string()
          },
          position.symbol
      );
  }
}
```

## API Token

Your API token can be found by logging in to https://trade.tastyworks.com/ while your browser developer tools are open on the `Network` tab.
Select one of the requests made to https://api.tastyworks.com/ and in the `Request Headers` section that appears, find the `Authorization` header item.
The value of this item can be used as your `token` in this API.
