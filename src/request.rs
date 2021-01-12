use lazy_static::lazy_static;
use regex::Regex;
use reqwest::{header, Client};

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub use reqwest::StatusCode;

const SESSION_ID_KEY: &str = "sessionId";
const API_ENV_KEY: &str = "TASTYWORKS_API_TOKEN";

const RESTART_MSG: &str =
    "Try restarting tastyworks desktop and logging in, even if you are currently logged in.";

fn install_msg() -> String {
    format!("Ensure that tastyworks desktop is installed or define a valid token in the {} environment variable.", API_ENV_KEY)
}

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

#[derive(Debug)]
pub enum RequestError {
    TokenMissing,
    SessionKeyMissing,
    FailedRegex {
        obfuscated_line: String,
    },
    Io {
        path: PathBuf,
    },
    ConfigDirMissing,
    Decode {
        e: reqwest::Error,
        url: String,
    },
    FailedRequest {
        e: reqwest::Error,
        url: String,
    },
    FailedResponse {
        status: reqwest::StatusCode,
        url: String,
    },
}

impl Error for RequestError {}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::TokenMissing => {
                write!(f, "API token could not be found. {}", RESTART_MSG)
            }
            Self::SessionKeyMissing => {
                write!(f, "Session key could not be found. {}", RESTART_MSG)
            }
            Self::FailedRegex { obfuscated_line } => {
                write!(
                    f,
                    "Preferences json regex failed on line: {}",
                    obfuscated_line
                )
            }
            Self::Io { path } => {
                write!(
                    f,
                    "Error reading file: {}. {}",
                    path.display(),
                    install_msg()
                )
            }
            Self::ConfigDirMissing => {
                write!(f, "Configuration directory not found. {}", install_msg())
            }
            Self::Decode { e, url } => {
                write!(f, "Error decoding {}. {}", url, e)
            }
            Self::FailedRequest { e, url } => {
                write!(f, "Failed request to {}. {}", url, e)
            }
            Self::FailedResponse { status, url } => {
                write!(
                    f,
                    "Failed response ({}) for: {}. {}",
                    status, url, RESTART_MSG
                )
            }
        }
    }
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
            return Err(RequestError::FailedRequest { e, url });
        }
        Ok(response) => {
            if response.status() != 200 {
                return Err(RequestError::FailedResponse {
                    status: response.status(),
                    url,
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
