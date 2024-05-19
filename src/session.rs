use crate::{
    api::{self, *},
    errors::*,
    request::*,
};

use reqwest::{header, Method};

use std::collections::HashMap;

pub struct Session {
    pub(crate) token: String,
}

impl Session {
    pub fn from_token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub async fn from_credentials(
        login: impl AsRef<str>,
        password: impl AsRef<str>,
        otp: Option<impl AsRef<str>>,
    ) -> Result<Self, ApiError> {
        let mut map = HashMap::new();
        map.insert("login", login.as_ref());
        map.insert("password", password.as_ref());
        let json = serde_json::to_string(&map).unwrap();
        let url = format!("{}/sessions", BASE_URL);
        let mut request = build_request(&url, Method::POST).body(json);
        if let Some(otp) = otp {
            let mut otp_header_value =
                header::HeaderValue::from_str(otp.as_ref()).map_err(Into::<RequestError>::into)?;
            otp_header_value.set_sensitive(true);
            request = request.header("X-Tastyworks-OTP", otp_header_value);
        }
        let request_result = map_result(&url, request.send().await).await?;
        let response: api::Response<sessions::Response> =
            deserialize_response(request_result).await?;
        Ok(Session {
            token: response.data.session_token,
        })
    }
}
