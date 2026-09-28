// SPDX-License-Identifier: GPL-2.0-or-later
//! Device identity and Home Assistant MQTT discovery payloads.

use serde_json::{json, Map, Value};

use crate::catalog::{Kind, MetricDef};
use crate::config::Config;
use crate::text::{sanitize_id, sanitize_topic_level};

/// Identity resolved from configuration and the runtime hostname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub hostname: String,
    /// `<base_topic>/<node>`.
    pub base: String,
    pub device_id: String,
    pub device_name: String,
    pub manufacturer: String,
    pub model: String,
    pub configuration_url: Option<String>,
    pub discovery_prefix: String,
    pub availability_topic: String,
    pub client_id: String,
}

impl Identity {
    pub fn resolve(cfg: &Config, hostname: &str) -> Identity {
        let node = cfg
            .identity
            .node
            .clone()
            .unwrap_or_else(|| sanitize_topic_level(hostname));
        let base = format!("{}/{}", cfg.identity.base_topic, node);
        let device_id = cfg
            .identity
            .device_id
            .clone()
            .unwrap_or_else(|| format!("openwrt_{}", sanitize_id(hostname)));
        Identity {
            hostname: hostname.to_string(),
            availability_topic: format!("{base}/availability"),
            base,
            device_name: cfg
                .identity
                .device_name
                .clone()
                .unwrap_or_else(|| format!("OpenWrt {hostname}")),
            manufacturer: cfg.identity.manufacturer.clone(),
            model: cfg.identity.model.clone(),
            configuration_url: cfg.identity.configuration_url.clone(),
            discovery_prefix: cfg.identity.discovery_prefix.clone(),
            client_id: cfg
                .broker
                .client_id
                .clone()
                .unwrap_or_else(|| format!("mqtt-metrics-{}", sanitize_id(hostname))),
            device_id,
        }
    }

    pub fn config_topic(&self, kind: Kind, unique_id: &str) -> String {
        format!(
            "{}/{}/{unique_id}/config",
            self.discovery_prefix,
            kind.component()
        )
    }
}

/// What the discovery payload needs to know about one entity.
pub struct EntityView<'a> {
    pub def: &'a MetricDef,
    pub unique_id: &'a str,
    pub name: &'a str,
    pub state_topic: &'a str,
    pub icon: Option<&'a str>,
    pub diagnostic: bool,
    pub ha_enabled: bool,
}

/// Discovery payload. It contains every field the legacy script sent (with
/// the same values for legacy entities) plus availability, state class,
/// entity category and origin.
pub fn discovery_payload(e: &EntityView, id: &Identity) -> String {
    let mut m = Map::new();
    m.insert("name".into(), json!(e.name));
    m.insert("unique_id".into(), json!(e.unique_id));
    m.insert("state_topic".into(), json!(e.state_topic));
    if let Kind::Binary { on, off } = e.def.kind {
        m.insert("payload_on".into(), json!(on));
        m.insert("payload_off".into(), json!(off));
    }
    let mut device = Map::new();
    device.insert("identifiers".into(), json!([id.device_id]));
    device.insert("name".into(), json!(id.device_name));
    device.insert("manufacturer".into(), json!(id.manufacturer));
    device.insert("model".into(), json!(id.model));
    if let Some(url) = &id.configuration_url {
        device.insert("configuration_url".into(), json!(url));
    }
    m.insert("device".into(), Value::Object(device));
    if let Some(icon) = e.icon {
        m.insert("icon".into(), json!(icon));
    }
    if let Some(class) = e.def.device_class {
        m.insert("device_class".into(), json!(class));
    }
    if let Some(unit) = e.def.unit {
        m.insert("unit_of_measurement".into(), json!(unit));
    }
    if let Some(sc) = e.def.state_class {
        m.insert("state_class".into(), json!(sc));
    }
    if e.diagnostic {
        m.insert("entity_category".into(), json!("diagnostic"));
    }
    if !e.ha_enabled {
        m.insert("enabled_by_default".into(), json!(false));
    }
    m.insert("availability_topic".into(), json!(id.availability_topic));
    m.insert(
        "origin".into(),
        json!({ "name": "mqtt-metrics", "sw_version": crate::VERSION }),
    );
    Value::Object(m).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{IFACE_STATUS, SYSTEM_UPTIME};

    #[test]
    fn default_identity_matches_legacy() {
        let id = Identity::resolve(&Config::default(), "router-primary");
        assert_eq!(id.base, "openwrt/router-primary");
        assert_eq!(id.device_id, "openwrt_router-primary");
        assert_eq!(id.device_name, "OpenWrt router-primary");
        assert_eq!(id.availability_topic, "openwrt/router-primary/availability");
        assert_eq!(
            id.config_topic(IFACE_STATUS.kind, "openwrt_router-primary_wan_wan_status"),
            "homeassistant/binary_sensor/openwrt_router-primary_wan_wan_status/config"
        );
    }

    #[test]
    fn payload_contains_legacy_fields() {
        let id = Identity::resolve(&Config::default(), "r1");
        let payload = discovery_payload(
            &EntityView {
                def: &SYSTEM_UPTIME,
                unique_id: "openwrt_r1_system_uptime",
                name: "Uptime",
                state_topic: "openwrt/r1/system/uptime",
                icon: SYSTEM_UPTIME.icon,
                diagnostic: false,
                ha_enabled: true,
            },
            &id,
        );
        let v: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["name"], "Uptime");
        assert_eq!(v["device_class"], "duration");
        assert_eq!(v["unit_of_measurement"], "s");
        assert_eq!(v["icon"], "mdi:clock-outline");
        assert_eq!(v["device"]["model"], "Router");
        assert_eq!(v["availability_topic"], "openwrt/r1/availability");
        assert!(v.get("enabled_by_default").is_none());
    }
}
