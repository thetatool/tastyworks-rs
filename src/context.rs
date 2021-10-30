use crate::{constants::SESSION_ID_KEY, errors::ApiError};

use lazy_static::lazy_static;
use regex::Regex;

use std::path::PathBuf;

#[derive(Debug)]
pub struct Context {
    pub(crate) token: String,
}

impl Context {
    /// Creates a context using the provided token.
    pub fn from_token(token: &str) -> Self {
        Context {
            token: token.to_string(),
        }
    }

    /// Creates a context by extracting the token from the installed version of tastyworks.
    ///
    /// Returns an error if the token can not be found, for example if tastyworks is not installed or logged in.
    pub fn from_installed_token() -> Result<Self, ApiError> {
        Ok(Context {
            token: extract_token_from_preferences_file()?,
        })
    }
}

fn extract_token_from_preferences_file() -> Result<String, ApiError> {
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
        .ok_or(ApiError::ConfigDirMissing)?;

    path.push("desktop/preferences_user.json");

    let json = std::fs::read_to_string(&path);
    match json {
        Ok(json) => {
            if let Some(m) = RE.captures(&json).and_then(|caps| caps.get(1)) {
                let m_str = m.as_str();
                if m_str.is_empty() {
                    Err(ApiError::TokenMissing)
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
                    Err(ApiError::FailedRegex { obfuscated_line })
                } else {
                    Err(ApiError::SessionKeyMissing)
                }
            }
        }
        Err(_) => Err(ApiError::Io { path }),
    }
}
