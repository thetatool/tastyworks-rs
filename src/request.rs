use crate::{context::Context, errors::RequestError};

use lazy_static::lazy_static;
use reqwest::{header, Client};

pub use reqwest::StatusCode;

lazy_static! {
    static ref CLIENT: Client = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.14; rv:79.0) \
             Gecko/20100101 Firefox/79.0"
        )
        .build()
        .unwrap();
}

pub async fn request(
    url_path: &str,
    params_string: &str,
    context: &Context,
) -> Result<reqwest::Response, RequestError> {
    let mut api_token_header_value = header::HeaderValue::from_str(&context.token).unwrap();
    api_token_header_value.set_sensitive(true);

    let params_string = if params_string.is_empty() {
        params_string.to_string()
    } else {
        format!("?{}", params_string)
    };

    let base_url = format!("https://api.tastyworks.com/{}", url_path);
    let url = format!("{}{}", base_url, params_string);

    let response = CLIENT
        .get(&url)
        .header(header::AUTHORIZATION, api_token_header_value)
        .send()
        .await;

    match response {
        Err(e) => {
            return Err(RequestError::FailedRequest {
                e,
                url: obfuscate_account_url(&url),
            });
        }
        Ok(response) => {
            if response.status() != 200 {
                return Err(RequestError::FailedResponse {
                    status: response.status(),
                    url: obfuscate_account_url(&url),
                });
            } else {
                Ok(response)
            }
        }
    }
}

pub(crate) fn obfuscate_account_url(url: impl AsRef<str>) -> String {
    const ACCOUNTS_STR: &str = "accounts/";

    let url = url.as_ref();
    if let Some(accounts_byte_idx) = url.find(ACCOUNTS_STR) {
        let mut ending_separator_found = false;
        url.char_indices()
            .map(|(char_byte_idx, ch)| {
                if char_byte_idx < accounts_byte_idx + ACCOUNTS_STR.len() || ending_separator_found
                {
                    ch
                } else if ch == '/' {
                    ending_separator_found = true;
                    ch
                } else {
                    '*'
                }
            })
            .collect()
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_obfuscate_account_url() {
        assert_eq!(obfuscate_account_url("accounts/123ABC"), "accounts/******");
        assert_eq!(
            obfuscate_account_url("foo/accounts/123AB/bar"),
            "foo/accounts/*****/bar"
        );
    }
}
