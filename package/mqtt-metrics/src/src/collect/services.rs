// SPDX-License-Identifier: GPL-2.0-or-later
//! procd service state for the services selected in LuCI.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;

/// Names of procd services that have at least one instance, for the LuCI
/// picker.
pub fn available(list: &Json) -> Vec<String> {
    list.as_object()
        .map(|m| {
            m.iter()
                .filter(|(_, v)| {
                    v.get("instances")
                        .and_then(Json::as_object)
                        .is_some_and(|i| !i.is_empty())
                })
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    if ctx.cfg.select.services.is_empty() {
        out.miss(&SERVICE_RUNNING, "no services are selected");
        return;
    }
    let list = match ctx.source.ubus("service", "list", None) {
        Ok(l) => l,
        Err(e) => {
            out.miss(&SERVICE_RUNNING, e.to_string());
            return;
        }
    };
    for name in &ctx.cfg.select.services {
        let reading = match list.get(name.as_str()) {
            None => Reading::missing("service is not registered with procd"),
            Some(svc) => {
                let instances = svc.get("instances").and_then(Json::as_object);
                match instances {
                    Some(i) if !i.is_empty() => {
                        Reading::Value(Value::Bool(i.values().all(|inst| {
                            inst.get("running").and_then(Json::as_bool).unwrap_or(false)
                        })))
                    }
                    _ => Reading::Value(Value::Bool(false)),
                }
            }
        };
        out.push(&SERVICE_RUNNING, Instance::named(name), reading);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn selected_services() {
        let mut cfg = Config::default();
        cfg.select.services = vec!["dnsmasq".into(), "adblock".into(), "nonexistent".into()];
        let r = run(
            Group::Service,
            &fixture("primary"),
            &cfg,
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            value_of(&r, "service.running", "dnsmasq", ""),
            Some(Reading::Value(Value::Bool(true)))
        );
        assert_eq!(
            value_of(&r, "service.running", "adblock", ""),
            Some(Reading::Value(Value::Bool(false)))
        );
        assert!(matches!(
            value_of(&r, "service.running", "nonexistent", ""),
            Some(Reading::Unavailable { .. })
        ));
    }
}
