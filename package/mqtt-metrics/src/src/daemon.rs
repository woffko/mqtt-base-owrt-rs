// SPDX-License-Identifier: GPL-2.0-or-later
//! The long-running publisher.
//!
//! One thread runs everything: signal flags, the control socket, the MQTT
//! connection with bounded reconnect backoff, and the publish engine. Every
//! blocking call has a timeout so the loop keeps turning.

use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::collect;
use crate::config::{self, Config, RUN_DIR, SECRET_PATH, STATE_PATH};
use crate::control::{self, Command};
use crate::engine::{Engine, Publication};
use crate::log::{Level, Log};
use crate::mqtt::{Client, Incoming, MqttError, Options, WillMessage};
use crate::rpcd;
use crate::source::{Source, SystemSource};
use crate::state::{write_atomic, TopicState};
use crate::status::{self, Connection};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const LOOP_STEP: Duration = Duration::from_secs(1);
const STATUS_EVERY: f64 = 15.0;
/// How long a pass may wait for the broker to acknowledge its messages.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Least time between two passes caused by Home Assistant announcements.
const BIRTH_SPACING_S: f64 = 30.0;
const CONFIG_RETRY_MIN_S: f64 = 10.0;
const CONFIG_RETRY_MAX_S: f64 = 300.0;
const BACKOFF_MIN_S: f64 = 5.0;
const BACKOFF_MAX_S: f64 = 120.0;
/// A connection that lasted this long counts as healthy: the next failure
/// starts again from the shortest delay.
const STABLE_S: f64 = 60.0;

/// Reconnect delays: 5 s, doubling up to 120 s. Without the growth a broker
/// that accepts the connection and then drops it (a duplicate client id, a
/// proxy in front of the broker) would make the daemon reconnect, collect and
/// republish everything every few seconds.
#[derive(Debug)]
struct Backoff {
    delay: f64,
    connected_at: Option<f64>,
}

impl Backoff {
    fn new() -> Backoff {
        Backoff {
            delay: BACKOFF_MIN_S,
            connected_at: None,
        }
    }

    /// A connection was established at `now`.
    fn connected(&mut self, now: f64) {
        self.connected_at = Some(now);
    }

    /// A connection attempt failed or the connection dropped at `now`.
    /// Returns how long to wait before the next attempt.
    fn failed(&mut self, now: f64) -> f64 {
        if self
            .connected_at
            .take()
            .is_some_and(|since| now - since >= STABLE_S)
        {
            self.delay = BACKOFF_MIN_S;
        }
        let wait = self.delay;
        self.delay = (self.delay * 2.0).min(BACKOFF_MAX_S);
        wait
    }

    /// The configuration changed or the connection was closed on purpose:
    /// the next attempt happens at once and starts from the shortest delay.
    fn reset(&mut self) {
        self.delay = BACKOFF_MIN_S;
        self.connected_at = None;
    }
}

/// A configuration that could not be read at all (`uci` failed or timed out)
/// may be readable a moment later. One that is invalid stays invalid until
/// someone edits it, and the reload that follows the edit reads it again, so
/// only the first kind is retried.
#[derive(Debug)]
struct ConfigRetry {
    delay: f64,
    at: Option<f64>,
}

impl ConfigRetry {
    fn new() -> ConfigRetry {
        ConfigRetry {
            delay: CONFIG_RETRY_MIN_S,
            at: None,
        }
    }

    /// The outcome of a load at `now`.
    fn loaded(&mut self, now: f64, unreadable: bool) {
        if unreadable {
            self.at = Some(now + self.delay);
            self.delay = (self.delay * 2.0).min(CONFIG_RETRY_MAX_S);
        } else {
            self.at = None;
            self.delay = CONFIG_RETRY_MIN_S;
        }
    }

    fn due(&self, now: f64) -> bool {
        self.at.is_some_and(|at| now >= at)
    }
}

/// Home Assistant announces itself once when it starts. Every announcement
/// makes the daemon collect and republish everything, and anyone who can
/// publish to the birth topic can send as many as they like, so passes caused
/// by announcements are spaced out. An announcement that arrives too soon is
/// answered as soon as the spacing allows.
#[derive(Debug, Default)]
struct BirthLimit {
    last: Option<f64>,
    deferred: bool,
}

impl BirthLimit {
    /// An announcement arrived at `now`: `true` when it is answered now.
    fn heard(&mut self, now: f64) -> bool {
        if self.last.is_none_or(|last| now - last >= BIRTH_SPACING_S) {
            self.last = Some(now);
            self.deferred = false;
            true
        } else {
            self.deferred = true;
            false
        }
    }

    /// `true` when a deferred announcement can be answered at `now`.
    fn due(&mut self, now: f64) -> bool {
        if self.deferred && self.last.is_some_and(|last| now - last >= BIRTH_SPACING_S) {
            self.last = Some(now);
            self.deferred = false;
            true
        } else {
            false
        }
    }
}

/// Only one daemon may run. Two instances share the client id (the broker
/// drops one whenever the other connects, and they take turns forever) and
/// the control socket. The lock is released when the process ends, however it
/// ends.
fn lock_instance(path: &Path) -> Result<File, String> {
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => {
            Err("another mqtt-metricsd is already running".to_string())
        }
        Err(TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

struct Signals {
    term: Arc<AtomicBool>,
    hup: Arc<AtomicBool>,
}

impl Signals {
    fn register() -> std::io::Result<Signals> {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        let term = Arc::new(AtomicBool::new(false));
        let hup = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGTERM, Arc::clone(&term))?;
        signal_hook::flag::register(SIGINT, Arc::clone(&term))?;
        signal_hook::flag::register(SIGHUP, Arc::clone(&hup))?;
        Ok(Signals { term, hup })
    }
}

fn password_hash(password: Option<&str>) -> u64 {
    let mut h = DefaultHasher::new();
    password.hash(&mut h);
    h.finish()
}

struct Session {
    client: Client,
    password_hash: u64,
    broker: config::Broker,
    /// Last Will topic registered with the broker for this session.
    availability_topic: String,
    client_id: String,
    /// Birth topic this session is subscribed to.
    birth: Option<String>,
    /// The broker refused that subscription; already reported.
    birth_denied: bool,
}

/// What a clean disconnect leaves on the session's availability topic.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Farewell {
    /// Retained `offline` (service stopped or disabled).
    Offline,
    /// Empty retained message: the topic is no longer used (identity change).
    Clear,
}

struct Daemon {
    log: Log,
    source: SystemSource,
    started: Instant,
    config: Result<Config, Vec<String>>,
    engine: Option<Engine>,
    session: Option<Session>,
    conn: Connection,
    need_full: bool,
    next_attempt: f64,
    backoff: Backoff,
    config_retry: ConfigRetry,
    birth_limit: BirthLimit,
    last_status: f64,
}

impl Daemon {
    fn mono(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn now(&self) -> u64 {
        self.source.now()
    }

    fn set_state(&mut self, state: &'static str) {
        if self.conn.state != state {
            self.conn.state = state;
            self.conn.since = self.now();
        }
    }

    fn load(&mut self) {
        let (cfg, errors) = rpcd::load_config(&self.source);
        let unreadable = errors
            .iter()
            .any(|e| e.starts_with(rpcd::CONFIG_UNREADABLE));
        self.config_retry.loaded(self.mono(), unreadable);
        self.config = if errors.is_empty() {
            Ok(cfg)
        } else {
            Err(errors)
        };
        match &self.config {
            Ok(cfg) => {
                self.log = Log::new(cfg.log_level);
                let hostname = collect::hostname(&self.source);
                match self.engine.as_mut() {
                    Some(engine) => engine.reconfigure(cfg.clone(), &hostname),
                    None => {
                        let topics = TopicState::load(Path::new(STATE_PATH));
                        self.engine = Some(Engine::new(cfg.clone(), &hostname, topics));
                    }
                }
                self.conn.broker = format!("{}:{}", cfg.broker.host, cfg.broker.port);
                self.need_full = true;
                self.log.info(&format!(
                    "configuration loaded: broker {}, base topic {}/{}, discovery {}",
                    if cfg.configured() {
                        self.conn.broker.as_str()
                    } else {
                        "(not set)"
                    },
                    cfg.identity.base_topic,
                    cfg.identity.node.as_deref().unwrap_or(&hostname),
                    if cfg.identity.discovery { "on" } else { "off" }
                ));
            }
            Err(errors) => {
                for e in errors {
                    self.log.error(&format!("configuration: {e}"));
                }
            }
        }
    }

    /// Reload configuration; reconnect when broker settings or the password
    /// changed.
    fn reload(&mut self) {
        self.load();
        let secret = config::read_secret(SECRET_PATH).ok().flatten();
        let identity = self.engine.as_ref().map(|e| e.identity().clone());
        let (reconnect, farewell) = match (&self.config, &self.session, &identity) {
            (Ok(cfg), Some(s), Some(id)) => {
                let moved = id.availability_topic != s.availability_topic;
                let changed = cfg.broker != s.broker
                    || password_hash(secret.as_deref()) != s.password_hash
                    || id.client_id != s.client_id
                    || moved;
                (
                    changed,
                    if moved {
                        Farewell::Clear
                    } else {
                        Farewell::Offline
                    },
                )
            }
            (_, Some(_), _) => (true, Farewell::Offline),
            _ => (false, Farewell::Offline),
        };
        if !reconnect {
            self.update_birth_subscription();
        }
        if reconnect {
            self.log
                .info("broker settings or device identity changed; reconnecting");
            self.disconnect(farewell);
        }
        if reconnect || self.session.is_none() {
            // A changed configuration deserves an immediate attempt, not the
            // rest of a delay that grew while the old one failed.
            self.next_attempt = self.mono();
            self.backoff.reset();
        }
    }

    fn connect(&mut self, cfg: &Config) {
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        let id = engine.identity().clone();
        let password = match config::read_secret(SECRET_PATH) {
            Ok(p) => p,
            Err(e) => {
                self.log.warn(&format!("{SECRET_PATH}: {e}"));
                None
            }
        };
        let opts = Options {
            host: cfg.broker.host.clone(),
            port: cfg.broker.port,
            client_id: id.client_id.clone(),
            keepalive: cfg.broker.keepalive,
            username: cfg.broker.username.clone(),
            password: password.clone(),
            will: Some(WillMessage {
                topic: id.availability_topic.clone(),
                payload: "offline".to_string(),
                retain: true,
            }),
            connect_timeout: CONNECT_TIMEOUT,
            tls: cfg.broker.tls_settings(),
        };
        self.set_state("connecting");
        match Client::connect(&opts) {
            Ok(mut client) => {
                let birth = wanted_birth(cfg);
                let mut setup = client
                    .publish(&id.availability_topic, b"online", 1, true)
                    .map(|_| ());
                if setup.is_ok() {
                    if let Some(topic) = &birth {
                        setup = client.subscribe(topic);
                    }
                }
                match setup {
                    Ok(()) => {
                        self.log.info(&format!(
                            "connected to {} as {}",
                            self.conn.broker, id.client_id
                        ));
                        self.session = Some(Session {
                            client,
                            password_hash: password_hash(password.as_deref()),
                            broker: cfg.broker.clone(),
                            availability_topic: id.availability_topic.clone(),
                            client_id: id.client_id.clone(),
                            birth,
                            birth_denied: false,
                        });
                        self.conn.last_connect = Some(self.now());
                        self.conn.last_error = None;
                        self.conn.next_attempt = None;
                        self.conn.birth_denied = false;
                        self.backoff.connected(self.mono());
                        self.need_full = true;
                        self.set_state("connected");
                    }
                    Err(e) => self.connect_failed(&e),
                }
            }
            Err(e) => self.connect_failed(&e),
        }
    }

    /// Subscribe to a new birth topic after a reload that kept the session.
    /// A subscription that is no longer wanted is left in place: `is_birth`
    /// only accepts the configured topic, and the next reconnect drops it.
    fn update_birth_subscription(&mut self) {
        let Ok(cfg) = &self.config else { return };
        let wanted = wanted_birth(cfg);
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if wanted == session.birth {
            return;
        }
        if let Some(topic) = &wanted {
            if let Err(e) = session.client.subscribe(topic) {
                self.lost(&e);
                return;
            }
        }
        session.birth = wanted;
    }

    fn connect_failed(&mut self, e: &MqttError) {
        let msg = e.to_string();
        if self.conn.last_error.as_deref() != Some(msg.as_str()) {
            self.log
                .warn(&format!("cannot connect to {}: {msg}", self.conn.broker));
        }
        self.conn.last_error = Some(msg);
        self.conn.reconnects += 1;
        self.schedule_retry();
        self.set_state("disconnected");
    }

    fn lost(&mut self, e: &MqttError) {
        self.log.warn(&format!("connection lost: {e}"));
        self.session = None;
        self.conn.last_error = Some(e.to_string());
        self.schedule_retry();
        self.set_state("disconnected");
    }

    fn schedule_retry(&mut self) {
        let mono = self.mono();
        let wait = self.backoff.failed(mono);
        self.next_attempt = mono + wait;
        self.conn.next_attempt = Some(self.now() + wait as u64);
    }

    /// Report a refused birth subscription once per session. The connection
    /// stays up: metrics are still published, only the resend after a Home
    /// Assistant restart does not happen.
    fn check_subscription(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.client.subscription_denied && !session.birth_denied {
            session.birth_denied = true;
            self.conn.birth_denied = true;
            let topic = session.birth.clone().unwrap_or_default();
            self.log.warn(&format!(
                "the broker denied the subscription to {topic}; discovery is not resent when Home Assistant restarts (allow the topic in the broker ACL or turn the birth subscription off)"
            ));
        }
    }

    /// Leave `offline` (or clear the topic) and disconnect cleanly. A clean
    /// DISCONNECT discards the Last Will, so the state is published here.
    fn disconnect(&mut self, farewell: Farewell) {
        if let Some(mut session) = self.session.take() {
            let payload: &[u8] = match farewell {
                Farewell::Offline => b"offline",
                Farewell::Clear => b"",
            };
            let _ = session
                .client
                .publish(&session.availability_topic, payload, 1, true);
            let _ = session.client.drain(Duration::from_secs(2));
            session.client.disconnect();
        }
    }

    fn is_birth(&self, msg: &Incoming) -> bool {
        match &self.config {
            Ok(cfg) => is_birth_message(cfg, msg),
            Err(_) => false,
        }
    }

    fn handle_incoming(&mut self, messages: Vec<Incoming>) {
        for msg in messages {
            if self.is_birth(&msg) {
                if self.birth_limit.heard(self.mono()) {
                    self.log
                        .info("Home Assistant came online; resending discovery");
                    self.need_full = true;
                } else {
                    self.log
                        .debug("Home Assistant announced itself again; the resend is deferred");
                }
            } else {
                // Read-only design: no other incoming message is acted on.
                self.log.debug(&format!("ignored message on {}", msg.topic));
            }
        }
    }

    fn send(&mut self, publications: Vec<Publication>) {
        if publications.is_empty() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let mut received = Vec::new();
        let mut failure = None;
        for p in &publications {
            match session
                .client
                .publish(&p.topic, p.payload.as_bytes(), p.qos, p.retain)
            {
                Ok(msgs) => {
                    received.extend(msgs);
                    self.conn.messages += 1;
                }
                Err(e) => {
                    self.conn.publish_errors += 1;
                    failure = Some(e);
                    break;
                }
            }
        }
        if failure.is_none() {
            self.conn.last_publish = Some(self.now());
        }
        self.handle_incoming(received);
        if let Some(e) = failure {
            self.lost(&e);
            // Everything is republished after reconnecting.
            self.need_full = true;
        }
    }

    fn persist_topics(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            if let Some(state) = engine.take_dirty_topics() {
                if let Err(e) = state.save(Path::new(STATE_PATH)) {
                    self.log.warn(&format!("{STATE_PATH}: {e}"));
                }
            }
        }
    }

    fn write_status(&mut self) {
        let errors = match &self.config {
            Ok(_) => Vec::new(),
            Err(e) => e.clone(),
        };
        let text = status::render(self.engine.as_ref(), &self.conn, &errors, self.now());
        let path = rpcd::status_path();
        if let Err(e) = write_atomic(&path, text.as_bytes(), 0o600) {
            self.log.debug(&format!("{}: {e}", path.display()));
        }
        self.last_status = self.mono();
    }

    fn handle_control(&mut self, server: Option<&control::Server>) {
        let Some(server) = server else { return };
        for req in server.poll() {
            match req.command {
                Command::Reload => {
                    self.reload();
                    if self.config.is_err() {
                        self.set_state("config_error");
                    }
                    match &self.config {
                        Ok(_) => req.reply(true, "configuration reloaded"),
                        Err(e) => req.reply(false, &e.join("; ")),
                    }
                }
                Command::Republish => {
                    if self.session.is_some() {
                        self.need_full = true;
                        self.publish_round();
                        req.reply(self.session.is_some(), "discovery and states republished");
                    } else {
                        req.reply(false, "not connected to the broker");
                    }
                }
                Command::Purge => {
                    if self.session.is_some() {
                        let pubs = self.engine.as_mut().map(Engine::purge).unwrap_or_default();
                        let count = pubs.len();
                        self.send(pubs);
                        if self.confirm_delivery(Duration::from_secs(5)) {
                            if let Some(engine) = self.engine.as_mut() {
                                engine.settle();
                            }
                        }
                        self.persist_topics();
                        self.need_full = true;
                        self.publish_round();
                        req.reply(
                            self.session.is_some(),
                            &format!("cleared {count} retained topics and republished"),
                        );
                    } else {
                        req.reply(false, "not connected to the broker");
                    }
                }
            }
            self.write_status();
        }
    }

    fn publish_round(&mut self) {
        if self.session.is_none() {
            return;
        }
        let now = self.now();
        let mono = self.mono();
        if self.birth_limit.due(mono) {
            self.need_full = true;
        }
        let full = self.need_full;
        let pubs = match self.engine.as_mut() {
            Some(engine) if full => {
                self.need_full = false;
                // Availability is restated so it survives a purge.
                let mut pubs = vec![Publication {
                    topic: engine.identity().availability_topic.clone(),
                    payload: "online".to_string(),
                    retain: true,
                    qos: 1,
                }];
                pubs.extend(engine.full_pass(&self.source, now, mono));
                pubs
            }
            Some(engine) => engine.tick(&self.source, now, mono),
            None => Vec::new(),
        };
        let changed = !pubs.is_empty();
        self.send(pubs);
        if full && self.confirm_delivery(DELIVERY_TIMEOUT) {
            if let Some(engine) = self.engine.as_mut() {
                engine.settle();
            }
        }
        self.persist_topics();
        if changed {
            self.write_status();
        }
    }

    /// Wait until the broker has acknowledged everything sent. A broker that
    /// does not is treated like a lost connection.
    fn confirm_delivery(&mut self, timeout: Duration) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        match session.client.drain(timeout) {
            Ok(messages) => {
                self.handle_incoming(messages);
                true
            }
            Err(e) => {
                self.lost(&e);
                self.need_full = true;
                false
            }
        }
    }

    fn wait_step(&self) -> Duration {
        let mono = self.mono();
        let until_due = self
            .engine
            .as_ref()
            .and_then(Engine::next_due)
            .map_or(LOOP_STEP.as_secs_f64(), |t| (t - mono).max(0.05));
        Duration::from_secs_f64(until_due.min(LOOP_STEP.as_secs_f64()))
    }
}

/// The only incoming message the daemon reacts to: a live (non-retained)
/// `online` on the configured birth topic with discovery on. Everything else
/// is dropped, which keeps the integration read-only.
pub fn is_birth_message(cfg: &Config, msg: &Incoming) -> bool {
    cfg.identity.discovery
        && cfg.identity.birth_topic.as_deref() == Some(msg.topic.as_str())
        && msg.payload == b"online"
        && !msg.retain
}

/// Birth topic to subscribe to: only with discovery on.
fn wanted_birth(cfg: &Config) -> Option<String> {
    if cfg.identity.discovery {
        cfg.identity.birth_topic.clone()
    } else {
        None
    }
}

pub fn run(level: Option<Level>) -> i32 {
    let mut log = Log::new(level.unwrap_or(Level::Info));
    let _instance = match lock_instance(&Path::new(RUN_DIR).join("daemon.lock")) {
        Ok(lock) => lock,
        Err(e) => {
            log.error(&e);
            return 1;
        }
    };
    let signals = match Signals::register() {
        Ok(s) => s,
        Err(e) => {
            log.error(&format!("cannot install signal handlers: {e}"));
            return 1;
        }
    };
    let server = match control::Server::bind(&rpcd::control_path()) {
        Ok(s) => Some(s),
        Err(e) => {
            log.warn(&format!("control socket unavailable: {e}"));
            None
        }
    };
    let mut d = Daemon {
        log,
        source: SystemSource,
        started: Instant::now(),
        config: Err(Vec::new()),
        engine: None,
        session: None,
        conn: Connection {
            state: "starting",
            ..Connection::default()
        },
        need_full: true,
        next_attempt: 0.0,
        backoff: Backoff::new(),
        config_retry: ConfigRetry::new(),
        birth_limit: BirthLimit::default(),
        last_status: 0.0,
    };
    d.conn.since = d.now();
    d.log
        .info(&format!("mqtt-metrics {} starting", crate::VERSION));
    d.load();
    if let Some(level) = level {
        d.log = Log::new(level);
    }
    log = d.log;
    d.write_status();

    loop {
        if signals.term.load(Ordering::Relaxed) {
            log.info("stopping");
            d.disconnect(Farewell::Offline);
            d.set_state("stopped");
            d.write_status();
            let _ = std::fs::remove_file(rpcd::control_path());
            return 0;
        }
        if signals.hup.swap(false, Ordering::Relaxed) {
            log.info("SIGHUP: reloading configuration");
            d.reload();
            if d.config.is_err() {
                d.set_state("config_error");
            }
            d.write_status();
        }
        d.handle_control(server.as_ref());

        let cfg = match &d.config {
            Ok(cfg) => cfg.clone(),
            Err(_) => {
                d.set_state("config_error");
                if d.config_retry.due(d.mono()) {
                    log.info("trying to read the configuration again");
                    d.reload();
                    if d.config.is_err() {
                        d.set_state("config_error");
                    }
                    d.write_status();
                } else if d.mono() - d.last_status >= STATUS_EVERY {
                    d.write_status();
                }
                std::thread::sleep(LOOP_STEP);
                continue;
            }
        };
        if !cfg.enabled || !cfg.configured() {
            d.disconnect(Farewell::Offline);
            let state = if cfg.enabled {
                "unconfigured"
            } else {
                "disabled"
            };
            let changed = d.conn.state != state;
            d.set_state(state);
            if changed || d.mono() - d.last_status >= STATUS_EVERY {
                d.write_status();
            }
            std::thread::sleep(LOOP_STEP);
            continue;
        }

        if d.session.is_none() {
            if d.mono() >= d.next_attempt {
                d.connect(&cfg);
                d.write_status();
            } else {
                // The wait can last two minutes; the status stays current.
                if d.mono() - d.last_status >= STATUS_EVERY {
                    d.write_status();
                }
                std::thread::sleep(LOOP_STEP.min(Duration::from_secs_f64(
                    (d.next_attempt - d.mono()).max(0.05),
                )));
            }
            continue;
        }

        d.publish_round();

        let step = d.wait_step();
        let polled = d.session.as_mut().map(|s| {
            s.client
                .poll(step)
                .and_then(|m| s.client.maintain().map(|()| m))
        });
        match polled {
            Some(Ok(messages)) => {
                d.handle_incoming(messages);
                d.check_subscription();
            }
            Some(Err(e)) => {
                d.lost(&e);
                d.need_full = true;
                d.write_status();
            }
            None => {}
        }
        if d.mono() - d.last_status >= STATUS_EVERY {
            d.write_status();
        }
    }
}

/// `--purge`: clear every retained topic this router owns.
///
/// A running daemon owns those topics and has to republish afterwards, so it
/// does the work. Without a daemon, connect once and clear them directly.
pub fn purge_once() -> i32 {
    let log = Log::new(Level::Info);
    match control::send(
        &rpcd::control_path(),
        Command::Purge,
        Duration::from_secs(60),
    ) {
        Ok((true, message)) => {
            log.info(&message);
            return 0;
        }
        Ok((false, message)) => {
            log.error(&message);
            return 1;
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => {
            log.error(&format!("the running daemon did not answer: {e}"));
            return 1;
        }
    }
    let source = SystemSource;
    let (cfg, errors) = rpcd::load_config(&source);
    if !errors.is_empty() || !cfg.configured() {
        log.error("configuration is invalid or no broker is set");
        return 1;
    }
    let hostname = collect::hostname(&source);
    let mut engine = Engine::new(
        cfg.clone(),
        &hostname,
        TopicState::load(Path::new(STATE_PATH)),
    );
    let id = engine.identity().clone();
    let opts = Options {
        host: cfg.broker.host.clone(),
        port: cfg.broker.port,
        client_id: format!("{}-purge", id.client_id),
        keepalive: 30,
        username: cfg.broker.username.clone(),
        password: config::read_secret(SECRET_PATH).ok().flatten(),
        will: None,
        connect_timeout: CONNECT_TIMEOUT,
        tls: cfg.broker.tls_settings(),
    };
    let mut client = match Client::connect(&opts) {
        Ok(c) => c,
        Err(e) => {
            log.error(&format!("cannot connect: {e}"));
            return 1;
        }
    };
    let pubs = engine.purge();
    for p in &pubs {
        if let Err(e) = client.publish(&p.topic, p.payload.as_bytes(), p.qos, p.retain) {
            log.error(&format!("publish failed: {e}"));
            return 1;
        }
    }
    if let Err(e) = client.drain(Duration::from_secs(10)) {
        log.error(&format!("broker did not acknowledge: {e}"));
        return 1;
    }
    client.disconnect();
    engine.settle();
    if let Some(state) = engine.take_dirty_topics() {
        let _ = state.save(Path::new(STATE_PATH));
    }
    log.info(&format!("cleared {} retained topics", pubs.len()));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(topic: &str, payload: &[u8], retain: bool) -> Incoming {
        Incoming {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            retain,
        }
    }

    #[test]
    fn only_a_live_birth_message_is_accepted() {
        let cfg = Config::default();
        assert!(is_birth_message(
            &cfg,
            &msg("homeassistant/status", b"online", false)
        ));
        // Retained birth, other payloads, other topics and command-like
        // topics are all dropped.
        assert!(!is_birth_message(
            &cfg,
            &msg("homeassistant/status", b"online", true)
        ));
        assert!(!is_birth_message(
            &cfg,
            &msg("homeassistant/status", b"offline", false)
        ));
        for topic in [
            "openwrt/r1/set",
            "openwrt/r1/command",
            "homeassistant/button/openwrt_r1_reboot/set",
            "homeassistant/status/extra",
            "#",
        ] {
            assert!(
                !is_birth_message(&cfg, &msg(topic, b"online", false)),
                "{topic}"
            );
        }
        let mut off = Config::default();
        off.identity.discovery = false;
        assert!(!is_birth_message(
            &off,
            &msg("homeassistant/status", b"online", false)
        ));
        let mut none = Config::default();
        none.identity.birth_topic = None;
        assert!(!is_birth_message(
            &none,
            &msg("homeassistant/status", b"online", false)
        ));
    }

    #[test]
    fn announcements_from_home_assistant_are_spaced_out() {
        let mut limit = BirthLimit::default();
        assert!(limit.heard(0.0), "the first one is answered at once");
        assert!(!limit.heard(1.0));
        assert!(!limit.heard(2.0));
        assert!(!limit.due(29.0));
        assert!(limit.due(30.0), "the deferred one is answered when allowed");
        assert!(!limit.due(31.0), "and only once");
        assert!(!limit.heard(40.0));
        assert!(limit.due(60.0));
        // Quiet for a while: the next one is answered at once again.
        assert!(limit.heard(500.0));
        assert!(!limit.due(1000.0));
    }

    #[test]
    fn an_unreadable_configuration_is_retried_and_an_invalid_one_is_not() {
        let mut retry = ConfigRetry::new();
        assert!(!retry.due(1000.0), "nothing to retry after a good load");

        // uci failed: try again after 10 s, then 20 s, and so on up to 5 min.
        retry.loaded(100.0, true);
        assert!(!retry.due(109.0));
        assert!(retry.due(110.0));
        retry.loaded(110.0, true);
        assert!(!retry.due(129.0));
        assert!(retry.due(130.0));
        for _ in 0..10 {
            retry.loaded(0.0, true);
        }
        assert_eq!(retry.at, Some(CONFIG_RETRY_MAX_S));

        // An invalid configuration stays invalid until it is edited.
        retry.loaded(500.0, false);
        assert!(!retry.due(10_000.0));
        retry.loaded(600.0, true);
        assert_eq!(retry.at, Some(600.0 + CONFIG_RETRY_MIN_S));
    }

    #[test]
    fn backoff_grows_and_resets_after_a_stable_session() {
        let mut b = Backoff::new();
        let waits: Vec<f64> = (0..8).map(|i| b.failed(f64::from(i))).collect();
        assert_eq!(
            waits,
            vec![5.0, 10.0, 20.0, 40.0, 80.0, 120.0, 120.0, 120.0]
        );

        // A connection that dropped again within a minute keeps the delay.
        b.connected(100.0);
        assert_eq!(b.failed(110.0), 120.0);

        // One that lasted a minute or more starts over.
        b.connected(200.0);
        assert_eq!(b.failed(260.0), 5.0);
        assert_eq!(b.failed(265.0), 10.0);

        // A configuration change tries again from the shortest delay.
        b.reset();
        assert_eq!(b.failed(300.0), 5.0);
    }

    #[test]
    fn a_second_instance_is_refused() {
        let dir = std::env::temp_dir().join(format!("mqtt-metrics-lock-{}", std::process::id()));
        let path = dir.join("daemon.lock");
        let first = lock_instance(&path).unwrap();
        let err = lock_instance(&path).unwrap_err();
        assert!(err.contains("already running"), "{err}");
        drop(first);
        // A helper process forked by another test thread holds a copy of the
        // descriptor until it calls exec, so the lock can take a moment to
        // become free.
        let freed = (0..300).any(|_| {
            let ok = lock_instance(&path).is_ok();
            if !ok {
                std::thread::sleep(Duration::from_millis(10));
            }
            ok
        });
        assert!(freed, "the lock is free once the first instance is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
