// SPDX-License-Identifier: GPL-2.0-or-later
//! UCI configuration (`/etc/config/mqtt-metrics`).
//!
//! The configuration is read and validated once per (re)load. The broker
//! password is not part of UCI: LuCI can read every UCI option, so the password lives in a
//! root-only file ([`SECRET_PATH`]) that the browser can only overwrite.

use std::collections::BTreeMap;

use crate::catalog::Group;
use crate::log::Level;
use crate::text::{valid_object_id, valid_topic};
use crate::uci::{self, Package, Section};

pub const CONFIG_NAME: &str = "mqtt-metrics";
pub const SECRET_PATH: &str = "/etc/mqtt-metrics/secret";
pub const STATE_PATH: &str = "/etc/mqtt-metrics/state.json";
pub const RUN_DIR: &str = "/var/run/mqtt-metrics";

pub const MIN_INTERVAL: u32 = 5;
pub const MAX_INTERVAL: u32 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tls {
    pub enabled: bool,
    pub ca: Option<String>,
    pub verify: bool,
    pub cert: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub client_id: Option<String>,
    pub keepalive: u16,
    pub qos: u8,
    pub retain: bool,
    pub tls: Tls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub base_topic: String,
    /// Topic level for this router; `None` means the runtime hostname.
    pub node: Option<String>,
    pub discovery: bool,
    pub discovery_prefix: String,
    /// `None` disables the Home Assistant birth subscription (`birth = 0` or
    /// an empty `birth_topic`).
    pub birth_topic: Option<String>,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub manufacturer: String,
    pub model: String,
    pub configuration_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub interval: u32,
    pub change_only: bool,
    pub max_age: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    /// `true`: interfaces matching the automatic `wan*`/`wwan*` rule.
    pub interfaces_auto: bool,
    pub interfaces: Vec<String>,
    /// Devices for traffic metrics; empty means the devices of the selected
    /// interfaces.
    pub devices: Vec<String>,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupConfig {
    pub enabled: bool,
    pub interval: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricOverride {
    pub publish: Option<bool>,
    pub discovery: Option<bool>,
    pub ha_enabled: Option<bool>,
    pub diagnostic: Option<bool>,
    pub name: Option<String>,
    pub object_id: Option<String>,
    pub topic: Option<String>,
    pub icon: Option<String>,
    pub interval: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub log_level: Level,
    pub broker: Broker,
    pub identity: Identity,
    pub publish: Publish,
    pub select: Select,
    pub groups: BTreeMap<Group, GroupConfig>,
    pub metrics: BTreeMap<String, MetricOverride>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            enabled: true,
            log_level: Level::Info,
            broker: Broker {
                host: String::new(),
                port: 1883,
                username: None,
                client_id: None,
                keepalive: 60,
                qos: 1,
                retain: true,
                tls: Tls {
                    enabled: false,
                    ca: None,
                    verify: true,
                    cert: None,
                    key: None,
                },
            },
            identity: Identity {
                base_topic: "openwrt".to_string(),
                node: None,
                discovery: true,
                discovery_prefix: "homeassistant".to_string(),
                birth_topic: Some("homeassistant/status".to_string()),
                device_id: None,
                device_name: None,
                manufacturer: "OpenWrt".to_string(),
                model: "Router".to_string(),
                configuration_url: None,
            },
            publish: Publish {
                interval: 300,
                change_only: false,
                max_age: 3600,
            },
            select: Select {
                interfaces_auto: true,
                interfaces: Vec::new(),
                devices: Vec::new(),
                services: Vec::new(),
            },
            groups: BTreeMap::new(),
            metrics: BTreeMap::new(),
        }
    }
}

impl Broker {
    /// Transport settings for the MQTT client; `None` for plain TCP.
    pub fn tls_settings(&self) -> Option<crate::mqtt::transport::TlsSettings> {
        self.tls
            .enabled
            .then(|| crate::mqtt::transport::TlsSettings {
                ca_file: self.tls.ca.clone(),
                verify: self.tls.verify,
                cert_file: self.tls.cert.clone(),
                key_file: self.tls.key.clone(),
            })
    }
}

impl Config {
    pub fn group(&self, group: Group) -> GroupConfig {
        self.groups.get(&group).cloned().unwrap_or(GroupConfig {
            enabled: true,
            interval: None,
        })
    }

    /// `true` when a broker host is set; without it the daemon idles.
    pub fn configured(&self) -> bool {
        !self.broker.host.is_empty()
    }
}

struct Reader<'a> {
    errors: Vec<String>,
    section: Option<&'a Section>,
    name: &'a str,
}

impl<'a> Reader<'a> {
    fn raw(&self, option: &str) -> Option<&'a str> {
        self.section.and_then(|s| s.get(option)).map(str::trim)
    }

    fn string(&self, option: &str) -> Option<String> {
        self.raw(option)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    }

    fn bool(&mut self, option: &str, default: bool) -> bool {
        match self.raw(option) {
            None | Some("") => default,
            Some(v) => parse_bool(v).unwrap_or_else(|| {
                self.errors
                    .push(format!("{}.{option}: '{v}' is not a boolean", self.name));
                default
            }),
        }
    }

    fn opt_bool(&mut self, option: &str) -> Option<bool> {
        match self.raw(option) {
            None | Some("") => None,
            Some(v) => {
                let parsed = parse_bool(v);
                if parsed.is_none() {
                    self.errors
                        .push(format!("{}.{option}: '{v}' is not a boolean", self.name));
                }
                parsed
            }
        }
    }

    fn number(&mut self, option: &str, min: u32, max: u32) -> Option<u32> {
        match self.raw(option) {
            None | Some("") => None,
            Some(v) => match v.parse::<u32>() {
                Ok(n) if (min..=max).contains(&n) => Some(n),
                _ => {
                    self.errors.push(format!(
                        "{}.{option}: '{v}' must be a number from {min} to {max}",
                        self.name
                    ));
                    None
                }
            },
        }
    }

    fn topic(&mut self, option: &str) -> Option<String> {
        let value = self.string(option)?;
        if valid_topic(&value) {
            Some(value)
        } else {
            self.errors.push(format!(
                "{}.{option}: '{value}' is not a valid topic (no wildcards, no empty levels)",
                self.name
            ));
            None
        }
    }
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "yes" | "on" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn reader<'a>(pkg: &'a Package, name: &'a str, errors: Vec<String>) -> Reader<'a> {
    Reader {
        errors,
        section: pkg.section(name),
        name,
    }
}

/// Build a configuration from parsed UCI data. All problems are collected and
/// returned together so LuCI and the log can show them at once.
pub fn from_uci(pkg: &Package) -> Result<Config, Vec<String>> {
    let (cfg, errors) = from_uci_lenient(pkg);
    if errors.is_empty() {
        Ok(cfg)
    } else {
        Err(errors)
    }
}

/// The same parse, returning what could be read together with the problems.
/// An invalid value keeps its default. The daemon refuses a configuration with
/// problems; this is for describing the stored settings (LuCI) without
/// presenting defaults in their place.
pub fn from_uci_lenient(pkg: &Package) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut errors = Vec::new();

    let mut r = reader(pkg, "main", errors);
    cfg.enabled = r.bool("enabled", true);
    if let Some(level) = r.string("log_level") {
        match Level::parse(&level) {
            Some(l) => cfg.log_level = l,
            None => r.errors.push(format!(
                "main.log_level: '{level}' is not error, warn, info or debug"
            )),
        }
    }
    errors = r.errors;

    let mut r = reader(pkg, "broker", errors);
    cfg.broker.host = r.string("host").unwrap_or_default();
    if cfg.broker.host.contains(char::is_whitespace) || cfg.broker.host.len() > 253 {
        r.errors.push(format!(
            "broker.host: '{}' is not a host name or address",
            cfg.broker.host
        ));
    }
    if let Some(port) = r.number("port", 1, 65_535) {
        cfg.broker.port = u16::try_from(port).unwrap_or(1883);
    }
    cfg.broker.username = r.string("username");
    cfg.broker.client_id = r.string("client_id");
    if let Some(id) = &cfg.broker.client_id {
        if !valid_object_id(id) {
            r.errors.push(format!(
                "broker.client_id: '{id}' may contain only letters, digits, '_' and '-'"
            ));
        }
    }
    if let Some(keepalive) = r.number("keepalive", 5, 3600) {
        cfg.broker.keepalive = u16::try_from(keepalive).unwrap_or(60);
    }
    if let Some(qos) = r.number("qos", 0, 1) {
        cfg.broker.qos = u8::try_from(qos).unwrap_or(1);
    }
    cfg.broker.retain = r.bool("retain", true);
    cfg.broker.tls.enabled = r.bool("tls", false);
    cfg.broker.tls.ca = r.string("tls_ca");
    cfg.broker.tls.verify = r.bool("tls_verify", true);
    cfg.broker.tls.cert = r.string("tls_cert");
    cfg.broker.tls.key = r.string("tls_key");
    if cfg.broker.tls.cert.is_some() != cfg.broker.tls.key.is_some() {
        r.errors
            .push("broker.tls_cert and broker.tls_key must be set together".to_string());
    }
    if cfg.broker.tls.enabled && !crate::mqtt::TLS_SUPPORTED {
        r.errors
            .push("broker.tls: this build has no TLS support".to_string());
    }
    errors = r.errors;

    let mut r = reader(pkg, "identity", errors);
    if let Some(v) = r.topic("base_topic") {
        cfg.identity.base_topic = v;
    }
    cfg.identity.node = r.string("node");
    if let Some(node) = &cfg.identity.node {
        if node.contains(['/', '+', '#', '\0']) {
            r.errors.push(format!(
                "identity.node: '{node}' must be a single topic level"
            ));
        }
    }
    cfg.identity.discovery = r.bool("discovery", true);
    if let Some(v) = r.topic("discovery_prefix") {
        cfg.identity.discovery_prefix = v;
    }
    match r.raw("birth_topic") {
        None => {}
        Some("") => cfg.identity.birth_topic = None,
        Some(_) => cfg.identity.birth_topic = r.topic("birth_topic"),
    }
    // LuCI cannot store an empty option, so the subscription has its own
    // switch; an empty `birth_topic` set with `uci` still turns it off.
    if !r.bool("birth", true) {
        cfg.identity.birth_topic = None;
    }
    cfg.identity.device_id = r.string("device_id");
    if let Some(id) = &cfg.identity.device_id {
        if !valid_object_id(id) {
            r.errors.push(format!(
                "identity.device_id: '{id}' may contain only letters, digits, '_' and '-'"
            ));
        }
    }
    cfg.identity.device_name = r.string("device_name");
    if let Some(v) = r.string("manufacturer") {
        cfg.identity.manufacturer = v;
    }
    if let Some(v) = r.string("model") {
        cfg.identity.model = v;
    }
    cfg.identity.configuration_url = r.string("configuration_url");
    errors = r.errors;

    let mut r = reader(pkg, "publish", errors);
    if let Some(v) = r.number("interval", MIN_INTERVAL, MAX_INTERVAL) {
        cfg.publish.interval = v;
    }
    cfg.publish.change_only = r.bool("change_only", false);
    if let Some(v) = r.number("max_age", MIN_INTERVAL, 7 * MAX_INTERVAL) {
        cfg.publish.max_age = v;
    }
    errors = r.errors;

    let mut r = reader(pkg, "select", errors);
    match r.string("interfaces").as_deref() {
        None | Some("auto") => cfg.select.interfaces_auto = true,
        Some("manual") => cfg.select.interfaces_auto = false,
        Some(other) => r.errors.push(format!(
            "select.interfaces: '{other}' is not auto or manual"
        )),
    }
    if let Some(section) = r.section {
        cfg.select.interfaces = section.list("interface");
        cfg.select.devices = section.list("device");
        cfg.select.services = section.list("service");
    }
    errors = r.errors;

    for section in pkg.sections_of_type("group") {
        let Some(group) = Group::parse(&section.name) else {
            errors.push(format!("group '{}' is unknown", section.name));
            continue;
        };
        let mut r = reader(pkg, &section.name, std::mem::take(&mut errors));
        let enabled = r.bool("enabled", true);
        let interval = r.number("interval", MIN_INTERVAL, MAX_INTERVAL);
        errors = r.errors;
        cfg.groups.insert(group, GroupConfig { enabled, interval });
    }

    for section in pkg.sections_of_type("metric") {
        let mut r = reader(pkg, &section.name, std::mem::take(&mut errors));
        let over = MetricOverride {
            publish: r.opt_bool("publish"),
            discovery: r.opt_bool("discovery"),
            ha_enabled: r.opt_bool("ha_enabled"),
            diagnostic: r.opt_bool("diagnostic"),
            name: r.string("name"),
            object_id: r.string("object_id"),
            topic: r.topic("topic"),
            icon: r.string("icon"),
            interval: r.number("interval", MIN_INTERVAL, MAX_INTERVAL),
        };
        if let Some(id) = &over.object_id {
            if !valid_object_id(id) {
                r.errors.push(format!(
                    "{}.object_id: '{id}' may contain only letters, digits, '_' and '-'",
                    section.name
                ));
            }
        }
        if let Some(icon) = &over.icon {
            let ok = icon.strip_prefix("mdi:").is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            });
            if !ok {
                r.errors.push(format!(
                    "{}.icon: '{icon}' is not an mdi: icon name",
                    section.name
                ));
            }
        }
        if over.name.as_ref().is_some_and(|n| n.len() > 128) {
            r.errors
                .push(format!("{}.name: longer than 128 characters", section.name));
        }
        errors = r.errors;
        cfg.metrics.insert(section.name.clone(), over);
    }

    (cfg, errors)
}

/// Parse `uci show mqtt-metrics` output.
pub fn from_uci_text(text: &str) -> Result<Config, Vec<String>> {
    from_uci(&uci::parse(CONFIG_NAME, text))
}

/// Parse `uci show mqtt-metrics` output, keeping the valid settings when some
/// are not.
pub fn from_uci_text_lenient(text: &str) -> (Config, Vec<String>) {
    from_uci_lenient(&uci::parse(CONFIG_NAME, text))
}

/// Read the password file. A missing file means "no password".
pub fn read_secret(path: &str) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let value = text.trim_end_matches(['\n', '\r']).to_string();
            Ok(if value.is_empty() { None } else { Some(value) })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_FILE: &str = include_str!("../../files/etc/config/mqtt-metrics");

    fn uci_show(conf: &str) -> String {
        // Minimal converter from the UCI file format to `uci show` output for
        // tests of the shipped default file.
        let mut out = String::new();
        let mut section = String::new();
        let mut anon = 0;
        for line in conf.lines() {
            let line = line.trim();
            let words: Vec<&str> = line.splitn(3, ' ').collect();
            match words.as_slice() {
                ["config", stype, name] => {
                    section = name.trim_matches('\'').to_string();
                    out.push_str(&format!("mqtt-metrics.{section}={stype}\n"));
                }
                ["config", stype] => {
                    section = format!("@{stype}[{anon}]");
                    anon += 1;
                    out.push_str(&format!("mqtt-metrics.{section}={stype}\n"));
                }
                ["option", key, value] | ["list", key, value] => {
                    out.push_str(&format!("mqtt-metrics.{section}.{key}={value}\n"));
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn shipped_default_file_equals_builtin_defaults() {
        let cfg = from_uci_text(&uci_show(DEFAULT_FILE)).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn parses_overrides_and_selection() {
        let text = "\
mqtt-metrics.broker=broker
mqtt-metrics.broker.host='192.0.2.10'
mqtt-metrics.broker.port='8883'
mqtt-metrics.broker.username='openwrt'
mqtt-metrics.identity=identity
mqtt-metrics.identity.base_topic='openwrt-shadow'
mqtt-metrics.identity.discovery='0'
mqtt-metrics.identity.birth_topic=''
mqtt-metrics.publish=publish
mqtt-metrics.publish.interval='60'
mqtt-metrics.publish.change_only='1'
mqtt-metrics.select=select
mqtt-metrics.select.interfaces='manual'
mqtt-metrics.select.interface='wan' 'lan'
mqtt-metrics.traffic=group
mqtt-metrics.traffic.interval='30'
mqtt-metrics.system_uptime=metric
mqtt-metrics.system_uptime.publish='0'
mqtt-metrics.system_uptime.name='Router Uptime'
mqtt-metrics.system_uptime.icon='mdi:timer'
";
        let cfg = from_uci_text(text).unwrap();
        assert_eq!(cfg.broker.host, "192.0.2.10");
        assert_eq!(cfg.broker.port, 8883);
        assert_eq!(cfg.broker.username.as_deref(), Some("openwrt"));
        assert_eq!(cfg.identity.base_topic, "openwrt-shadow");
        assert!(!cfg.identity.discovery);
        assert_eq!(cfg.identity.birth_topic, None);
        assert_eq!(cfg.publish.interval, 60);
        assert!(cfg.publish.change_only);
        assert!(!cfg.select.interfaces_auto);
        assert_eq!(cfg.select.interfaces, vec!["wan", "lan"]);
        assert_eq!(cfg.group(Group::Traffic).interval, Some(30));
        assert!(cfg.group(Group::System).enabled);
        let over = &cfg.metrics["system_uptime"];
        assert_eq!(over.publish, Some(false));
        assert_eq!(over.name.as_deref(), Some("Router Uptime"));
    }

    #[test]
    fn the_birth_subscription_can_be_switched_off() {
        let on = from_uci_text("mqtt-metrics.identity=identity\n").unwrap();
        assert_eq!(
            on.identity.birth_topic.as_deref(),
            Some("homeassistant/status")
        );
        for text in [
            "mqtt-metrics.identity=identity\nmqtt-metrics.identity.birth='0'\n",
            "mqtt-metrics.identity=identity\nmqtt-metrics.identity.birth_topic=''\n",
        ] {
            assert_eq!(from_uci_text(text).unwrap().identity.birth_topic, None);
        }
        let custom = from_uci_text(
            "mqtt-metrics.identity=identity\nmqtt-metrics.identity.birth_topic='ha/status'\n",
        )
        .unwrap();
        assert_eq!(custom.identity.birth_topic.as_deref(), Some("ha/status"));
    }

    #[test]
    fn collects_all_errors() {
        let text = "\
mqtt-metrics.broker=broker
mqtt-metrics.broker.port='0'
mqtt-metrics.broker.qos='2'
mqtt-metrics.identity=identity
mqtt-metrics.identity.base_topic='a/+/b'
mqtt-metrics.identity.node='x/y'
mqtt-metrics.publish=publish
mqtt-metrics.publish.interval='1'
mqtt-metrics.bogus=group
mqtt-metrics.m=metric
mqtt-metrics.m.object_id='bad id'
mqtt-metrics.m.icon='timer'
mqtt-metrics.m.publish='maybe'
";
        let errors = from_uci_text(text).unwrap_err();
        assert_eq!(errors.len(), 9, "{errors:#?}");
    }
}
