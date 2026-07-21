// see https://developer.tastytrade.com/streaming-market-data/

use crate::{api, request::request, session::Session};

use num_rational::Rational64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use url::Url;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_POLL_DURATION: Duration = Duration::from_millis(100);
const QUOTE_TOKENS_ENDPOINT: &str = "api-quote-tokens";

type Socket = tungstenite::protocol::WebSocket<MaybeTlsStream<TcpStream>>;

pub struct Client {
    base_url: String,
    token: String,
    transport: Option<Transport>,
    ticker_channel: Option<i32>,
    history_channel: Option<i32>,
    subscription_fields: HashMap<String, Vec<String>>,
    // Opening a channel requires a blocking read. Preserve frames for existing channels that
    // arrive before the CHANNEL_OPENED response so poll_subscriptions can process them later.
    pending_messages: VecDeque<String>,
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
            ticker_channel: None,
            history_channel: None,
            subscription_fields: HashMap::new(),
            pending_messages: VecDeque::new(),
            last_keepalive_at: None,
        })
    }

    pub fn connect(&mut self) -> Result<(), StreamerError> {
        self.reset_connection_state();

        let mut transport = Transport::connect(&self.base_url)?;
        complete_handshake(&mut transport, &self.token)?;

        // Feed channels open on their first subscription. This keeps ticker-only connection setup
        // independent of the separate history service.
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

    pub fn add_symbol_subscriptions(
        &mut self,
        kind: SymbolSubscriptionKind,
        fields: &[String],
        symbols: &[String],
    ) -> Result<(), StreamerError> {
        if symbols.is_empty() {
            return Ok(());
        }

        let feed_channel = self.ensure_ticker_channel()?;
        self.ensure_feed_setup(feed_channel, kind.name(), fields)?;

        for message in protocol::feed_symbol_subscription_messages(
            feed_channel,
            kind.name(),
            symbols,
            protocol::SubscriptionAction::Add,
        ) {
            self.transport_mut()?.send_text(&message)?;
        }

        Ok(())
    }

    pub fn remove_symbol_subscriptions(
        &mut self,
        kind: SymbolSubscriptionKind,
        symbols: &[String],
    ) -> Result<(), StreamerError> {
        if symbols.is_empty() {
            return Ok(());
        }

        self.transport_mut()?;
        let Some(feed_channel) = self.ticker_channel else {
            // Without a ticker channel, this client cannot have symbol subscriptions to remove.
            return Ok(());
        };
        for message in protocol::feed_symbol_subscription_messages(
            feed_channel,
            kind.name(),
            symbols,
            protocol::SubscriptionAction::Remove,
        ) {
            self.transport_mut()?.send_text(&message)?;
        }

        Ok(())
    }

    pub fn add_candle_subscriptions(
        &mut self,
        fields: &[String],
        subscriptions: &[CandleSubscription],
    ) -> Result<(), StreamerError> {
        if subscriptions.is_empty() {
            return Ok(());
        }

        let feed_channel = self.ensure_history_channel()?;
        self.ensure_feed_setup(feed_channel, "Candle", fields)?;

        for message in protocol::feed_candle_subscription_messages(
            feed_channel,
            subscriptions,
            protocol::SubscriptionAction::Add,
        ) {
            self.transport_mut()?.send_text(&message)?;
        }

        Ok(())
    }

    pub fn remove_candle_subscriptions(
        &mut self,
        subscriptions: &[CandleSubscription],
    ) -> Result<(), StreamerError> {
        if subscriptions.is_empty() {
            return Ok(());
        }

        self.transport_mut()?;
        let Some(feed_channel) = self.history_channel else {
            // Without a history channel, this client cannot have candle subscriptions to remove.
            return Ok(());
        };
        for message in protocol::feed_candle_subscription_messages(
            feed_channel,
            subscriptions,
            protocol::SubscriptionAction::Remove,
        ) {
            self.transport_mut()?.send_text(&message)?;
        }

        Ok(())
    }

    pub fn poll_subscriptions(
        &mut self,
    ) -> Result<HashMap<String, SubscriptionData>, StreamerError> {
        let mut new_subscription_data = HashMap::new();
        let started_at = Instant::now();
        loop {
            let msg_json = match self.pending_messages.pop_front() {
                Some(msg_json) => msg_json,
                None => match self.transport_mut()?.read_text_message(false)? {
                    Some(msg_json) => msg_json,
                    None => break,
                },
            };
            accumulate_feed_data(
                &self.subscription_fields,
                &mut new_subscription_data,
                &msg_json,
            )?;

            // During active trading, new frames can arrive faster than this nonblocking loop can
            // drain them. Limit each poll so the caller can apply updates, change subscriptions,
            // and call this client often enough to send keepalives.
            if started_at.elapsed() >= MAX_POLL_DURATION {
                break;
            }
        }

        self.keep_alive()?;

        Ok(new_subscription_data)
    }

    fn ensure_ticker_channel(&mut self) -> Result<i32, StreamerError> {
        if let Some(channel) = self.ticker_channel {
            return Ok(channel);
        }

        let channel = self.open_feed_channel(1, "TICKER")?;
        self.ticker_channel = Some(channel);
        Ok(channel)
    }

    fn ensure_history_channel(&mut self) -> Result<i32, StreamerError> {
        if let Some(channel) = self.history_channel {
            return Ok(channel);
        }

        let channel = self.open_feed_channel(3, "HISTORY")?;
        self.history_channel = Some(channel);
        Ok(channel)
    }

    fn ensure_feed_setup(
        &mut self,
        feed_channel: i32,
        name: &str,
        fields: &[String],
    ) -> Result<(), StreamerError> {
        if self.subscription_fields.contains_key(name) {
            return Ok(());
        }

        self.transport_mut()?
            .send_json(&protocol::feed_setup_message(feed_channel, name, fields))?;
        self.subscription_fields
            .insert(name.to_string(), fields.to_vec());
        Ok(())
    }

    fn reset_connection_state(&mut self) {
        self.transport = None;
        self.ticker_channel = None;
        self.history_channel = None;
        self.subscription_fields.clear();
        self.pending_messages.clear();
        self.last_keepalive_at = None;
    }

    fn open_feed_channel(&mut self, channel: i32, contract: &str) -> Result<i32, StreamerError> {
        self.transport_mut()?
            .send_json(&protocol::feed_channel_request_message(channel, contract))?;
        loop {
            let msg_json = self.transport_mut()?.read_required_text_message()?;
            if let Some(opened_channel) =
                protocol::parse_channel_opened_message(&msg_json, channel)?
            {
                return Ok(opened_channel);
            }
            self.pending_messages.push_back(msg_json);
        }
    }

    fn keep_alive(&mut self) -> Result<(), StreamerError> {
        if self
            .last_keepalive_at
            .is_some_and(|last_keepalive_at| last_keepalive_at.elapsed() < KEEPALIVE_INTERVAL)
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
        self.socket.send(tungstenite::Message::text(msg))?;
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
        // tungstenite does not provide a connection-level switch for nonblocking reads. Set the
        // mode on the underlying TCP stream before each read instead.
        // https://github.com/snapview/tungstenite-rs/issues/103
        socket_tcp_stream_mut(self.socket.get_mut()).set_nonblocking(!blocking)?;

        match self.socket.read() {
            Ok(tungstenite::Message::Close(_)) => Err(StreamerError::Disconnected),
            Ok(msg) => {
                log::debug!("Received message: {}", msg);
                Ok(Some(msg))
            }
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(None)
            }
            Err(e) => Err(StreamerError::from(e)),
        }
    }
}

fn socket_tcp_stream_mut(stream: &mut MaybeTlsStream<TcpStream>) -> &mut TcpStream {
    match stream {
        MaybeTlsStream::Plain(stream) => stream,
        #[cfg(feature = "native-tls")]
        MaybeTlsStream::NativeTls(stream) => stream.get_mut(),
        #[cfg(feature = "rustls-tls")]
        MaybeTlsStream::Rustls(stream) => &mut stream.sock,
        _ => unreachable!("unsupported tungstenite stream variant"),
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
) -> Result<(), StreamerError> {
    // COMPACT frames omit field names and contain `[event_type, flat_field_values...]`. Keep the
    // requested field order with the values so SubscriptionData can reconstruct each event row.
    let mut feed_data = match protocol::parse_feed_message(msg_json)? {
        protocol::FeedMessage::Data(data) => data,
        protocol::FeedMessage::Error { error, message } => {
            return Err(StreamerError::Server { error, message });
        }
        protocol::FeedMessage::Other => return Ok(()),
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

    Ok(())
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
    Server { error: String, message: String },
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
            Self::Server { error, message } => write!(f, "Streamer {error}: {message}"),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CandlePeriod {
    value: u32,
    unit: CandleUnit,
}

impl CandlePeriod {
    pub const fn minutes(value: u32) -> Self {
        Self::new(value, CandleUnit::Minute)
    }

    pub const fn hours(value: u32) -> Self {
        Self::new(value, CandleUnit::Hour)
    }

    pub const fn days(value: u32) -> Self {
        Self::new(value, CandleUnit::Day)
    }

    pub const fn weeks(value: u32) -> Self {
        Self::new(value, CandleUnit::Week)
    }

    pub const fn months(value: u32) -> Self {
        Self::new(value, CandleUnit::Month)
    }

    pub fn parse_dxlink(code: &str) -> Option<Self> {
        let unit_code = code.chars().last()?;
        let value_len = code.len().checked_sub(unit_code.len_utf8())?;
        let value = if value_len == 0 {
            1
        } else {
            code[..value_len].parse::<u32>().ok()?
        };
        if value == 0 {
            return None;
        }

        let unit = match unit_code {
            'm' => CandleUnit::Minute,
            'h' => CandleUnit::Hour,
            'd' => CandleUnit::Day,
            'w' => CandleUnit::Week,
            'M' => CandleUnit::Month,
            _ => return None,
        };
        Some(Self::new(value, unit))
    }

    const fn new(value: u32, unit: CandleUnit) -> Self {
        assert!(value > 0, "candle period must be greater than zero");
        Self { value, unit }
    }
}

impl fmt::Display for CandlePeriod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.dxlink_code())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CandleUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

impl CandleUnit {
    const fn dxlink_code(self) -> char {
        match self {
            Self::Minute => 'm',
            Self::Hour => 'h',
            Self::Day => 'd',
            Self::Week => 'w',
            Self::Month => 'M',
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CandlePrice {
    #[default]
    Last,
    Mark,
}

impl CandlePrice {
    fn dxlink_code(self) -> Option<char> {
        match self {
            Self::Last => None,
            Self::Mark => Some('m'),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleSubscription {
    pub symbol: String,
    pub period: CandlePeriod,
    pub from_time: i64,
    pub extended_trading_hours: bool,
    pub price: CandlePrice,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CandleStreamKey {
    pub symbol: String,
    pub period: CandlePeriod,
    pub extended_trading_hours: bool,
    pub price: CandlePrice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandleStreamKeyParseError;

impl fmt::Display for CandleStreamKeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid candle event symbol")
    }
}

impl Error for CandleStreamKeyParseError {}

impl CandleSubscription {
    pub fn new(symbol: impl Into<String>, period: CandlePeriod, from_time: i64) -> Self {
        Self {
            symbol: symbol.into(),
            period,
            from_time,
            extended_trading_hours: false,
            price: CandlePrice::Last,
        }
    }

    pub fn with_extended_trading_hours(mut self, extended_trading_hours: bool) -> Self {
        self.extended_trading_hours = extended_trading_hours;
        self
    }

    pub fn with_price(mut self, price: CandlePrice) -> Self {
        self.price = price;
        self
    }

    fn feed_symbol(&self) -> String {
        format_candle_symbol(
            &self.symbol,
            self.period,
            self.extended_trading_hours,
            self.price,
        )
    }

    pub fn stream_key(&self) -> CandleStreamKey {
        CandleStreamKey {
            symbol: self.symbol.clone(),
            period: self.period,
            extended_trading_hours: self.extended_trading_hours,
            price: self.price,
        }
    }
}

impl TryFrom<&str> for CandleStreamKey {
    type Error = CandleStreamKeyParseError;

    fn try_from(event_symbol: &str) -> Result<Self, Self::Error> {
        // A candle eventSymbol includes its period and optional attributes, for example
        // `SPY{=5m,tho=true,a=m}`. fromTime is not part of the stream identity.
        let Some((symbol, candle_spec)) = event_symbol.split_once("{=") else {
            return Err(CandleStreamKeyParseError);
        };
        let period_end = candle_spec.find([',', '}']).unwrap_or(candle_spec.len());
        let period_code = &candle_spec[..period_end];
        let Some(period) = CandlePeriod::parse_dxlink(period_code) else {
            return Err(CandleStreamKeyParseError);
        };

        let mut extended_trading_hours = false;
        let mut price = CandlePrice::Last;
        let attributes = candle_spec[period_end..].trim_end_matches('}');
        for attribute in attributes
            .split(',')
            .filter(|attribute| !attribute.is_empty())
        {
            match attribute {
                "tho=true" => extended_trading_hours = true,
                "a=m" => price = CandlePrice::Mark,
                _ => return Err(CandleStreamKeyParseError),
            }
        }

        Ok(CandleStreamKey {
            symbol: symbol.to_string(),
            period,
            extended_trading_hours,
            price,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolSubscriptionKind {
    Quote,
    Trade,
    TradeEth,
    Greeks,
    Profile,
    Summary,
    TimeAndSale,
    TheoPrice,
}

impl SymbolSubscriptionKind {
    fn name(self) -> &'static str {
        match self {
            Self::Quote => "Quote",
            Self::Trade => "Trade",
            Self::TradeEth => "TradeETH",
            Self::Greeks => "Greeks",
            Self::Profile => "Profile",
            Self::Summary => "Summary",
            Self::TimeAndSale => "TimeAndSale",
            Self::TheoPrice => "TheoPrice",
        }
    }
}

fn format_candle_symbol(
    symbol: &str,
    period: CandlePeriod,
    extended_trading_hours: bool,
    price: CandlePrice,
) -> String {
    let mut candle_symbol = format!("{symbol}{{={period}");

    if extended_trading_hours {
        candle_symbol.push_str(",tho=true");
    }

    if let Some(price_code) = price.dxlink_code() {
        candle_symbol.push_str(",a=");
        candle_symbol.push(price_code);
    }

    candle_symbol.push('}');
    candle_symbol
}

mod protocol {
    use super::*;

    const CONTROL_CHANNEL: i32 = 0;
    const FEED_SERVICE: &str = "FEED";
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
        channel: i32,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type")]
    enum IncomingFeedMessage {
        #[serde(rename = "FEED_DATA")]
        Data { data: Vec<Value> },
        #[serde(rename = "ERROR")]
        Error { error: String, message: String },
        #[serde(other)]
        Other,
    }

    #[derive(Debug, Serialize)]
    struct SubscriptionEntry<'a> {
        #[serde(rename = "type")]
        event_type: &'a str,
        symbol: &'a str,
        #[serde(rename = "fromTime", skip_serializing_if = "Option::is_none")]
        from_time: Option<i64>,
    }

    #[derive(Clone, Copy)]
    pub(super) enum SubscriptionAction {
        Add,
        Remove,
    }

    impl SubscriptionAction {
        fn name(self) -> &'static str {
            match self {
                Self::Add => "add",
                Self::Remove => "remove",
            }
        }
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

    pub(super) enum FeedMessage {
        Data(CompactFeedData),
        Error { error: String, message: String },
        Other,
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

    pub(super) fn feed_channel_request_message(channel: i32, contract: &str) -> Value {
        json!({
            "type": "CHANNEL_REQUEST",
            "channel": channel,
            "service": FEED_SERVICE,
            "parameters": {
                "contract": contract,
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

    pub(super) fn feed_symbol_subscription_messages(
        channel: i32,
        event_type: &str,
        symbols: &[String],
        action: SubscriptionAction,
    ) -> Vec<String> {
        feed_subscription_messages(
            channel,
            symbols
                .iter()
                .map(|symbol| serialize_subscription_entry(event_type, symbol, None)),
            action,
        )
    }

    pub(super) fn feed_candle_subscription_messages(
        channel: i32,
        subscriptions: &[CandleSubscription],
        action: SubscriptionAction,
    ) -> Vec<String> {
        // The history service permits only one candle subscription in each request. This limit is
        // independent of the general dxLink message-size limit used for ticker subscriptions.
        subscriptions
            .iter()
            .map(|subscription| {
                feed_subscription_message(
                    channel,
                    &[serialize_subscription_entry(
                        "Candle",
                        &subscription.feed_symbol(),
                        Some(subscription.from_time),
                    )],
                    action,
                )
            })
            .collect()
    }

    fn feed_subscription_messages(
        channel: i32,
        entries: impl IntoIterator<Item = String>,
        action: SubscriptionAction,
    ) -> Vec<String> {
        let mut messages = vec![];
        let mut batch = vec![];
        let mut size = feed_subscription_message(channel, &[], action).len();

        for entry in entries {
            let entry_size = entry.len() + usize::from(!batch.is_empty());

            if size + entry_size > MAX_SEND_SUBSCRIPTION_BYTE_SIZE && !batch.is_empty() {
                messages.push(feed_subscription_message(channel, &batch, action));
                batch.clear();
                size = feed_subscription_message(channel, &[], action).len();
            }

            size += entry_size;
            batch.push(entry);
        }

        if !batch.is_empty() {
            messages.push(feed_subscription_message(channel, &batch, action));
        }

        messages
    }

    fn serialize_subscription_entry(
        event_type: &str,
        symbol: &str,
        from_time: Option<i64>,
    ) -> String {
        serde_json::to_string(&SubscriptionEntry {
            event_type,
            symbol,
            from_time,
        })
        .expect("feed subscription entry should serialize")
    }

    pub(super) fn keepalive_message() -> Value {
        json!({
            "type": "KEEPALIVE",
            "channel": CONTROL_CHANNEL,
        })
    }

    pub(super) fn parse_channel_opened_message(
        msg_json: &str,
        requested_channel: i32,
    ) -> Result<Option<i32>, StreamerError> {
        let message = serde_json::from_str::<Message>(msg_json)?;
        if message.message_type == "ERROR" {
            let IncomingFeedMessage::Error { error, message } =
                serde_json::from_str::<IncomingFeedMessage>(msg_json)?
            else {
                unreachable!("message type was already parsed as ERROR");
            };
            return Err(StreamerError::Server { error, message });
        }
        if message.message_type != "CHANNEL_OPENED" {
            return Ok(None);
        }

        let response = serde_json::from_str::<ChannelOpenedMessage>(msg_json)?;
        if response.channel != requested_channel {
            return Err(StreamerError::ResponseParse(format!(
                "CHANNEL_OPENED channel {}, expected {}",
                response.channel, requested_channel
            )));
        }
        Ok(Some(response.channel))
    }

    pub(super) fn parse_feed_message(msg_json: &str) -> Result<FeedMessage, StreamerError> {
        let mut data = match serde_json::from_str::<IncomingFeedMessage>(msg_json)? {
            IncomingFeedMessage::Data { data } => data,
            IncomingFeedMessage::Error { error, message } => {
                return Ok(FeedMessage::Error { error, message });
            }
            IncomingFeedMessage::Other => return Ok(FeedMessage::Other),
        };

        let name = data
            .first()
            .and_then(|name| name.as_str())
            .map(String::from)
            .ok_or_else(|| StreamerError::ResponseParse("name".to_string()))?;
        let data_seq = data
            .get_mut(1)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| StreamerError::ResponseParse("data seq".to_string()))?;

        Ok(FeedMessage::Data(CompactFeedData {
            name,
            data_seq: std::mem::take(data_seq),
        }))
    }

    // Subscription entries are pre-serialized because batching uses their exact wire size.
    fn feed_subscription_message(
        channel: i32,
        entries: &[String],
        action: SubscriptionAction,
    ) -> String {
        let mut message = format!(
            r#"{{"type":"FEED_SUBSCRIPTION","channel":{},"{}":["#,
            channel,
            action.name(),
        );
        for (i, entry) in entries.iter().enumerate() {
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
    fn test_streamer_error_messages_are_reported() {
        let message = r#"{"type":"ERROR","channel":1,"error":"BAD_ACTION","message":"Your subscription rate is too high"}"#;
        let feed_error =
            accumulate_feed_data(&HashMap::new(), &mut HashMap::new(), message).unwrap_err();
        let channel_error = protocol::parse_channel_opened_message(message, 1).unwrap_err();

        assert_eq!(
            feed_error.to_string(),
            "Streamer BAD_ACTION: Your subscription rate is too high"
        );
        assert_eq!(channel_error.to_string(), feed_error.to_string());
    }

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
        let messages = protocol::feed_symbol_subscription_messages(
            3,
            "Quote",
            &[large_symbol.clone(), large_symbol],
            protocol::SubscriptionAction::Add,
        );

        assert_eq!(messages.len(), 2);
        let message_0 = serde_json::from_str::<Value>(&messages[0]).unwrap();
        let message_1 = serde_json::from_str::<Value>(&messages[1]).unwrap();

        assert_eq!(message_0["add"].as_array().unwrap().len(), 1);
        assert_eq!(message_1["add"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_feed_subscription_messages_support_remove() {
        let messages = protocol::feed_symbol_subscription_messages(
            1,
            "Greeks",
            &[".SPY260821C600".to_string()],
            protocol::SubscriptionAction::Remove,
        );

        assert_eq!(
            serde_json::from_str::<Value>(&messages[0]).unwrap(),
            json!({
                "type": "FEED_SUBSCRIPTION",
                "channel": 1,
                "remove": [{
                    "type": "Greeks",
                    "symbol": ".SPY260821C600",
                }],
            })
        );
    }

    #[test]
    fn test_feed_subscription_messages_serialize_candle_from_time_and_symbol() {
        let subscriptions =
            [
                CandleSubscription::new("SPY", CandlePeriod::minutes(5), 1_775_192_400_000)
                    .with_extended_trading_hours(true)
                    .with_price(CandlePrice::Mark),
            ];
        let messages = protocol::feed_candle_subscription_messages(
            1,
            &subscriptions,
            protocol::SubscriptionAction::Add,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&messages[0]).unwrap(),
            json!({
                "type": "FEED_SUBSCRIPTION",
                "channel": 1,
                "add": [{
                    "type": "Candle",
                    "symbol": "SPY{=5m,tho=true,a=m}",
                    "fromTime": 1_775_192_400_000_i64,
                }],
            })
        );
    }

    #[test]
    fn test_candle_subscriptions_are_sent_individually() {
        let subscriptions = [
            CandleSubscription::new("SPY", CandlePeriod::minutes(5), 1),
            CandleSubscription::new("QQQ", CandlePeriod::minutes(5), 1),
        ];

        let messages = protocol::feed_candle_subscription_messages(
            1,
            &subscriptions,
            protocol::SubscriptionAction::Add,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&messages[0]).unwrap()["add"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            serde_json::from_str::<Value>(&messages[1]).unwrap()["add"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_candle_period_parse_dxlink_codes() {
        assert_eq!(CandlePeriod::parse_dxlink("d"), Some(CandlePeriod::days(1)));
        assert_eq!(
            CandlePeriod::parse_dxlink("1d"),
            Some(CandlePeriod::days(1))
        );
        assert_eq!(
            CandlePeriod::parse_dxlink("5m"),
            Some(CandlePeriod::minutes(5))
        );
        assert_eq!(CandlePeriod::parse_dxlink("0d"), None);
        assert_eq!(CandlePeriod::parse_dxlink("foo"), None);
    }

    #[test]
    fn test_candle_subscription_stream_key_matches_decoded_event_symbol() {
        let subscription = CandleSubscription::new("SPY", CandlePeriod::days(1), 123)
            .with_extended_trading_hours(true)
            .with_price(CandlePrice::Mark);

        assert_eq!(
            subscription.stream_key(),
            CandleStreamKey::try_from("SPY{=d,tho=true,a=m}").unwrap()
        );
    }

    #[test]
    fn test_subscription_changes_report_not_connected() {
        let mut client = Client {
            base_url: "ws://unused".to_string(),
            token: "test-token".to_string(),
            transport: None,
            ticker_channel: None,
            history_channel: None,
            subscription_fields: HashMap::new(),
            pending_messages: VecDeque::new(),
            last_keepalive_at: None,
        };
        let fields = ["eventSymbol".to_string()];
        let symbols = ["SPY".to_string()];
        let candles = [CandleSubscription::new("SPY", CandlePeriod::minutes(5), 1)];

        assert!(matches!(
            client.add_symbol_subscriptions(SymbolSubscriptionKind::Quote, &fields, &symbols),
            Err(StreamerError::NotConnected)
        ));
        assert!(matches!(
            client.remove_symbol_subscriptions(SymbolSubscriptionKind::Quote, &symbols),
            Err(StreamerError::NotConnected)
        ));
        assert!(matches!(
            client.add_candle_subscriptions(&fields, &candles),
            Err(StreamerError::NotConnected)
        ));
        assert!(matches!(
            client.remove_candle_subscriptions(&candles),
            Err(StreamerError::NotConnected)
        ));
    }

    #[test]
    fn test_channels_open_lazily_and_poll_reports_disconnect() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("ws://{}", listener.local_addr().unwrap());

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let read_text_message = |socket: &mut tungstenite::WebSocket<std::net::TcpStream>| {
                socket.read().unwrap().into_text().unwrap()
            };
            let send_text_message = |socket: &mut tungstenite::WebSocket<std::net::TcpStream>,
                                     message: &str| {
                socket.send(tungstenite::Message::text(message)).unwrap();
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
            let ticker_request = read_text_message(&mut socket);
            assert!(ticker_request.contains("\"contract\":\"TICKER\""));
            send_text_message(
                &mut socket,
                r#"{"type":"CHANNEL_OPENED","channel":1,"service":"FEED","version":1}"#,
            );
            let feed_setup = read_text_message(&mut socket);
            assert!(feed_setup.contains("\"type\":\"FEED_SETUP\""));
            let subscription = read_text_message(&mut socket);
            assert!(subscription.contains("\"type\":\"FEED_SUBSCRIPTION\""));

            let history_request = read_text_message(&mut socket);
            assert!(history_request.contains("\"contract\":\"HISTORY\""));
            send_text_message(
                &mut socket,
                r#"{"type":"FEED_DATA","channel":1,"data":["Quote",["SPY"]]}"#,
            );
            send_text_message(
                &mut socket,
                r#"{"type":"CHANNEL_OPENED","channel":3,"service":"FEED","version":1}"#,
            );
            let feed_setup = read_text_message(&mut socket);
            assert!(feed_setup.contains("\"type\":\"FEED_SETUP\""));
            let subscription = read_text_message(&mut socket);
            assert!(subscription.contains("\"type\":\"FEED_SUBSCRIPTION\""));

            std::thread::sleep(Duration::from_millis(50));
            socket.close(None).unwrap();
        });

        let mut client = Client {
            base_url,
            token: "test-token".to_string(),
            transport: None,
            ticker_channel: None,
            history_channel: None,
            subscription_fields: HashMap::new(),
            pending_messages: VecDeque::new(),
            last_keepalive_at: None,
        };
        client.connect().unwrap();
        assert!(client.ticker_channel.is_none());
        assert!(client.history_channel.is_none());

        client
            .add_symbol_subscriptions(
                SymbolSubscriptionKind::Quote,
                &["eventSymbol".to_string()],
                &["SPY".to_string()],
            )
            .unwrap();
        assert_eq!(client.ticker_channel, Some(1));
        assert!(client.history_channel.is_none());

        client
            .add_candle_subscriptions(
                &["eventSymbol".to_string()],
                &[CandleSubscription::new("SPY", CandlePeriod::minutes(5), 1)],
            )
            .unwrap();
        assert_eq!(client.history_channel, Some(3));

        let data = client.poll_subscriptions().unwrap();
        assert_eq!(
            data["Quote"]
                .iter_field("eventSymbol")
                .cloned()
                .collect::<Vec<_>>(),
            vec![json!("SPY")]
        );

        std::thread::sleep(Duration::from_millis(70));
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

        accumulate_feed_data(
            &subscription_fields,
            &mut new_subscription_data,
            r#"{"type":"FEED_DATA","channel":1,"data":["Quote",["SPY",600.1,600.2,"QQQ",499.1,499.2]]}"#,
        )
        .unwrap();

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
