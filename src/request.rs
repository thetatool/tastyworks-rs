use crate::{
    errors::{ApiError, RequestError},
    session::Session,
};

use lazy_static::lazy_static;
use reqwest::{header, Client, Method};

pub use reqwest::StatusCode;

pub(crate) const BASE_URL: &str = "https://api.tastyworks.com";
const VERSION: &str = env!("CARGO_PKG_VERSION");

lazy_static! {
    static ref CLIENT: Client = Client::builder()
        .user_agent(format!("tasyworks-rs/{}", VERSION))
        .build()
        .unwrap();
}

pub async fn request(
    url_path: &str,
    params_string: &str,
    session: &Session,
) -> Result<reqwest::Response, RequestError> {
    let mut api_token_header_value = header::HeaderValue::from_str(&session.token).unwrap();
    api_token_header_value.set_sensitive(true);

    let params_string = if params_string.is_empty() {
        params_string.to_string()
    } else {
        format!("?{}", params_string)
    };

    let url = &format!("{}/{}{}", BASE_URL, url_path, params_string);
    let response = build_request(&url, Method::GET)
        .header(header::AUTHORIZATION, api_token_header_value)
        .send()
        .await;

    map_result(&url, response).await
}

pub(crate) fn build_request(url: &str, method: Method) -> reqwest::RequestBuilder {
    CLIENT
        .request(method, url)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
}

pub(crate) async fn map_result(
    url: &str,
    result: Result<reqwest::Response, reqwest::Error>,
) -> Result<reqwest::Response, RequestError> {
    match result {
        Err(e) => {
            return Err(RequestError::FailedRequest {
                e,
                url: obfuscate_account_url(url),
            });
        }
        Ok(response) => {
            if response.status() == 200 || response.status() == 201 {
                Ok(response)
            } else {
                return Err(RequestError::FailedResponse {
                    status: response.status(),
                    body: response.text().await.unwrap_or_else(|e| e.to_string()),
                    url: obfuscate_account_url(url),
                });
            }
        }
    }
}

pub(crate) async fn deserialize_response<T>(response: reqwest::Response) -> Result<T, ApiError>
where
    T: serde::de::DeserializeOwned,
{
    let url = response.url().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| RequestError::FailedRequest {
            e,
            url: obfuscate_account_url(&url),
        })?;

    let de = &mut serde_json::Deserializer::from_slice(&bytes);
    let result: Result<T, _> = serde_path_to_error::deserialize(de);
    result.map_err(|e| ApiError::Decode {
        e: Box::new(e),
        url: obfuscate_account_url(&url),
    })
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
