// SPDX-License-Identifier: GPL-2.0-or-later
//! Blocking MQTT 3.1.1 client with a bounded in-flight window.
//!
//! The client is used from a single thread. Every socket operation has a
//! timeout so a stalled broker is detected and the daemon reconnects instead
//! of hanging. Published QoS 1 messages are not queued across reconnects: a
//! new session republishes discovery and the full state anyway.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use super::codec::{self, CodecError, Connect, Packet, Will};
use super::transport::Transport;

/// Largest incoming packet accepted (only the HA birth message is expected).
const MAX_INCOMING: usize = 64 * 1024;
const MAX_INFLIGHT: usize = 32;
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum MqttError {
    Io(io::Error),
    Resolve(String),
    Refused(u8),
    Protocol(String),
    Timeout(&'static str),
}

impl fmt::Display for MqttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MqttError::Io(e) => write!(f, "network error: {e}"),
            MqttError::Resolve(h) => write!(f, "cannot resolve {h}"),
            MqttError::Refused(code) => write!(
                f,
                "broker refused the connection: {}",
                codec::connack_meaning(*code)
            ),
            MqttError::Protocol(what) => write!(f, "protocol error: {what}"),
            MqttError::Timeout(what) => write!(f, "timeout waiting for {what}"),
        }
    }
}

impl From<io::Error> for MqttError {
    fn from(e: io::Error) -> MqttError {
        MqttError::Io(e)
    }
}

impl From<CodecError> for MqttError {
    fn from(e: CodecError) -> MqttError {
        MqttError::Protocol(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct WillMessage {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

#[derive(Clone)]
pub struct Options {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub keepalive: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub will: Option<WillMessage>,
    pub connect_timeout: Duration,
    pub tls: Option<super::transport::TlsSettings>,
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the password.
        f.debug_struct("Options")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<set>"))
            .finish_non_exhaustive()
    }
}

/// An application message received from the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

pub struct Client {
    stream: Transport,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    next_id: u16,
    inflight: BTreeMap<u16, Instant>,
    keepalive: Duration,
    last_tx: Instant,
    ping_sent: Option<Instant>,
    pub sent: u64,
    /// The broker refused a subscription (for example an ACL that does not
    /// allow the Home Assistant birth topic). Publishing keeps working, so the
    /// connection is kept and the caller decides what to report.
    pub subscription_denied: bool,
}

impl Client {
    pub fn connect(opts: &Options) -> Result<Client, MqttError> {
        let addrs: Vec<_> = (opts.host.as_str(), opts.port)
            .to_socket_addrs()
            .map_err(|_| MqttError::Resolve(opts.host.clone()))?
            .collect();
        let mut last_err = None;
        let mut tcp = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, opts.connect_timeout) {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        let tcp = match (tcp, last_err) {
            (Some(s), _) => s,
            (None, Some(e)) => return Err(MqttError::Io(e)),
            (None, None) => return Err(MqttError::Resolve(opts.host.clone())),
        };
        tcp.set_nodelay(true)?;
        tcp.set_write_timeout(Some(WRITE_TIMEOUT))?;
        tcp.set_read_timeout(Some(opts.connect_timeout))?;
        let stream = Transport::establish(tcp, &opts.host, opts.tls.as_ref())?;

        let mut client = Client {
            stream,
            rbuf: Vec::with_capacity(4096),
            wbuf: Vec::with_capacity(4096),
            next_id: 1,
            inflight: BTreeMap::new(),
            keepalive: Duration::from_secs(u64::from(opts.keepalive)),
            last_tx: Instant::now(),
            ping_sent: None,
            sent: 0,
            subscription_denied: false,
        };
        let will = opts.will.as_ref().map(|w| Will {
            topic: &w.topic,
            payload: w.payload.as_bytes(),
            qos: 1,
            retain: w.retain,
        });
        codec::encode_connect(
            &Connect {
                client_id: &opts.client_id,
                keepalive: opts.keepalive,
                clean_session: true,
                username: opts.username.as_deref(),
                password: opts.password.as_deref().map(str::as_bytes),
                will,
            },
            &mut client.wbuf,
        )?;
        client.flush()?;

        let deadline = Instant::now() + opts.connect_timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(MqttError::Timeout("CONNACK"));
            }
            match client.read_packet(deadline - Instant::now())? {
                Some(Packet::ConnAck { code: 0, .. }) => break,
                Some(Packet::ConnAck { code, .. }) => return Err(MqttError::Refused(code)),
                Some(_) => return Err(MqttError::Protocol("expected CONNACK".to_string())),
                None => {}
            }
        }
        Ok(client)
    }

    fn flush(&mut self) -> Result<(), MqttError> {
        self.stream.write_all(&self.wbuf)?;
        self.stream.flush()?;
        self.wbuf.clear();
        self.last_tx = Instant::now();
        Ok(())
    }

    fn alloc_id(&mut self) -> u16 {
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).unwrap_or(1);
            if !self.inflight.contains_key(&id) {
                return id;
            }
        }
    }

    /// Read at most one packet, waiting up to `timeout` for data.
    fn read_packet(&mut self, timeout: Duration) -> Result<Option<Packet>, MqttError> {
        if let Some((packet, used)) = codec::decode(&self.rbuf, MAX_INCOMING)? {
            self.rbuf.drain(..used);
            return Ok(Some(packet));
        }
        self.stream
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err(MqttError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "broker closed the connection",
            ))),
            Ok(n) => {
                self.rbuf.extend_from_slice(&chunk[..n]);
                match codec::decode(&self.rbuf, MAX_INCOMING)? {
                    Some((packet, used)) => {
                        self.rbuf.drain(..used);
                        Ok(Some(packet))
                    }
                    None => Ok(None),
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(None),
            Err(e) => Err(MqttError::Io(e)),
        }
    }

    /// Process one received packet; application messages are returned.
    fn handle(&mut self, packet: Packet) -> Result<Option<Incoming>, MqttError> {
        match packet {
            Packet::PubAck { packet_id } => {
                self.inflight.remove(&packet_id);
                Ok(None)
            }
            Packet::PingResp => {
                self.ping_sent = None;
                Ok(None)
            }
            Packet::Publish {
                topic,
                payload,
                qos,
                retain,
                packet_id,
            } => {
                if qos == 1 {
                    if let Some(id) = packet_id {
                        codec::encode_puback(id, &mut self.wbuf);
                        self.flush()?;
                    }
                } else if qos == 2 {
                    return Err(MqttError::Protocol(
                        "broker sent QoS 2 to a QoS 0 subscription".to_string(),
                    ));
                }
                Ok(Some(Incoming {
                    topic,
                    payload,
                    retain,
                }))
            }
            Packet::SubAck { codes, .. } => {
                if codes.contains(&0x80) {
                    self.subscription_denied = true;
                }
                Ok(None)
            }
            Packet::ConnAck { .. } => Err(MqttError::Protocol("unexpected CONNACK".to_string())),
            Packet::Other(_) => Ok(None),
        }
    }

    /// Wait up to `timeout` for incoming packets and return application
    /// messages. Returns early once a packet was processed and no more data is
    /// buffered.
    pub fn poll(&mut self, timeout: Duration) -> Result<Vec<Incoming>, MqttError> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.read_packet(left)? {
                Some(packet) => {
                    if let Some(msg) = self.handle(packet)? {
                        out.push(msg);
                    }
                    if self.rbuf.is_empty() {
                        return Ok(out);
                    }
                }
                None if left.is_zero() => return Ok(out),
                None => {
                    if Instant::now() >= deadline {
                        return Ok(out);
                    }
                }
            }
        }
    }

    pub fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> Result<Vec<Incoming>, MqttError> {
        let mut received = Vec::new();
        let window_deadline = Instant::now() + self.ack_timeout();
        while qos == 1 && self.inflight.len() >= MAX_INFLIGHT {
            if Instant::now() >= window_deadline {
                return Err(MqttError::Timeout("PUBACK"));
            }
            received.extend(self.poll(Duration::from_millis(200))?);
        }
        let id = if qos == 1 {
            Some(self.alloc_id())
        } else {
            None
        };
        codec::encode_publish(topic, payload, qos, retain, id, &mut self.wbuf)?;
        self.flush()?;
        if let Some(id) = id {
            self.inflight.insert(id, Instant::now());
        }
        self.sent += 1;
        Ok(received)
    }

    pub fn subscribe(&mut self, filter: &str) -> Result<(), MqttError> {
        let id = self.alloc_id();
        codec::encode_subscribe(id, filter, 0, &mut self.wbuf)?;
        self.flush()
    }

    fn ack_timeout(&self) -> Duration {
        self.keepalive.max(Duration::from_secs(10))
    }

    /// Send PINGREQ when idle and detect a dead connection: missing PINGRESP
    /// or PUBACKs older than the keepalive period.
    pub fn maintain(&mut self) -> Result<(), MqttError> {
        let now = Instant::now();
        if let Some(sent) = self.ping_sent {
            if now.duration_since(sent) > self.keepalive {
                return Err(MqttError::Timeout("PINGRESP"));
            }
        }
        if let Some(oldest) = self.inflight.values().min() {
            if now.duration_since(*oldest) > self.ack_timeout() {
                return Err(MqttError::Timeout("PUBACK"));
            }
        }
        if self.ping_sent.is_none() && now.duration_since(self.last_tx) >= self.keepalive / 2 {
            codec::encode_pingreq(&mut self.wbuf);
            self.flush()?;
            self.ping_sent = Some(now);
        }
        Ok(())
    }

    /// Wait until every QoS 1 publish is acknowledged or `timeout` expires.
    pub fn drain(&mut self, timeout: Duration) -> Result<Vec<Incoming>, MqttError> {
        let deadline = Instant::now() + timeout;
        let mut received = Vec::new();
        while !self.inflight.is_empty() {
            if Instant::now() >= deadline {
                return Err(MqttError::Timeout("PUBACK"));
            }
            received.extend(self.poll(Duration::from_millis(200))?);
        }
        Ok(received)
    }

    pub fn inflight(&self) -> usize {
        self.inflight.len()
    }

    /// Clean disconnect. The broker discards the Last Will, so callers
    /// publish their own `offline` state first.
    pub fn disconnect(mut self) {
        codec::encode_disconnect(&mut self.wbuf);
        let _ = self.flush();
        self.stream.shutdown(Shutdown::Both);
    }
}
