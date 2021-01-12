use lazy_static::lazy_static;
use regex::Regex;
use reqwest::{header, Client};

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

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

pub async fn request(
    url_path: &str,
    params_string: &str,
) -> Result<reqwest::Response, Box<dyn Error>> {
    let mut api_token_header_value = None;
    let mut error: Option<Box<dyn Error>> = None;

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
        .await?;

    if response.status() != 200 {
        return Err(FailedRequestError {
            status: response.status(),
            url,
        }
        .into());
    }

    Ok(response)
}

#[derive(Debug)]
pub struct TokenMissingError;

impl Error for TokenMissingError {}

impl fmt::Display for TokenMissingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "API token could not be found. {}", RESTART_MSG)
    }
}

#[derive(Debug)]
pub struct SessionKeyMissingError;

impl Error for SessionKeyMissingError {}

impl fmt::Display for SessionKeyMissingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Session key could not be found. {}", RESTART_MSG)
    }
}

#[derive(Debug)]
pub struct FailedRegexError {
    obfuscated_line: String,
}

impl Error for FailedRegexError {}

impl fmt::Display for FailedRegexError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Preferences json regex failed on line: {}",
            self.obfuscated_line
        )
    }
}

#[derive(Debug)]
pub struct IOError {
    pub path: PathBuf,
}

impl Error for IOError {}

impl fmt::Display for IOError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Error reading file: {}. {}",
            self.path.display(),
            install_msg()
        )
    }
}

#[derive(Debug)]
struct ConfigDirMissingError;

impl Error for ConfigDirMissingError {}

impl fmt::Display for ConfigDirMissingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Configuration directory not found. {}", install_msg())
    }
}

#[derive(Debug)]
pub struct FailedRequestError {
    pub status: reqwest::StatusCode,
    pub url: String,
}

impl Error for FailedRequestError {}

impl fmt::Display for FailedRequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Failed request ({}) to: {}. {}",
            self.status, self.url, RESTART_MSG
        )
    }
}

fn extract_token_from_preferences_file() -> Result<String, Box<dyn Error>> {
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
        .ok_or(ConfigDirMissingError)?;

    path.push("desktop/preferences_user.json");

    let json = std::fs::read_to_string(&path);
    match json {
        Ok(json) => {
            if let Some(m) = RE.captures(&json).and_then(|caps| caps.get(1)) {
                let m_str = m.as_str();
                if m_str.is_empty() {
                    Err(TokenMissingError.into())
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
                    Err(FailedRegexError { obfuscated_line }.into())
                } else {
                    Err(SessionKeyMissingError.into())
                }
            }
        }
        Err(_) => Err(IOError { path }.into()),
    }
}
