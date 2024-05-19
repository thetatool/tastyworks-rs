use std::error::Error;
use std::fmt;

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
    FailedRequest {
        e: reqwest::Error,
        url: String,
    },
    FailedResponse {
        status: reqwest::StatusCode,
        body: String,
        url: String,
    },
    InvalidHeader {
        e: reqwest::header::InvalidHeaderValue,
    },
}

impl From<reqwest::header::InvalidHeaderValue> for RequestError {
    fn from(e: reqwest::header::InvalidHeaderValue) -> Self {
        RequestError::InvalidHeader { e }
    }
}

impl Error for RequestError {}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::FailedRequest { e, url } => {
                write!(f, "Failed request to {}. {}", url, e)
            }
            Self::FailedResponse { status, body, url } => {
                write!(
                    f,
                    "Failed response (status: {}, body: {}) for {}",
                    status, body, url
                )
            }
            Self::InvalidHeader { e } => {
                write!(f, "Invalid header: {}", e)
            }
        }
    }
}
