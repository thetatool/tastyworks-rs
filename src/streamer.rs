use crate::{api, request::request, session::Session};

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
    token: String,
    socket: Option<tungstenite::protocol::WebSocket<tungstenite::client::AutoStream>>,
    feed_channel: Option<i32>,
    subscription_fields: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(rename = "type")]
    message_type: String,
}

#[derive(Debug, Deserialize)]
struct ChannelOpenedMessage {
    #[serde(rename = "type")]
    message_type: String,
    channel: i32,
}

impl Client {
    pub async fn new(session: &Session) -> Result<Self, Box<dyn Error>> {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct Data {
            dxlink_url: String,
            token: String,
        }

        let response = request("api-quote-tokens", "", session).await?;
        let api::Response { data, .. } = response.json::<api::Response<Data>>().await?;

        let base_url = data.dxlink_url;
        let token = data.token;

        Ok(Client {
            base_url,
            token,
            socket: None,
            feed_channel: None,
            subscription_fields: HashMap::new(),
        })
    }

    pub fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        log::debug!("Connecting to dxfeed");
        let (socket, response) = tungstenite::connect(Url::parse(&self.base_url)?)?;
        log::debug!("Connected to dxfeed: {}", response.status());

        self.socket = Some(socket);
        self.send_message(
            r#"
{
  "type": "SETUP",
  "channel": 0,
  "keepaliveTimeout": 60,
  "acceptKeepaliveTimeout": 60,
  "version": "0.1-js/1.0.0"
}
"#,
        )?;
        let msg = self.read_message(true)?.ok_or(ReadMessageError)?;
        let msg_json = msg.to_text()?;
        let _ = match serde_json::from_str::<Message>(msg_json) {
            Ok(response) if response.message_type == "SETUP" => response,
            _ => return Err(ResponseParseError("SETUP".to_string()).into()),
        };
        // flush remaining messages e.g. unauthorized auth message
        while let Some(_) = self.read_message(false)? {}

        self.send_message(&format!(
            r#"
{{
  "type": "AUTH",
  "channel": 0,
  "token": "{}"
}}
"#,
            self.token,
        ))?;

        #[derive(Deserialize)]
        struct AuthResponse {
            state: String,
        }

        let msg = self.read_message(true)?.ok_or(ReadMessageError)?;
        let msg_json = msg.to_text()?;
        let auth_response = serde_json::from_str::<AuthResponse>(msg_json)
            .or(Err(ResponseParseError("AUTH".to_string())))?;
        if auth_response.state != "AUTHORIZED" {
            return Err(NotAuthorizedError.into());
        }
        Ok(())
    }

    pub fn add_subscription(
        &mut self,
        name: &str,
        fields: &[String],
        symbols: &[String],
    ) -> Result<(), Box<dyn Error>> {
        if self.socket.is_none() {
            return Err(NotConnectedError.into());
        }

        if self.feed_channel.is_none() {
            self.send_message(
                r#"
{
  "type": "CHANNEL_REQUEST",
  "channel": 1,
  "service": "FEED",
  "parameters": {
    "contract": "AUTO"
  }
}
"#,
            )?;
            let msg = self.read_message(true)?.ok_or(ReadMessageError)?;
            let msg_json = msg.to_text()?;
            let response = match serde_json::from_str::<ChannelOpenedMessage>(msg_json) {
                Ok(response) if response.message_type == "CHANNEL_OPENED" => response,
                _ => return Err(ResponseParseError("CHANNEL_OPENED".to_string()).into()),
            };
            self.feed_channel = Some(response.channel);
        }

        if !self.subscription_fields.contains_key(name) {
            self.send_message(&format!(
                r#"
{{
  "type": "FEED_SETUP",
  "channel": {channel},
  "acceptAggregationPeriod": 10,
  "acceptDataFormat": "COMPACT",
  "acceptEventFields": {{
    "{name}": ["{fields}"]
  }}
}}
"#,
                channel = self.feed_channel.unwrap(),
                name = name,
                fields = fields.join("\",\"")
            ))?;
            self.subscription_fields
                .insert(name.to_string(), fields.to_vec());
        }

        for chunk in symbols.chunks(MAX_SUBSCRIPTION_SIZE) {
            self.send_message(&format!(
                r#"
{{
  "type": "FEED_SUBSCRIPTION",
  "channel": {channel},
  "add": [{add}]
}}
"#,
                channel = self.feed_channel.unwrap(),
                add = chunk
                    .iter()
                    .map(|s| format!(r#"{{"type":"{}","symbol":"{}"}}"#, name, s))
                    .join(",")
            ))?;
            // TODO: replace with something more reliable
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

        let mut new_subscription_data = HashMap::new();
        while let Some(msg) = self.read_message(false)? {
            let msg_json = msg.to_text()?;
            let mut feed_data = if let Ok(data) = serde_json::from_str::<DxFeedData>(msg_json) {
                data
            } else {
                continue;
            };
            let name = feed_data
                .data
                .get(0)
                .and_then(|name| name.as_str())
                .map(String::from)
                .ok_or_else(|| ResponseParseError("name".to_string()))?;
            let data_seq = feed_data
                .data
                .get_mut(1)
                .and_then(|seq| seq.as_array_mut())
                .ok_or_else(|| ResponseParseError("data seq".to_string()))?;
            let subscription_fields = self
                .subscription_fields
                .get(&name)
                .ok_or_else(|| ResponseParseError("missing subscription fields".to_string()))?;
            new_subscription_data
                .entry(name)
                .or_insert(SubscriptionData {
                    subscription_fields: subscription_fields.clone(),
                    data_seq: vec![],
                })
                .data_seq
                .append(data_seq);
        }

        self.keep_alive()?;

        Ok(new_subscription_data)
    }

    fn keep_alive(&mut self) -> Result<(), Box<dyn Error>> {
        if self.socket.is_none() {
            return Err(NotConnectedError.into());
        }
        self.send_message(
            r#"
{
  "type": "KEEPALIVE",
  "channel": 0
}
"#,
        )?;
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
struct DxFeedData {
    data: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct NotAuthorizedError;

impl Error for NotAuthorizedError {}

impl fmt::Display for NotAuthorizedError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Failed to AUTHORIZE")
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
struct ReadMessageError;

impl Error for ReadMessageError {}

impl fmt::Display for ReadMessageError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Failed to read message")
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
