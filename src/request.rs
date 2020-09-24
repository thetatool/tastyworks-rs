use lazy_static::lazy_static;
use regex::Regex;
use reqwest::{header, Client};

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

const RESTART_MSG: &str = "Restart tastyworks and check that you are logged in.";

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
            lazy_static! {
                static ref RE: Regex = Regex::new(r#""sessionId"\s*:\s*"([^"]+)"#).unwrap();
            }

            let mut path = dirs::data_local_dir().expect("Undefined config directory");
            path.push("tastyworks/desktop/preferences_user.json");

            let json = std::fs::read_to_string(&path);
            match json {
                Ok(json) => {
                    if let Some(m) = RE.captures(&json).and_then(|caps| caps.get(1)) {
                        t.replace(Some(m.as_str().to_string()));
                    } else {
                        error = Some(TokenParseError.into());
                    }
                }
                Err(_) => {
                    error = Some(IOError { path }.into());
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
        return Err(UnauthorizedRequestError {
            status: response.status(),
            url,
        }
        .into());
    }

    Ok(response)
}

#[derive(Debug)]
struct TokenParseError;

impl Error for TokenParseError {}

impl fmt::Display for TokenParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "API token could not be found. {}", RESTART_MSG)
    }
}

#[derive(Debug)]
struct IOError {
    path: PathBuf,
}

impl Error for IOError {}

impl fmt::Display for IOError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error reading file: {}", self.path.display())
    }
}

#[derive(Debug)]
struct UnauthorizedRequestError {
    status: reqwest::StatusCode,
    url: String,
}

impl Error for UnauthorizedRequestError {}

impl fmt::Display for UnauthorizedRequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Failed response ({}) to request: {}. {}",
            self.status, self.url, RESTART_MSG
        )
    }
}
