use crate::{api, context::Context, request::request};

use itertools::Itertools;
use num_rational::Rational64;
use serde::Deserialize;

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use url::Url;

const MAX_SUBSCRIPTION_SIZE: usize = 500;

pub struct Client {
    base_url: String,
    access_token: String,
    client_id: Option<String>,
    socket: Option<tungstenite::protocol::WebSocket<tungstenite::client::AutoStream>>,
    next_message_id: i32,
    subscription_fields: HashMap<String, Vec<String>>,
}

impl Client {
    pub async fn new(context: &Context) -> Result<Self, Box<dyn Error>> {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct Data {
            websocket_url: String,
            token: String,
        }

        let response = request("quote-streamer-tokens", "", context).await?;
        let api::Response { data, .. } = response.json::<api::Response<Data>>().await?;

        let base_url = format!("{}/cometd", data.websocket_url).replace("https", "wss");
        let access_token = data.token;

        Ok(Client {
            base_url,
            access_token,
            client_id: None,
            socket: None,
            next_message_id: 1,
            subscription_fields: HashMap::new(),
        })
    }

    pub fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        log::debug!("Connecting to dxfeed");
        let (socket, response) = tungstenite::connect(Url::parse(&self.base_url)?)?;
        log::debug!("Connected to dxfeed: {}", response.status());

        self.socket = Some(socket);
        self.send_message(&format!(
            r#"
[
  {{
    "ext": {{
      "com.devexperts.auth.AuthToken": "{auth_token}"
    }},
    "id": "{id}",
    "version": "1.0",
    "minimumVersion": "1.0",
    "channel": "/meta/handshake",
    "supportedConnectionTypes": [
      "websocket",
      "long-polling",
      "callback-polling"
    ],
    "advice": {{
      "timeout": 60000,
      "interval": 0
    }}
  }}
]
"#,
            id = self.next_message_id,
            auth_token = self.access_token
        ))?;
        self.next_message_id += 1;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DxFeedConnectResponse {
            client_id: String,
        }

        let msg = self.read_message(true)?.ok_or(ClientIdResponseError)?;
        let msg_json = msg.to_text()?;
        let client_id = serde_json::from_str::<Vec<DxFeedConnectResponse>>(msg_json)?
            .into_iter()
            .next()
            .ok_or(ClientIdResponseError)?
            .client_id;

        log::debug!("Parsed dxfeed clientId: {}", client_id);
        self.client_id = Some(client_id);

        Ok(())
    }

    pub fn add_subscription(
        &mut self,
        name: &str,
        symbols: &[String],
    ) -> Result<(), Box<dyn Error>> {
        if self.socket.is_none() {
            return Err(NotConnectedError.into());
        }

        for chunk in symbols.chunks(MAX_SUBSCRIPTION_SIZE) {
            self.send_message(&format!(
                r#"
[
  {{
    "id": "{id}",
    "channel": "/service/sub",
    "data": {{
      "add": {{
        "{name}": [
          {symbols}
        ]
      }}
    }},
    "clientId": "{client_id}"
  }}
]
"#,
                id = self.next_message_id,
                client_id = self.client_id.as_ref().unwrap(),
                name = name,
                symbols = chunk.iter().map(|s| format!("\"{}\"", s)).join(",")
            ))?;
            self.next_message_id += 1;

            // TODO: replace with something more reliable, probably need to handle heartbeat
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        Ok(())
    }

    pub fn poll_subscriptions(
        &mut self,
    ) -> Result<HashMap<String, SubscriptionData>, Box<dyn Error>> {
        if self.socket.is_none() {
            return Err(NotConnectedError.into());
        }

        let mut subscription_data = HashMap::new();
        while let Some(msg) = self.read_message(false)? {
            let msg_json = msg.to_text()?;
            let result = serde_json::from_str::<Vec<DxFeedData>>(msg_json).unwrap_or_default();

            for data in &result {
                let (name, data_seq) = data.parse_data_seq(&mut self.subscription_fields)?;

                let data = subscription_data
                    .entry(name.to_string())
                    .or_insert(SubscriptionData {
                        subscription_fields: vec![],
                        data_seq: vec![],
                    });

                data.data_seq.append(&mut data_seq.clone());
            }
        }

        for (name, data) in &mut subscription_data {
            data.subscription_fields = self
                .subscription_fields
                .get(name)
                .ok_or_else(|| ResponseParseError("missing subscription fields".to_string()))?
                .clone();
        }

        self.ping()?;

        Ok(subscription_data)
    }

    fn ping(&mut self) -> Result<(), Box<dyn Error>> {
        if self.socket.is_none() {
            return Err(NotConnectedError.into());
        }

        self.send_message(&format!(
            r#"
[
  {{
    "id": "{id}",
    "channel": "/meta/connect",
    "connectionType": "websocket",
    "advice": {{
      "timeout": 0
    }},
    "clientId": "{client_id}"
  }}
]
"#,
            id = self.next_message_id,
            client_id = self.client_id.as_ref().unwrap(),
        ))?;
        self.next_message_id += 1;

        Ok(())
    }

    fn send_message(&mut self, msg: &str) -> Result<(), Box<dyn Error>> {
        let socket = self.socket.as_mut().ok_or(NotConnectedError)?;
        let msg = msg.replace("\n", "").replace(" ", "");
        log::debug!("Sending message: {}", msg);
        socket
            .write_message(tungstenite::Message::Text(msg))
            .map_err(Into::into)
    }

    fn read_message(
        &mut self,
        blocking: bool,
    ) -> Result<Option<tungstenite::Message>, Box<dyn Error>> {
        let socket = self.socket.as_mut().ok_or(NotConnectedError)?;

        // see https://github.com/snapview/tungstenite-rs/issues/103
        let stream = match socket.get_mut() {
            tungstenite::stream::Stream::Plain(stream) => stream,
            tungstenite::stream::Stream::Tls(stream) => stream.get_mut(),
        };
        stream.set_nonblocking(!blocking)?;

        let message = socket.read_message();
        match message {
            Ok(msg) => {
                log::debug!("Received message: {}", msg);
                Ok(Some(msg))
            }
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[derive(Debug)]
pub struct SubscriptionData {
    subscription_fields: Vec<String>,
    data_seq: Vec<serde_json::Value>,
}

impl SubscriptionData {
    pub fn iter_field(&self, field: &str) -> impl Iterator<Item = &serde_json::Value> + '_ {
        let index = self
            .subscription_fields
            .iter()
            .position(|f| f == field)
            .unwrap_or_else(|| panic!("Missing index for field: {}", field));

        self.data_seq
            .chunks(self.subscription_fields.len())
            .map(move |chunk| &chunk[index])
    }
}

pub trait SubscriptionValue {
    fn to_price(&self) -> Option<Rational64>;
}

impl SubscriptionValue for serde_json::Value {
    fn to_price(&self) -> Option<Rational64> {
        if let Some("NaN") = self.as_str() {
            None
        } else {
            self.as_f64().and_then(Rational64::approximate_float)
        }
    }
}

#[derive(Debug)]
pub struct Price {
    pub symbol: String,
    pub price: Rational64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DxFeedData {
    data: Vec<serde_json::Value>,
}

impl DxFeedData {
    fn parse_data_seq(
        &self,
        subscription_fields: &mut HashMap<String, Vec<String>>,
    ) -> Result<(String, &Vec<serde_json::Value>), ResponseParseError> {
        if self.data.len() != 2 {
            return Err(ResponseParseError("data length".to_string()));
        }

        // the first data frame includes both the field names and first data sequence.
        // subsequent frames contain the remaining data sequences without field names.
        let has_header = self
            .data
            .get(0)
            .filter(|header| header.is_array())
            .is_some();

        if has_header {
            let name = self
                .data
                .get(0)
                .and_then(|header| header.as_array())
                .and_then(|header| {
                    match (
                        header.get(0).and_then(|v| v.as_str()),
                        header.get(1).and_then(|v| v.as_array()),
                    ) {
                        (Some(name), Some(fields)) => {
                            let mut arr_str = vec![];
                            for v in fields {
                                if let Some(s) = v.as_str() {
                                    arr_str.push(s.to_string());
                                } else {
                                    return None;
                                }
                            }
                            subscription_fields.insert(name.to_string(), arr_str);
                            Some(name)
                        }
                        _ => None,
                    }
                })
                .ok_or_else(|| ResponseParseError("header name".to_string()))?;
            let data_seq = self
                .data
                .get(1)
                .and_then(|seq| seq.as_array())
                .ok_or_else(|| ResponseParseError("header data seq".to_string()))?;

            Ok((name.to_string(), data_seq))
        } else {
            let name = self
                .data
                .get(0)
                .and_then(|name| name.as_str())
                .ok_or_else(|| ResponseParseError("name".to_string()))?;
            let data_seq = self
                .data
                .get(1)
                .and_then(|seq| seq.as_array())
                .ok_or_else(|| ResponseParseError("data seq".to_string()))?;
            Ok((name.to_string(), data_seq))
        }
    }
}

#[derive(Debug, Clone)]
struct ClientIdResponseError;

impl Error for ClientIdResponseError {}

impl fmt::Display for ClientIdResponseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Could not find clientId in response")
    }
}

#[derive(Debug, Clone)]
struct NotConnectedError;

impl Error for NotConnectedError {}

impl fmt::Display for NotConnectedError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "The streamer client is not connected")
    }
}

#[derive(Debug, Clone)]
struct ResponseParseError(String);

impl Error for ResponseParseError {}

impl fmt::Display for ResponseParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Response could not be parsed: {}", self.0)
    }
}
