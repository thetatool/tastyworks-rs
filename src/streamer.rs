// see https://developer.tastytrade.com/streaming-market-data/

use crate::{api, request::request, session::Session};

use num_rational::Rational64;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};
use url::Url;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const QUOTE_TOKENS_ENDPOINT: &str = "api-quote-tokens";

type Socket = tungstenite::protocol::WebSocket<tungstenite::client::AutoStream>;

pub struct Client {
    base_url: String,
    token: String,
    transport: Option<Transport>,
    feed_channel: Option<i32>,
    subscription_fields: HashMap<String, Vec<String>>,
    last_keepalive_at: Option<Instant>,
}

impl Client {
    pub async fn new(session: &Session) -> Result<Self, Box<dyn Error>> {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct Data {
            dxlink_url: String,
            token: String,
        }

        let response = request(QUOTE_TOKENS_ENDPOINT, "", session).await?;
        let api::Response { data, .. } = response.json::<api::Response<Data>>().await?;

        Ok(Client {
            base_url: data.dxlink_url,
            token: data.token,
            transport: None,
            feed_channel: None,
            subscription_fields: HashMap::new(),
            last_keepalive_at: None,
        })
    }

    pub fn connect(&mut self) -> Result<(), StreamerError> {
        self.reset_connection_state();

        let mut transport = Transport::connect(&self.base_url)?;
        complete_handshake(&mut transport, &self.token)?;

        self.transport = Some(transport);
        self.last_keepalive_at = Some(Instant::now());

        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.reset_connection_state();
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    pub fn add_subscription(
        &mut self,
        name: &str,
        fields: &[String],
        symbols: &[String],
    ) -> Result<(), StreamerError> {
        if self.feed_channel.is_none() {
            let feed_channel = self.open_feed_channel(1)?;
            self.feed_channel = Some(feed_channel);
        }

        let feed_channel = self.feed_channel.expect("missing feed channel");

        if !self.subscription_fields.contains_key(name) {
            self.transport_mut()?
                .send_json(&protocol::feed_setup_message(feed_channel, name, fields))?;
            self.subscription_fields
                .insert(name.to_string(), fields.to_vec());
        }

        for message in protocol::feed_subscription_messages(feed_channel, name, symbols) {
            self.transport_mut()?.send_text(&message)?;
        }

        Ok(())
    }

    pub fn poll_subscriptions(
        &mut self,
    ) -> Result<HashMap<String, SubscriptionData>, StreamerError> {
        let mut new_subscription_data = HashMap::new();
        while let Some(msg_json) = self.transport_mut()?.read_text_message(false)? {
            if !accumulate_feed_data(
                &self.subscription_fields,
                &mut new_subscription_data,
                &msg_json,
            )? {
                continue;
            }
        }

        self.keep_alive()?;

        Ok(new_subscription_data)
    }

    fn reset_connection_state(&mut self) {
        self.transport = None;
        self.feed_channel = None;
        self.subscription_fields.clear();
        self.last_keepalive_at = None;
    }

    fn open_feed_channel(&mut self, channel: i32) -> Result<i32, StreamerError> {
        self.transport_mut()?
            .send_json(&protocol::feed_channel_request_message(channel))?;
        let msg_json = self.transport_mut()?.read_required_text_message()?;
        protocol::parse_channel_opened_message(&msg_json)
    }

    fn keep_alive(&mut self) -> Result<(), StreamerError> {
        if self
            .last_keepalive_at
            .map(|last_keepalive_at| last_keepalive_at.elapsed() < KEEPALIVE_INTERVAL)
            .unwrap_or(false)
        {
            return Ok(());
        }

        self.transport_mut()?
            .send_json(&protocol::keepalive_message())?;
        self.last_keepalive_at = Some(Instant::now());
        Ok(())
    }

    fn transport_mut(&mut self) -> Result<&mut Transport, StreamerError> {
        self.transport.as_mut().ok_or(StreamerError::NotConnected)
    }
}

struct Transport {
    socket: Socket,
}

impl Transport {
    fn connect(base_url: &str) -> Result<Self, StreamerError> {
        log::debug!("Connecting to dxfeed");
        let (socket, response) = tungstenite::connect(Url::parse(base_url)?)?;
        log::debug!("Connected to dxfeed: {}", response.status());

        Ok(Self { socket })
    }

    fn send_json(&mut self, msg: &Value) -> Result<(), StreamerError> {
        let msg = serde_json::to_string(msg)?;
        self.send_text(&msg)
    }

    fn send_text(&mut self, msg: &str) -> Result<(), StreamerError> {
        log::debug!("Sending message: {}", msg);
        self.socket
            .write_message(tungstenite::Message::Text(msg.to_string()))?;
        Ok(())
    }

    fn read_required_text_message(&mut self) -> Result<String, StreamerError> {
        self.read_text_message(true)?
            .ok_or(StreamerError::ReadMessage)
    }

    fn read_text_message(&mut self, blocking: bool) -> Result<Option<String>, StreamerError> {
        let msg = match self.read_message(blocking)? {
            Some(msg) => msg,
            None => return Ok(None),
        };

        Ok(Some(msg.to_text()?.to_string()))
    }

    fn read_message(
        &mut self,
        blocking: bool,
    ) -> Result<Option<tungstenite::Message>, StreamerError> {
        // see https://github.com/snapview/tungstenite-rs/issues/103
        let stream = match self.socket.get_mut() {
            tungstenite::stream::Stream::Plain(stream) => stream,
            tungstenite::stream::Stream::Tls(stream) => stream.get_mut(),
        };
        stream.set_nonblocking(!blocking)?;

        match self.socket.read_message() {
            Ok(tungstenite::Message::Close(_)) => Err(StreamerError::Disconnected),
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

fn complete_handshake(transport: &mut Transport, token: &str) -> Result<(), StreamerError> {
    transport.send_json(&protocol::setup_message())?;

    let mut handshake = protocol::HandshakeState::default();
    loop {
        let msg_json = transport.read_required_text_message()?;
        match handshake.observe(&msg_json)? {
            protocol::HandshakeAction::Ignore => {}
            protocol::HandshakeAction::SendAuth => {
                transport.send_json(&protocol::auth_message(token))?;
            }
            protocol::HandshakeAction::Ready => return Ok(()),
        }
    }
}

fn accumulate_feed_data(
    subscription_fields: &HashMap<String, Vec<String>>,
    new_subscription_data: &mut HashMap<String, SubscriptionData>,
    msg_json: &str,
) -> Result<bool, StreamerError> {
    // In COMPACT mode dxlink sends `data` as `[event_type, flat_field_values...]`. We keep the
    // caller-provided field list per subscription and chunk the flat value sequence back into rows
    // when iterating fields later.
    let mut feed_data = if let Some(data) = protocol::parse_compact_feed_data(msg_json)? {
        data
    } else {
        return Ok(false);
    };

    let subscription_fields = subscription_fields
        .get(&feed_data.name)
        .ok_or_else(|| StreamerError::ResponseParse("missing subscription fields".to_string()))?;

    new_subscription_data
        .entry(feed_data.name)
        .or_insert(SubscriptionData {
            subscription_fields: subscription_fields.clone(),
            data_seq: vec![],
        })
        .data_seq
        .append(&mut feed_data.data_seq);

    Ok(true)
}

pub struct SubscriptionData {
    subscription_fields: Vec<String>,
    data_seq: Vec<Value>,
}

impl SubscriptionData {
    pub fn iter_field(&self, field: &str) -> impl Iterator<Item = &Value> + '_ {
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

impl SubscriptionValue for Value {
    fn to_price(&self) -> Option<Rational64> {
        if let Some("NaN") = self.as_str() {
            None
        } else {
            self.as_f64().and_then(Rational64::approximate_float)
        }
    }
}

#[derive(Debug)]
pub enum StreamerError {
    NotConnected,
    Disconnected,
    ReadMessage,
    ResponseParse(String),
    Json(serde_json::Error),
    Url(url::ParseError),
    WebSocket(tungstenite::Error),
    Io(std::io::Error),
}

impl StreamerError {
    pub fn is_disconnect(&self) -> bool {
        matches!(self, Self::NotConnected | Self::Disconnected)
    }
}

impl Error for StreamerError {}

impl fmt::Display for StreamerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::NotConnected => write!(f, "The streamer client is not connected"),
            Self::Disconnected => write!(f, "The streamer connection was closed"),
            Self::ReadMessage => write!(f, "Failed to read message"),
            Self::ResponseParse(field) => write!(f, "Response could not be parsed: {}", field),
            Self::Json(e) => write!(f, "{}", e),
            Self::Url(e) => write!(f, "{}", e),
            Self::WebSocket(e) => write!(f, "{}", e),
            Self::Io(e) => write!(f, "{}", e),
        }
    }
}

impl From<serde_json::Error> for StreamerError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<url::ParseError> for StreamerError {
    fn from(e: url::ParseError) -> Self {
        Self::Url(e)
    }
}

impl From<tungstenite::Error> for StreamerError {
    fn from(e: tungstenite::Error) -> Self {
        match e {
            tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
                Self::Disconnected
            }
            tungstenite::Error::Io(e) => Self::from(e),
            e => Self::WebSocket(e),
        }
    }
}

impl From<std::io::Error> for StreamerError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof => Self::Disconnected,
            _ => Self::Io(e),
        }
    }
}

mod protocol {
    use super::*;

    const CONTROL_CHANNEL: i32 = 0;
    const FEED_SERVICE: &str = "FEED";
    const FEED_CONTRACT: &str = "AUTO";
    const KEEPALIVE_TIMEOUT_SECS: i32 = 60;
    const MAX_SEND_SUBSCRIPTION_BYTE_SIZE: usize = 8192;
    const VERSION: &str = "0.1-DXF-JS/0.3.0";

    #[derive(Debug, Deserialize)]
    struct Message {
        #[serde(rename = "type")]
        message_type: String,
    }

    #[derive(Debug, Deserialize)]
    struct AuthStateMessage {
        state: String,
    }

    #[derive(Debug, Deserialize)]
    struct ChannelOpenedMessage {
        #[serde(rename = "type")]
        message_type: String,
        channel: i32,
    }

    #[derive(Debug, Deserialize)]
    struct FeedDataMessage {
        #[serde(rename = "type")]
        message_type: String,
        data: Vec<Value>,
    }

    #[derive(Debug, Serialize)]
    struct SubscriptionEntry<'a> {
        #[serde(rename = "type")]
        event_type: &'a str,
        symbol: &'a str,
    }

    #[derive(Debug)]
    enum ControlMessage {
        Setup,
        AuthUnauthorized,
        AuthAuthorized,
        Other,
    }

    #[derive(Debug, Default)]
    pub(super) struct HandshakeState {
        setup_received: bool,
        unauthorized_received: bool,
        auth_sent: bool,
        authorized: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum HandshakeAction {
        Ignore,
        SendAuth,
        Ready,
    }

    impl HandshakeState {
        pub(super) fn observe(&mut self, msg_json: &str) -> Result<HandshakeAction, StreamerError> {
            match parse_control_message(msg_json)? {
                ControlMessage::Setup => {
                    self.setup_received = true;
                }
                ControlMessage::AuthUnauthorized => {
                    self.unauthorized_received = true;
                }
                ControlMessage::AuthAuthorized => {
                    self.authorized = true;
                }
                ControlMessage::Other => {}
            }

            if self.is_ready() {
                return Ok(HandshakeAction::Ready);
            }

            if self.can_authorize() && !self.auth_sent {
                self.auth_sent = true;
                return Ok(HandshakeAction::SendAuth);
            }

            Ok(HandshakeAction::Ignore)
        }

        pub(super) fn can_authorize(&self) -> bool {
            self.setup_received && self.unauthorized_received
        }

        pub(super) fn is_ready(&self) -> bool {
            self.auth_sent && self.authorized
        }
    }

    #[derive(Debug)]
    pub(super) struct CompactFeedData {
        pub(super) name: String,
        pub(super) data_seq: Vec<Value>,
    }

    pub(super) fn setup_message() -> Value {
        json!({
            "type": "SETUP",
            "channel": CONTROL_CHANNEL,
            "keepaliveTimeout": KEEPALIVE_TIMEOUT_SECS,
            "acceptKeepaliveTimeout": KEEPALIVE_TIMEOUT_SECS,
            "version": VERSION,
        })
    }

    pub(super) fn auth_message(token: &str) -> Value {
        json!({
            "type": "AUTH",
            "channel": CONTROL_CHANNEL,
            "token": token,
        })
    }

    pub(super) fn feed_channel_request_message(channel: i32) -> Value {
        json!({
            "type": "CHANNEL_REQUEST",
            "channel": channel,
            "service": FEED_SERVICE,
            "parameters": {
                "contract": FEED_CONTRACT,
            },
        })
    }

    pub(super) fn feed_setup_message(channel: i32, name: &str, fields: &[String]) -> Value {
        json!({
            "type": "FEED_SETUP",
            "channel": channel,
            "acceptAggregationPeriod": 0.1,
            "acceptDataFormat": "COMPACT",
            "acceptEventFields": {
                name: fields,
            },
        })
    }

    pub(super) fn feed_subscription_messages(
        channel: i32,
        name: &str,
        symbols: &[String],
    ) -> Vec<String> {
        let mut messages = vec![];
        let mut add = vec![];
        let mut size = feed_subscription_message(channel, &[]).len();

        for symbol in symbols {
            let entry = serde_json::to_string(&SubscriptionEntry {
                event_type: name,
                symbol,
            })
            .expect("feed subscription entry should serialize");
            // Every entry after the first adds one extra byte for the separating comma in `add:[...]`.
            let entry_size = entry.len() + usize::from(!add.is_empty());

            if size + entry_size > MAX_SEND_SUBSCRIPTION_BYTE_SIZE && !add.is_empty() {
                messages.push(feed_subscription_message(channel, &add));
                add.clear();
                size = feed_subscription_message(channel, &[]).len();
            }

            size += entry_size;
            add.push(entry);
        }

        if !add.is_empty() {
            messages.push(feed_subscription_message(channel, &add));
        }

        messages
    }

    pub(super) fn keepalive_message() -> Value {
        json!({
            "type": "KEEPALIVE",
            "channel": CONTROL_CHANNEL,
        })
    }

    pub(super) fn parse_channel_opened_message(msg_json: &str) -> Result<i32, StreamerError> {
        match serde_json::from_str::<ChannelOpenedMessage>(msg_json) {
            Ok(response) if response.message_type == "CHANNEL_OPENED" => Ok(response.channel),
            _ => Err(StreamerError::ResponseParse("CHANNEL_OPENED".to_string())),
        }
    }

    pub(super) fn parse_compact_feed_data(
        msg_json: &str,
    ) -> Result<Option<CompactFeedData>, StreamerError> {
        let mut feed_data = match serde_json::from_str::<FeedDataMessage>(msg_json) {
            Ok(feed_data) if feed_data.message_type == "FEED_DATA" => feed_data,
            _ => return Ok(None),
        };

        let name = feed_data
            .data
            .get(0)
            .and_then(|name| name.as_str())
            .map(String::from)
            .ok_or_else(|| StreamerError::ResponseParse("name".to_string()))?;
        let data_seq = feed_data
            .data
            .get_mut(1)
            .and_then(|seq| seq.as_array_mut())
            .ok_or_else(|| StreamerError::ResponseParse("data seq".to_string()))?;

        Ok(Some(CompactFeedData {
            name,
            data_seq: std::mem::take(data_seq),
        }))
    }

    fn feed_subscription_message(channel: i32, add: &[String]) -> String {
        let mut message = format!(
            r#"{{"type":"FEED_SUBSCRIPTION","channel":{},"add":["#,
            channel
        );
        for (i, entry) in add.iter().enumerate() {
            if i > 0 {
                message.push(',');
            }
            message.push_str(entry);
        }
        message.push_str("]}");
        message
    }

    fn parse_control_message(msg_json: &str) -> Result<ControlMessage, StreamerError> {
        let message = serde_json::from_str::<Message>(msg_json)
            .map_err(|_| StreamerError::ResponseParse("connect message type".to_string()))?;

        match message.message_type.as_str() {
            "SETUP" => Ok(ControlMessage::Setup),
            "AUTH_STATE" => {
                let auth_response = serde_json::from_str::<AuthStateMessage>(msg_json)
                    .map_err(|_| StreamerError::ResponseParse("AUTH_STATE".to_string()))?;
                match auth_response.state.as_str() {
                    "AUTHORIZED" => Ok(ControlMessage::AuthAuthorized),
                    "UNAUTHORIZED" => Ok(ControlMessage::AuthUnauthorized),
                    _ => Err(StreamerError::ResponseParse("AUTH_STATE state".to_string())),
                }
            }
            _ => Ok(ControlMessage::Other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_setup_message_uses_compact_event_fields() {
        let fields = vec![
            "eventSymbol".to_string(),
            "bidPrice".to_string(),
            "askPrice".to_string(),
        ];

        let message = protocol::feed_setup_message(3, "Quote", &fields);

        assert_eq!(
            message,
            json!({
                "type": "FEED_SETUP",
                "channel": 3,
                "acceptAggregationPeriod": 0.1,
                "acceptDataFormat": "COMPACT",
                "acceptEventFields": {
                    "Quote": ["eventSymbol", "bidPrice", "askPrice"],
                },
            })
        );
    }

    #[test]
    fn test_handshake_state_tracks_documented_auth_sequence() {
        let mut handshake = protocol::HandshakeState::default();

        let action = handshake
            .observe(
                r#"{"type":"SETUP","channel":0,"keepaliveTimeout":60,"acceptKeepaliveTimeout":60,"version":"1.0-1.2.3"}"#,
            )
            .unwrap();
        assert_eq!(action, protocol::HandshakeAction::Ignore);
        assert!(!handshake.can_authorize());
        assert!(!handshake.is_ready());

        let action = handshake
            .observe(r#"{"type":"AUTH_STATE","channel":0,"state":"UNAUTHORIZED"}"#)
            .unwrap();
        assert_eq!(action, protocol::HandshakeAction::SendAuth);
        assert!(handshake.can_authorize());
        assert!(!handshake.is_ready());

        let action = handshake
            .observe(r#"{"type":"AUTH_STATE","channel":0,"state":"AUTHORIZED","userId":"abc"}"#)
            .unwrap();
        assert_eq!(action, protocol::HandshakeAction::Ready);
        assert!(handshake.is_ready());
    }

    #[test]
    fn test_feed_subscription_messages_chunk_by_serialized_size() {
        let large_symbol = "A".repeat(5000);
        let messages =
            protocol::feed_subscription_messages(3, "Quote", &[large_symbol.clone(), large_symbol]);
        assert_eq!(messages.len(), 2);
        let message_0 = serde_json::from_str::<Value>(&messages[0]).unwrap();
        let message_1 = serde_json::from_str::<Value>(&messages[1]).unwrap();

        assert_eq!(message_0["add"].as_array().unwrap().len(), 1);
        assert_eq!(message_1["add"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_poll_subscriptions_reports_disconnect() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("ws://{}", listener.local_addr().unwrap());

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let read_text_message = |socket: &mut tungstenite::WebSocket<std::net::TcpStream>| {
                socket.read_message().unwrap().into_text().unwrap()
            };
            let send_text_message = |socket: &mut tungstenite::WebSocket<std::net::TcpStream>,
                                     message: &str| {
                socket
                    .write_message(tungstenite::Message::Text(message.to_string()))
                    .unwrap();
            };

            let _ = read_text_message(&mut socket);
            send_text_message(
                &mut socket,
                r#"{"type":"SETUP","channel":0,"keepaliveTimeout":60,"acceptKeepaliveTimeout":60,"version":"1.0-test"}"#,
            );
            send_text_message(
                &mut socket,
                r#"{"type":"AUTH_STATE","channel":0,"state":"UNAUTHORIZED"}"#,
            );
            let _ = read_text_message(&mut socket);
            send_text_message(
                &mut socket,
                r#"{"type":"AUTH_STATE","channel":0,"state":"AUTHORIZED"}"#,
            );
            socket.close(None).unwrap();
        });

        let mut client = Client {
            base_url,
            token: "test-token".to_string(),
            transport: None,
            feed_channel: None,
            subscription_fields: HashMap::new(),
            last_keepalive_at: None,
        };
        client.connect().unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let err = match client.poll_subscriptions() {
            Ok(_) => panic!("expected disconnect error"),
            Err(err) => err,
        };
        assert!(err.is_disconnect());
        assert!(matches!(err, StreamerError::Disconnected));

        handle.join().unwrap();
    }

    #[test]
    fn test_accumulate_feed_data_parses_compact_rows() {
        let subscription_fields = HashMap::from([(
            "Quote".to_string(),
            vec![
                "eventSymbol".to_string(),
                "bidPrice".to_string(),
                "askPrice".to_string(),
            ],
        )]);
        let mut new_subscription_data = HashMap::new();

        let consumed = accumulate_feed_data(
            &subscription_fields,
            &mut new_subscription_data,
            r#"{"type":"FEED_DATA","channel":1,"data":["Quote",["SPY",600.1,600.2,"QQQ",499.1,499.2]]}"#,
        )
        .unwrap();

        assert!(consumed);

        let quote_data = new_subscription_data.get("Quote").unwrap();
        let event_symbols = quote_data
            .iter_field("eventSymbol")
            .cloned()
            .collect::<Vec<_>>();
        let bid_prices = quote_data
            .iter_field("bidPrice")
            .cloned()
            .collect::<Vec<_>>();
        let ask_prices = quote_data
            .iter_field("askPrice")
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(event_symbols, vec![json!("SPY"), json!("QQQ")]);
        assert_eq!(bid_prices, vec![json!(600.1), json!(499.1)]);
        assert_eq!(ask_prices, vec![json!(600.2), json!(499.2)]);
    }
}
