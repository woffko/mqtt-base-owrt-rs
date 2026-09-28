// SPDX-License-Identifier: GPL-2.0-or-later
//! HA cluster state from `ha-cluster status` and `keepalived dump`.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

fn vrrp_state(code: i64) -> String {
    match code {
        0 => "INIT".to_string(),
        1 => "BACKUP".to_string(),
        2 => "MASTER".to_string(),
        3 => "FAULT".to_string(),
        other => format!("UNKNOWN({other})"),
    }
}

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let cluster = ctx.source.ubus("ha-cluster", "status", None);
    let keepalived = ctx.source.ubus("keepalived", "dump", None);
    if matches!(cluster, Err(SourceError::Missing(_)))
        && matches!(keepalived, Err(SourceError::Missing(_)))
    {
        out.miss_group(
            Group::HaCluster,
            "ha-cluster and keepalived are not installed",
        );
        return;
    }

    match &cluster {
        Ok(c) => {
            let text = |key: &str| c.get(key).and_then(Json::as_str).unwrap_or("").to_string();
            out.push(
                &HA_ROLE,
                Instance::none(),
                Reading::non_empty(text("role"), "no role"),
            );
            out.push(
                &HA_STATE,
                Instance::none(),
                Reading::non_empty(text("state"), "no state"),
            );
            let peers = c
                .get("peers")
                .and_then(Json::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if peers.is_empty() {
                out.miss(&HA_PEER_ONLINE, "no peers are configured");
            }
            for peer in peers {
                let name = peer
                    .get("name")
                    .and_then(Json::as_str)
                    .filter(|n| !n.is_empty())
                    .or_else(|| peer.get("address").and_then(Json::as_str))
                    .unwrap_or("peer");
                let reading = match peer.get("online").and_then(Json::as_bool) {
                    Some(online) => Reading::Value(Value::Bool(online)),
                    None => Reading::missing("no peer state"),
                };
                out.push(&HA_PEER_ONLINE, Instance::named(name), reading);
            }
            match c.get("services").and_then(Json::as_object) {
                Some(services) if !services.is_empty() => {
                    for (name, svc) in services {
                        let reading = match svc.get("running").and_then(Json::as_bool) {
                            Some(r) => Reading::Value(Value::Bool(r)),
                            None => Reading::missing("no service state"),
                        };
                        out.push(&HA_SERVICE, Instance::named(name), reading);
                    }
                }
                _ => out.miss(&HA_SERVICE, "no cluster services are reported"),
            }
            for (def, key) in [
                (&HA_CONFIG_SYNC_AGE, "config_last_sync"),
                (&HA_LEASE_SYNC_AGE, "lease_last_sync"),
            ] {
                let ts = c
                    .get("sync")
                    .and_then(|s| s.get(key))
                    .and_then(Json::as_u64)
                    .filter(|t| *t > 0);
                let reading = match ts {
                    Some(t) => Reading::Value(Value::Int(
                        i64::try_from(ctx.now.saturating_sub(t)).unwrap_or(i64::MAX),
                    )),
                    None => Reading::missing("never synchronized"),
                };
                out.push(def, Instance::none(), reading);
            }
        }
        Err(e) => {
            for def in [
                &HA_ROLE,
                &HA_STATE,
                &HA_PEER_ONLINE,
                &HA_SERVICE,
                &HA_CONFIG_SYNC_AGE,
                &HA_LEASE_SYNC_AGE,
            ] {
                out.miss(def, e.to_string());
            }
        }
    }

    match &keepalived {
        Ok(k) => {
            let instances = k
                .get("status")
                .and_then(Json::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if instances.is_empty() {
                out.miss(&HA_VRRP, "keepalived reports no VRRP instances");
            }
            for item in instances {
                let data = item.get("data");
                let Some(name) = data.and_then(|d| d.get("iname")).and_then(Json::as_str) else {
                    continue;
                };
                let reading = match data.and_then(|d| d.get("state")).and_then(Json::as_i64) {
                    Some(code) => Reading::text(vrrp_state(code)),
                    None => Reading::missing("no VRRP state"),
                };
                out.push(&HA_VRRP, Instance::named(name), reading);
            }
        }
        Err(e) => out.miss(&HA_VRRP, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn master_and_backup_roles() {
        let cfg = Config::default();
        let p = run(
            Group::HaCluster,
            &fixture("primary"),
            &cfg,
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            value_of(&p, "ha.role", "", ""),
            Some(Reading::text("MASTER"))
        );
        assert_eq!(
            value_of(&p, "ha.vrrp_state", "lan", ""),
            Some(Reading::text("MASTER"))
        );
        assert_eq!(
            value_of(&p, "ha.service_running", "keepalived", ""),
            Some(Reading::Value(Value::Bool(true)))
        );
        let s = run(
            Group::HaCluster,
            &fixture("secondary"),
            &cfg,
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            value_of(&s, "ha.role", "", ""),
            Some(Reading::text("BACKUP"))
        );
        assert_eq!(
            value_of(&s, "ha.vrrp_state", "lan", ""),
            Some(Reading::text("BACKUP"))
        );
    }
}
