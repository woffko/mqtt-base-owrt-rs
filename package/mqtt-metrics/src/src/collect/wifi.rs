// SPDX-License-Identifier: GPL-2.0-or-later
//! Wireless interfaces from `iwinfo` over ubus.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let devices = match ctx.source.ubus("iwinfo", "devices", None) {
        Ok(d) => d,
        Err(SourceError::Missing(_)) => {
            out.miss_group(Group::Wifi, "rpcd-mod-iwinfo is not installed");
            return;
        }
        Err(e) => {
            out.miss_group_err(Group::Wifi, &e);
            return;
        }
    };
    let names: Vec<&str> = devices
        .get("devices")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_str).collect())
        .unwrap_or_default();
    if names.is_empty() {
        out.miss_group(Group::Wifi, "no wireless devices");
        return;
    }
    for dev in names {
        let arg = serde_json::json!({ "device": dev }).to_string();
        let inst = || Instance::named(dev);
        match ctx.source.ubus("iwinfo", "info", Some(&arg)) {
            Ok(info) => {
                for (def, key) in [(&WIFI_CHANNEL, "channel"), (&WIFI_TXPOWER, "txpower")] {
                    let reading = match info.get(key).and_then(Json::as_i64) {
                        Some(v) => Reading::Value(Value::Int(v)),
                        None => Reading::missing(format!("no {key}")),
                    };
                    out.push(def, inst(), reading);
                }
            }
            Err(e) => {
                out.push(&WIFI_CHANNEL, inst(), Reading::from_error(&e));
                out.push(&WIFI_TXPOWER, inst(), Reading::from_error(&e));
            }
        }
        let clients = ctx
            .source
            .ubus("iwinfo", "assoclist", Some(&arg))
            .ok()
            .and_then(|a| a.get("results").and_then(Json::as_array).map(Vec::len));
        out.push(
            &WIFI_CLIENTS,
            inst(),
            match clients {
                Some(n) => Reading::Value(Value::Int(i64::try_from(n).unwrap_or(i64::MAX))),
                None => Reading::failed("no association list"),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn no_radios_on_x86_vm() {
        let r = run(
            Group::Wifi,
            &fixture("primary"),
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert!(r.samples.is_empty());
        assert_eq!(r.missing.len(), 3);
        assert!(r
            .missing
            .iter()
            .all(|(_, why)| why == "no wireless devices"));
    }
}
