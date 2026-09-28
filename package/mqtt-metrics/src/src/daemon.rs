// SPDX-License-Identifier: GPL-2.0-or-later
//! The long-running publisher.
//!
//! One thread runs everything: signal flags, the control socket, the MQTT
//! connection with bounded reconnect backoff, and the publish engine. Every
//! blocking call has a timeout so the loop keeps turning.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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
const BACKOFF_MIN_S: f64 = 5.0;
const BACKOFF_MAX_S: f64 = 120.0;

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
    backoff: f64,
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
        if reconnect {
            self.log
                .info("broker settings or device identity changed; reconnecting");
            self.disconnect(farewell);
            self.next_attempt = self.mono();
            self.backoff = BACKOFF_MIN_S;
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
                let mut setup = client
                    .publish(&id.availability_topic, b"online", 1, true)
                    .map(|_| ());
                if setup.is_ok() {
                    if let Some(birth) = &cfg.identity.birth_topic {
                        if cfg.identity.discovery {
                            setup = client.subscribe(birth);
                        }
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
                        });
                        self.conn.last_connect = Some(self.now());
                        self.conn.last_error = None;
                        self.conn.next_attempt = None;
                        self.backoff = BACKOFF_MIN_S;
                        self.need_full = true;
                        self.set_state("connected");
                    }
                    Err(e) => self.connect_failed(&e),
                }
            }
            Err(e) => self.connect_failed(&e),
        }
    }

    fn connect_failed(&mut self, e: &MqttError) {
        let msg = e.to_string();
        if self.conn.last_error.as_deref() != Some(msg.as_str()) {
            self.log
                .warn(&format!("cannot connect to {}: {msg}", self.conn.broker));
        }
        self.conn.last_error = Some(msg);
        self.conn.reconnects += 1;
        self.next_attempt = self.mono() + self.backoff;
        self.conn.next_attempt = Some(self.now() + self.backoff as u64);
        self.backoff = (self.backoff * 2.0).min(BACKOFF_MAX_S);
        self.set_state("disconnected");
    }

    fn lost(&mut self, e: &MqttError) {
        self.log.warn(&format!("connection lost: {e}"));
        self.session = None;
        self.conn.last_error = Some(e.to_string());
        self.next_attempt = self.mono() + BACKOFF_MIN_S;
        self.conn.next_attempt = Some(self.now() + BACKOFF_MIN_S as u64);
        self.set_state("disconnected");
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
        let Ok(cfg) = &self.config else { return false };
        cfg.identity.birth_topic.as_deref() == Some(msg.topic.as_str())
            && msg.payload == b"online"
            && !msg.retain
    }

    fn handle_incoming(&mut self, messages: Vec<Incoming>) {
        for msg in messages {
            if self.is_birth(&msg) {
                self.log
                    .info("Home Assistant came online; resending discovery");
                self.need_full = true;
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
                        if let Some(s) = self.session.as_mut() {
                            let _ = s.client.drain(Duration::from_secs(5));
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
        let pubs = match self.engine.as_mut() {
            Some(engine) if self.need_full => {
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
        self.persist_topics();
        if changed {
            self.write_status();
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

pub fn run(level: Option<Level>) -> i32 {
    let mut log = Log::new(level.unwrap_or(Level::Info));
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
        backoff: BACKOFF_MIN_S,
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
        }
        d.handle_control(server.as_ref());

        let cfg = match &d.config {
            Ok(cfg) => cfg.clone(),
            Err(_) => {
                d.set_state("config_error");
                if d.mono() - d.last_status >= STATUS_EVERY {
                    d.write_status();
                }
                std::thread::sleep(LOOP_STEP);
                continue;
            }
        };
        if !cfg.enabled || !cfg.configured() {
            d.disconnect(Farewell::Offline);
            d.set_state(if cfg.enabled {
                "unconfigured"
            } else {
                "disabled"
            });
            if d.mono() - d.last_status >= STATUS_EVERY {
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
            Some(Ok(messages)) => d.handle_incoming(messages),
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

/// `--purge`: connect once and clear every retained topic this router owns.
pub fn purge_once() -> i32 {
    let log = Log::new(Level::Info);
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
    if let Some(state) = engine.take_dirty_topics() {
        let _ = state.save(Path::new(STATE_PATH));
    }
    log.info(&format!("cleared {} retained topics", pubs.len()));
    let _ = std::fs::create_dir_all(RUN_DIR);
    0
}
