// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime status written by the daemon to `/var/run/mqtt-metrics/status.json`
//! (tmpfs) and read by the rpcd plugin. It never contains the password.

use serde_json::{json, Value};

use crate::engine::Engine;

#[derive(Debug, Clone, Default)]
pub struct Connection {
    /// `disabled`, `unconfigured`, `config_error`, `connecting`,
    /// `connected` or `disconnected`.
    pub state: &'static str,
    pub since: u64,
    pub broker: String,
    pub last_error: Option<String>,
    pub last_connect: Option<u64>,
    pub reconnects: u64,
    pub messages: u64,
    pub publish_errors: u64,
    pub last_publish: Option<u64>,
    pub next_attempt: Option<u64>,
    /// The broker refused the subscription to the Home Assistant birth topic.
    pub birth_denied: bool,
}

pub fn render(
    engine: Option<&Engine>,
    conn: &Connection,
    config_errors: &[String],
    now: u64,
) -> String {
    let mut doc = json!({
        "version": crate::VERSION,
        "pid": std::process::id(),
        "updated": now,
        "state": conn.state,
        "since": conn.since,
        "broker": conn.broker,
        "last_error": conn.last_error,
        "last_connect": conn.last_connect,
        "reconnects": conn.reconnects,
        "messages": conn.messages,
        "publish_errors": conn.publish_errors,
        "last_publish": conn.last_publish,
        "next_attempt": conn.next_attempt,
        "birth_denied": conn.birth_denied,
        "config_errors": config_errors,
    });
    if let Some(engine) = engine {
        let id = engine.identity();
        doc["device"] = json!({
            "id": id.device_id,
            "name": id.device_name,
            "hostname": id.hostname,
            "base_topic": id.base,
            "availability_topic": id.availability_topic,
            "client_id": id.client_id,
            "discovery": engine.config().identity.discovery,
        });
        let metrics: Vec<Value> = engine
            .statuses()
            .into_iter()
            .filter(|s| s.entry.publish)
            .map(|s| {
                json!({
                    "key": s.entry.key,
                    "group": s.entry.def.group.id(),
                    "name": s.entry.name,
                    "topic": s.entry.state_topic,
                    "value": s.last_payload,
                    "unit": s.entry.def.unit,
                    "published_at": s.published_at,
                    "discovery": s.entry.announce(engine.config()),
                })
            })
            .collect();
        doc["metrics"] = Value::Array(metrics);
        doc["owned_topics"] = json!(engine.topics().topics.len());
    }
    doc.to_string()
}
