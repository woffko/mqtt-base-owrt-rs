// SPDX-License-Identifier: GPL-2.0-or-later
//! Compare one full publish pass on the primary router fixture with the
//! retained topics captured from the production broker
//! (`tests/fixtures/primary/retained_topics.txt`, taken before this daemon
//! published there). Home Assistant keeps the history of an entity only while
//! its unique_id, topics and name stay the same, so the default output has to
//! keep reproducing them.
//!
//! Allowed differences (see docs/TOPICS.md):
//! - entities that can never have a value are not announced, and removed
//!   (empty retained payload) when this router owns them;
//! - discovery payloads gain fields (availability, origin, state class);
//! - `system/uptime` differs because the captures were taken moments apart.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mqtt_metrics::config::Config;
use mqtt_metrics::engine::Engine;
use mqtt_metrics::source::{FixtureSource, Source};
use mqtt_metrics::state::TopicState;
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures")
}

fn reference(host: &str) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(fixtures().join("primary/retained_topics.txt")).unwrap();
    text.lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(t, _)| t.contains(host))
        .map(|(t, p)| (t.to_string(), p.to_string()))
        .collect()
}

/// Entities in the capture that are removed because their interface's
/// protocol can never provide the value.
fn documented_removal(topic: &str) -> bool {
    [
        "wan_wan_ipv6",
        "wan_wan_gateway6",
        "wan_wanb_ipv6",
        "wan_wanb_gateway6",
        "wan_wan_6_ipv4",
        "wan_wan_6_gateway4",
        "wan_wanb_6_ipv4",
        "wan_wanb_6_gateway4",
        "wan-wan/ipv6",
    ]
    .iter()
    .any(|needle| topic.contains(needle))
}

#[test]
fn default_output_keeps_the_captured_topics() {
    let host = "router-primary";
    let old = reference(host);
    assert!(old.len() > 40, "fixture has {} topics", old.len());

    // The router owns every captured topic, as the production routers did.
    let owned = TopicState {
        topics: old.keys().cloned().collect(),
    };
    let source = FixtureSource::new(&fixtures().join("primary"));
    let mut engine = Engine::new(Config::default(), host, owned);
    let out = engine.full_pass(&source, source.now(), 0.0);
    let published: BTreeMap<&str, &str> = out
        .iter()
        .filter(|p| !p.payload.is_empty())
        .map(|p| (p.topic.as_str(), p.payload.as_str()))
        .collect();
    let cleared: Vec<&str> = out
        .iter()
        .filter(|p| p.payload.is_empty())
        .map(|p| p.topic.as_str())
        .collect();

    for (topic, old_payload) in &old {
        match published.get(topic.as_str()) {
            Some(new_payload) if topic.ends_with("/config") => {
                let old_json: Value = serde_json::from_str(old_payload).unwrap();
                let new_json: Value = serde_json::from_str(new_payload).unwrap();
                for (key, value) in old_json.as_object().unwrap() {
                    assert_eq!(&new_json[key], value, "{topic}: field {key}");
                }
            }
            Some(new_payload) => {
                if !topic.ends_with("/system/uptime") {
                    assert_eq!(new_payload, old_payload, "{topic}");
                }
            }
            None => {
                assert!(
                    documented_removal(topic),
                    "{topic} is neither published nor a documented removal"
                );
                assert!(
                    cleared.contains(&topic.as_str()),
                    "{topic} must be cleared on the broker"
                );
            }
        }
    }
    for topic in published.keys() {
        assert!(
            old.contains_key(*topic),
            "{topic} is published but is not in the captured set"
        );
    }
}
