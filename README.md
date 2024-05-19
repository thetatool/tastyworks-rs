# tastyworks-rs

[![Crates.io](https://img.shields.io/crates/v/tastyworks.svg)](https://crates.io/crates/tastyworks)
[![Docs Status](https://docs.rs/tastyworks/badge.svg)](https://docs.rs/tastyworks)

Unofficial tastyworks/tastytrade API for Rust. Requires [API access to be enabled](https://support.tastytrade.com/support/s/solutions/articles/43000700385) for your account.

## Example

```rust
use tastyworks::Session;
use num_traits::ToPrimitive;

// Requests made by the API are asynchronous, so you must use a runtime such as `tokio`.
#[tokio::main]
async fn main() {
  let login = "username"; // or email
  let password = "password";
  let otp = Some("123456"); // 2FA code, may be None::<String>
  let session = Session::from_credentials(login, password, otp)
      .await.expect("Failed to login");

  let accounts = tastyworks::accounts(&session)
      .await.expect("Failed to fetch accounts");
  let account = accounts.first().expect("No accounts found");

  let positions = tastyworks::positions(account, &session)
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
