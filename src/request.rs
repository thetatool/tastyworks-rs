use crate::{
    constants::{API_ENV_KEY, SESSION_ID_KEY},
    errors::RequestError,
};

use lazy_static::lazy_static;
use regex::Regex;
use reqwest::{header, Client};

use std::path::PathBuf;

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

thread_local! {
    static API_TOKEN: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}

pub async fn request(
    url_path: &str,
    params_string: &str,
) -> Result<reqwest::Response, RequestError> {
    let mut api_token_header_value = None;
    let mut error: Option<RequestError> = None;

    API_TOKEN.with(|t| {
        if t.borrow().is_none() {
            if let Ok(token) = std::env::var(API_ENV_KEY) {
                t.replace(Some(token));
            } else {
                match extract_token_from_preferences_file() {
                    Ok(token) => {
                        t.replace(Some(token));
                    }
                    Err(e) => {
                        error = Some(e);
                    }
                }
            }
        };

        if let Some(t) = &*t.borrow() {
            let mut value = header::HeaderValue::from_str(&t).unwrap();
            value.set_sensitive(true);
            api_token_header_value = Some(value);
        }
    });

    if let Some(error) = error {
        return Err(error);
    }

    let params_string = if params_string.is_empty() {
        params_string.to_string()
    } else {
        format!("?{}", params_string)
    };

    let base_url = format!("https://api.tastyworks.com/{}", url_path);
    let url = format!("{}{}", base_url, params_string);

    let response = CLIENT
        .get(&url)
        .header(header::AUTHORIZATION, api_token_header_value.unwrap())
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

fn extract_token_from_preferences_file() -> Result<String, RequestError> {
    lazy_static! {
        static ref RE: Regex =
            Regex::new(&format!(r#""{}"\s*:\s*"([^"]*)"#, SESSION_ID_KEY)).unwrap();
    }

    let mut path: PathBuf = [dirs::home_dir(), dirs::data_local_dir()]
        .iter()
        .flatten()
        .filter_map(|p| {
            let mut p = p.to_path_buf();
            for f in &["tastyworks", ".tastyworks"] {
                p.push(f);
                if p.exists() {
                    return Some(p);
                }
                p.pop();
            }
            None
        })
        .next()
        .ok_or(RequestError::ConfigDirMissing)?;

    path.push("desktop/preferences_user.json");

    let json = std::fs::read_to_string(&path);
    match json {
        Ok(json) => {
            if let Some(m) = RE.captures(&json).and_then(|caps| caps.get(1)) {
                let m_str = m.as_str();
                if m_str.is_empty() {
                    Err(RequestError::TokenMissing)
                } else {
                    Ok(m_str.to_string())
                }
            } else {
                let line = json.lines().find(|line| line.contains(SESSION_ID_KEY));
                if let Some(line) = line {
                    let start_idx = line.find(SESSION_ID_KEY).unwrap();
                    let end_idx = start_idx + SESSION_ID_KEY.len();
                    let obfuscated_line: String = line
                        .char_indices()
                        .map(|(idx, c)| {
                            if c.is_alphanumeric() && (idx < start_idx || idx >= end_idx) {
                                '*'
                            } else {
                                c
                            }
                        })
                        .collect();
                    Err(RequestError::FailedRegex { obfuscated_line })
                } else {
                    Err(RequestError::SessionKeyMissing)
                }
            }
        }
        Err(_) => Err(RequestError::Io { path }),
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
