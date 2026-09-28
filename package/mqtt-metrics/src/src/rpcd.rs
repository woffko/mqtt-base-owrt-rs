// SPDX-License-Identifier: GPL-2.0-or-later
//! rpcd exec plugin (`/usr/libexec/rpcd/mqtt-metrics`, a symlink to the
//! daemon binary). rpcd runs `mqtt-metrics list` to learn the methods and
//! `mqtt-metrics call <method>` with the arguments as JSON on stdin.
//!
//! Replies never contain the broker password. Read methods are `status`,
//! `catalog`, `preview`, `legacy` and `log`; everything else is a write
//! method in the ACL.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::catalog::{Group, ALL};
use crate::collect::{self, net, CollectorState};
use crate::config::{self, Config, RUN_DIR, SECRET_PATH};
use crate::control::{self, Command};
use crate::engine::{self, Entry};
use crate::exec;
use crate::ha::Identity;
use crate::legacy;
use crate::mqtt::{Client, Options};
use crate::source::{Source, SystemSource};
use crate::state::write_atomic;

macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

pub fn status_path() -> PathBuf {
    Path::new(RUN_DIR).join("status.json")
}

pub fn control_path() -> PathBuf {
    Path::new(RUN_DIR).join("control.sock")
}

pub fn methods() -> Value {
    json!({
        "status": {},
        "catalog": {},
        "preview": { "key": "str" },
        "legacy": {},
        "log": {},
        "test_connection": {
            "host": "str", "port": 1883, "username": "str", "password": "str",
            "keep_password": true, "client_id": "str", "tls": true
        },
        "set_password": { "password": "str" },
        "import_legacy": {},
        "republish": {},
        "purge": {},
        "reload": {}
    })
}

pub fn call(method: &str, args: &Value) -> Value {
    match method {
        "status" => status(),
        "catalog" => catalog(&SystemSource),
        "preview" => preview(&SystemSource, args),
        "legacy" => legacy_status(),
        "log" => log(),
        "test_connection" => test_connection(args),
        "set_password" => set_password(args),
        "import_legacy" => import_legacy(),
        "republish" => control_command(Command::Republish),
        "purge" => control_command(Command::Purge),
        "reload" => control_command(Command::Reload),
        _ => json!({ "error": format!("unknown method {method}") }),
    }
}

/// Entry point for `list` / `call <method>`.
pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("list") => {
            out!("{}", methods());
            0
        }
        Some("call") => {
            let Some(method) = args.get(1) else {
                out!("{}", json!({ "error": "missing method" }));
                return 1;
            };
            let mut input = String::new();
            let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut input);
            let params: Value = serde_json::from_str(input.trim()).unwrap_or_else(|_| json!({}));
            out!("{}", call(method, &params));
            0
        }
        _ => 1,
    }
}

/// Load the configuration; on errors fall back to defaults and report them.
pub fn load_config(source: &dyn Source) -> (Config, Vec<String>) {
    let path = format!("/etc/config/{}", config::CONFIG_NAME);
    match source.command("uci", &["-q", "show", config::CONFIG_NAME]) {
        Ok(text) => match config::from_uci_text(&text) {
            Ok(cfg) => (cfg, Vec::new()),
            Err(errors) => (Config::default(), errors),
        },
        // No `uci` binary (tests, fixtures) or no configuration file (package
        // files not installed): run with the defaults.
        Err(crate::source::SourceError::Missing(_)) => (Config::default(), Vec::new()),
        Err(_)
            if matches!(
                source.read(&path),
                Err(crate::source::SourceError::Missing(_))
            ) =>
        {
            (Config::default(), Vec::new())
        }
        Err(e) => (Config::default(), vec![format!("cannot read {path}: {e}")]),
    }
}

fn read_status() -> Option<Value> {
    let text = std::fs::read_to_string(status_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn status() -> Value {
    let Some(mut doc) = read_status() else {
        return json!({ "state": "stopped", "running": false });
    };
    let running = doc
        .get("pid")
        .and_then(Value::as_u64)
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists());
    doc["running"] = json!(running);
    if !running {
        doc["state"] = json!("stopped");
    }
    doc
}

fn entry_json(e: &Entry, cfg: &Config, last: Option<&Value>) -> Value {
    let defaults_over = cfg.metrics.get(&e.key).cloned().unwrap_or_default();
    let (available, reason, permanent) = match &e.reading {
        collect::Reading::Value(_) => (true, None, false),
        collect::Reading::Unavailable { reason, permanent } => {
            (false, Some(reason.clone()), *permanent)
        }
    };
    json!({
        "key": e.key,
        "template": e.def.id,
        "group": e.def.group.id(),
        "component": e.def.kind.component(),
        "instance": e.inst.name,
        "sub_instance": e.inst.sub,
        "name": e.name,
        "default_name": catalog_expand(e.def.name, e),
        "object_id": e.object_id,
        "default_object_id": engine::default_object(e.def, &e.inst),
        "topic": e.state_topic,
        "default_topic": catalog_expand(e.def.topic, e),
        "topic_suffix": defaults_over.topic.clone().unwrap_or_else(|| catalog_expand(e.def.topic, e)),
        "unique_id": e.unique_id,
        "icon": e.icon,
        "default_icon": e.def.icon,
        "unit": e.def.unit,
        "device_class": e.def.device_class,
        "value": e.payload(),
        "last_published": last,
        "available": available,
        "reason": reason,
        "permanent": permanent,
        "publish": e.publish,
        "default_publish": e.def.legacy,
        "discovery": e.discovery,
        "announced": e.announce(cfg),
        "ha_enabled": e.ha_enabled,
        "diagnostic": e.diagnostic,
        "default_diagnostic": e.def.diagnostic,
        "interval": e.interval,
        "interval_override": defaults_over.interval,
        "legacy": e.def.legacy,
        "recommended": e.def.recommended,
        "overridden": cfg.metrics.contains_key(&e.key),
    })
}

fn catalog_expand(template: &str, e: &Entry) -> String {
    crate::catalog::expand(template, &e.inst)
}

fn full_round(source: &dyn Source, cfg: &Config) -> (Identity, engine::Round) {
    let id = Identity::resolve(cfg, &collect::hostname(source));
    let groups: Vec<Group> = Group::ALL.to_vec();
    let round = engine::collect_round(
        &groups,
        cfg,
        &id,
        source,
        source.now(),
        0.0,
        &mut CollectorState::default(),
        false,
    );
    (id, round)
}

pub fn catalog(source: &dyn Source) -> Value {
    let started = Instant::now();
    let (cfg, errors) = load_config(source);
    let (id, round) = full_round(source, &cfg);
    let status = read_status();
    let last_values: Map<String, Value> = status
        .as_ref()
        .and_then(|s| s.get("metrics"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some((m.get("key")?.as_str()?.to_string(), m.get("value")?.clone()))
                })
                .collect()
        })
        .unwrap_or_default();

    let metrics: Vec<Value> = round
        .entries
        .iter()
        .map(|e| entry_json(e, &cfg, last_values.get(&e.key)))
        .collect();

    let groups: Vec<Value> = Group::ALL
        .iter()
        .map(|g| {
            let gc = cfg.group(*g);
            let missing: Vec<Value> = round
                .missing
                .iter()
                .filter(|(mg, _, _)| mg == g)
                .map(|(_, d, r)| json!({ "template": d.id, "name": d.name, "reason": r }))
                .collect();
            json!({
                "id": g.id(),
                "title": g.title(),
                "enabled": gc.enabled,
                "interval": gc.interval,
                "templates": ALL.iter().filter(|d| d.group == *g).count(),
                "missing": missing,
            })
        })
        .collect();

    let net_view = net::view(source, &cfg).ok();
    let interfaces: Vec<Value> = net_view
        .as_ref()
        .map(|v| {
            let mut names: Vec<String> = v.uci.iter().map(|i| i.name.clone()).collect();
            for name in v.dump.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            names
                .iter()
                .filter(|n| n.as_str() != "loopback")
                .map(|n| {
                    json!({
                        "name": n,
                        "proto": v.proto(n),
                        "legacy_wan": net::legacy_wan(n),
                        "selected": v.selected.iter().any(|(s, _)| s == n),
                        "device": v.dump.get(n).and_then(net::l3_device),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let devices: Vec<String> = source
        .ubus("network.device", "status", None)
        .ok()
        .and_then(|d| {
            d.as_object()
                .map(|m| m.keys().filter(|k| k.as_str() != "lo").cloned().collect())
        })
        .unwrap_or_default();
    let services: Vec<String> = source
        .ubus("service", "list", None)
        .map(|l| collect_services(&l))
        .unwrap_or_default();

    json!({
        "config_errors": errors,
        "identity": {
            "hostname": id.hostname,
            "base_topic": id.base,
            "device_id": id.device_id,
            "device_name": id.device_name,
            "discovery_prefix": id.discovery_prefix,
            "availability_topic": id.availability_topic,
            "client_id": id.client_id,
            "discovery": cfg.identity.discovery,
        },
        "defaults": {
            "interval": cfg.publish.interval,
            "min_interval": config::MIN_INTERVAL,
            "max_interval": config::MAX_INTERVAL,
        },
        "groups": groups,
        "metrics": metrics,
        "interfaces": interfaces,
        "devices": devices,
        "services": services,
        "selected_devices": cfg.select.devices,
        "selected_services": cfg.select.services,
        "password_set": config::read_secret(SECRET_PATH).ok().flatten().is_some(),
        "tls_supported": crate::mqtt::TLS_SUPPORTED,
        "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn collect_services(list: &Value) -> Vec<String> {
    let mut names = collect::services_available(list);
    names.sort();
    names
}

fn preview(source: &dyn Source, args: &Value) -> Value {
    let Some(key) = args.get("key").and_then(Value::as_str) else {
        return json!({ "error": "missing key" });
    };
    let (cfg, _) = load_config(source);
    let (id, round) = full_round(source, &cfg);
    match round.entries.iter().find(|e| e.key == key) {
        Some(e) => json!({
            "key": e.key,
            "config_topic": e.config_topic,
            "state_topic": e.state_topic,
            "announced": e.announce(&cfg),
            "payload": serde_json::from_str::<Value>(&e.discovery_json(&id)).unwrap_or(Value::Null),
        }),
        None => json!({ "error": format!("metric {key} is not available on this router") }),
    }
}

fn legacy_status() -> Value {
    let d = legacy::detect();
    let broker = std::fs::read_to_string(legacy::SCRIPT)
        .ok()
        .and_then(|t| legacy::parse_script(&t));
    json!({
        "script": d.script,
        "marker": d.marker,
        "cron": d.cron,
        "rc_local": d.rc_local,
        "active": d.cron,
        "importable": broker.is_some(),
        "broker_host": broker.as_ref().map(|b| b.host.clone()),
        "broker_username": broker.as_ref().and_then(|b| b.username.clone()),
    })
}

fn log() -> Value {
    match exec::run(
        "logread",
        &["-e", "mqtt-metrics"],
        None,
        Duration::from_secs(10),
        1024 * 1024,
    ) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(200);
            json!({ "lines": lines[start..] })
        }
        Err(e) => json!({ "error": format!("logread: {e}") }),
    }
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn test_connection(args: &Value) -> Value {
    let (cfg, _) = load_config(&SystemSource);
    let host = arg_str(args, "host").unwrap_or_else(|| cfg.broker.host.clone());
    if host.is_empty() {
        return json!({ "ok": false, "error": "no broker host" });
    }
    let port = args
        .get("port")
        .and_then(|p| {
            p.as_u64()
                .or_else(|| p.as_str().and_then(|s| s.parse().ok()))
        })
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(cfg.broker.port);
    let keep = args
        .get("keep_password")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let password = if keep {
        config::read_secret(SECRET_PATH).ok().flatten()
    } else {
        args.get("password")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let opts = Options {
        host: host.clone(),
        port,
        // A separate client id so the test never takes over the daemon's session.
        client_id: format!("mqtt-metrics-test-{}", std::process::id()),
        keepalive: 30,
        username: arg_str(args, "username").or(cfg.broker.username.clone()),
        password,
        will: None,
        connect_timeout: Duration::from_secs(5),
        tls: match args.get("tls").and_then(Value::as_bool) {
            Some(false) => None,
            Some(true) => {
                let mut broker = cfg.broker.clone();
                broker.tls.enabled = true;
                broker.tls_settings()
            }
            None => cfg.broker.tls_settings(),
        },
    };
    let started = Instant::now();
    match Client::connect(&opts) {
        Ok(client) => {
            client.disconnect();
            json!({
                "ok": true,
                "message": format!("connected to {host}:{port}"),
                "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

fn reload_daemon() -> Option<String> {
    match control::send(&control_path(), Command::Reload, Duration::from_secs(10)) {
        Ok((true, _)) => None,
        Ok((false, msg)) => Some(msg),
        Err(e) => Some(format!("the service is not running ({e})")),
    }
}

fn set_password(args: &Value) -> Value {
    let password = args.get("password").and_then(Value::as_str).unwrap_or("");
    if password.len() > 1024 || password.contains(['\n', '\r', '\0']) {
        return json!({ "ok": false, "error": "invalid password" });
    }
    let path = Path::new(SECRET_PATH);
    let result = if password.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        write_atomic(path, password.as_bytes(), 0o600)
    };
    match result {
        Ok(()) => {
            json!({ "ok": true, "password_set": !password.is_empty(), "reload_warning": reload_daemon() })
        }
        Err(e) => json!({ "ok": false, "error": format!("{SECRET_PATH}: {e}") }),
    }
}

fn import_legacy() -> Value {
    match legacy::import(Path::new(legacy::SCRIPT)) {
        Ok(b) => json!({
            "ok": true,
            "host": b.host,
            "port": b.port,
            "username": b.username,
            "password_set": b.password.is_some(),
            "reload_warning": reload_daemon(),
        }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn control_command(command: Command) -> Value {
    match control::send(&control_path(), command, Duration::from_secs(20)) {
        Ok((ok, message)) => json!({ "ok": ok, "message": message }),
        Err(e) => json!({ "ok": false, "error": format!("the service is not running ({e})") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::fixture;

    #[test]
    fn catalog_from_fixture_lists_everything() {
        let src = fixture("primary");
        let doc = catalog(&src);
        let metrics = doc["metrics"].as_array().unwrap();
        assert!(metrics.len() > 60, "{}", metrics.len());
        let uptime = metrics
            .iter()
            .find(|m| m["key"] == "system_uptime")
            .unwrap();
        assert_eq!(uptime["publish"], true);
        assert_eq!(uptime["legacy"], true);
        assert_eq!(uptime["topic"], "openwrt/router-primary/system/uptime");
        let load = metrics
            .iter()
            .find(|m| m["key"] == "system_load_1")
            .unwrap();
        assert_eq!(load["publish"], false);
        assert_eq!(load["recommended"], true);
        let wifi = doc["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == "wifi")
            .unwrap();
        assert_eq!(wifi["missing"].as_array().unwrap().len(), 3);
        let ifaces = doc["interfaces"].as_array().unwrap();
        assert!(ifaces
            .iter()
            .any(|i| i["name"] == "lan" && i["selected"] == false));
        assert!(ifaces
            .iter()
            .any(|i| i["name"] == "wan" && i["selected"] == true));
        assert!(doc["services"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "dnsmasq"));
        assert!(!doc.to_string().contains("password\":\""));
    }

    #[test]
    fn preview_returns_discovery() {
        let v = preview(&fixture("primary"), &json!({ "key": "wan_wan_status" }));
        assert_eq!(v["payload"]["payload_on"], "up");
        assert_eq!(
            v["config_topic"],
            "homeassistant/binary_sensor/openwrt_router-primary_wan_wan_status/config"
        );
    }

    #[test]
    fn method_list_is_an_object() {
        assert!(methods().as_object().unwrap().contains_key("catalog"));
    }
}
