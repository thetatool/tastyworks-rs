use crate::constants::API_ENV_KEY;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

const RESTART_MSG: &str =
    "Try restarting tastyworks desktop and logging in, even if you are currently logged in.";

fn install_msg() -> String {
    format!("Ensure that tastyworks desktop is installed or define a valid token in the {} environment variable.", API_ENV_KEY)
}

#[derive(Debug)]
pub enum ApiError {
    Request(RequestError),
    Decode { e: Box<dyn Error>, url: String },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Request(e) => {
                write!(f, "{}", e)
            }
            Self::Decode { e, url } => {
                write!(f, "Error decoding {}. {}", url, e)
            }
        }
    }
}

impl Error for ApiError {}

impl From<RequestError> for ApiError {
    fn from(e: RequestError) -> Self {
        ApiError::Request(e)
    }
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
