// SPDX-License-Identifier: GPL-2.0-or-later
//! Per-device traffic counters and rates from `network.device status`.

use serde_json::Value as Json;

use super::{CollectorState, Context, Counters, GroupResult, Reading, RoundCache, Value};
use crate::catalog::*;

pub fn collect(
    ctx: &Context,
    cache: &mut RoundCache,
    state: &mut CollectorState,
    out: &mut GroupResult,
) {
    let devices = if ctx.cfg.select.devices.is_empty() {
        match cache.net(ctx) {
            Ok(view) => view.selected_devices(),
            Err(e) => {
                out.miss_group_err(Group::Traffic, e);
                return;
            }
        }
    } else {
        ctx.cfg.select.devices.clone()
    };
    if devices.is_empty() {
        out.miss_group(Group::Traffic, "no devices are selected");
        return;
    }
    let status = match ctx.source.ubus("network.device", "status", None) {
        Ok(s) => s,
        Err(e) => {
            out.miss_group_err(Group::Traffic, &e);
            return;
        }
    };
    for dev in &devices {
        device(ctx, state, dev, status.get(dev.as_str()), out);
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Rate in Mbit/s between two counter readings, `None` on a reset or when no
/// time has passed.
pub fn rate_mbit(prev: u64, cur: u64, seconds: f64) -> Option<f64> {
    let delta = cur.checked_sub(prev)?;
    if seconds <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(delta as f64 * 8.0 / seconds / 1_000_000.0)
}

fn device(
    ctx: &Context,
    state: &mut CollectorState,
    dev: &str,
    status: Option<&Json>,
    out: &mut GroupResult,
) {
    let inst = || Instance::named(dev);
    let all = [
        &DEV_RX_BYTES,
        &DEV_TX_BYTES,
        &DEV_RX_RATE,
        &DEV_TX_RATE,
        &DEV_RX_ERRORS,
        &DEV_TX_ERRORS,
        &DEV_RX_DROPPED,
        &DEV_TX_DROPPED,
        &DEV_SPEED,
        &DEV_CARRIER,
    ];
    let Some(status) = status else {
        for def in all {
            out.push(def, inst(), Reading::missing("device does not exist"));
        }
        state.traffic.remove(dev);
        return;
    };
    let stat = |key: &str| {
        status
            .get("statistics")
            .and_then(|s| s.get(key))
            .and_then(Json::as_u64)
    };
    for (def, key) in [
        (&DEV_RX_BYTES, "rx_bytes"),
        (&DEV_TX_BYTES, "tx_bytes"),
        (&DEV_RX_ERRORS, "rx_errors"),
        (&DEV_TX_ERRORS, "tx_errors"),
        (&DEV_RX_DROPPED, "rx_dropped"),
        (&DEV_TX_DROPPED, "tx_dropped"),
    ] {
        let reading = match stat(key) {
            Some(v) => Reading::Value(Value::Int(to_i64(v))),
            None => Reading::missing("no counter"),
        };
        out.push(def, inst(), reading);
    }

    match (stat("rx_bytes"), stat("tx_bytes")) {
        (Some(rx), Some(tx)) => {
            let cur = Counters {
                rx,
                tx,
                at: ctx.mono,
            };
            let prev = state.traffic.insert(dev.to_string(), cur);
            let rates = prev.map(|p| {
                (
                    rate_mbit(p.rx, rx, ctx.mono - p.at),
                    rate_mbit(p.tx, tx, ctx.mono - p.at),
                )
            });
            for (def, value) in [
                (&DEV_RX_RATE, rates.and_then(|r| r.0)),
                (&DEV_TX_RATE, rates.and_then(|r| r.1)),
            ] {
                let reading = match value {
                    Some(v) => Reading::Value(Value::Float(v, 3)),
                    None => Reading::missing("waiting for a second sample"),
                };
                out.push(def, inst(), reading);
            }
        }
        _ => {
            out.push(&DEV_RX_RATE, inst(), Reading::missing("no counter"));
            out.push(&DEV_TX_RATE, inst(), Reading::missing("no counter"));
        }
    }

    let speed = status
        .get("speed")
        .and_then(Json::as_str)
        .and_then(|s| s.trim_end_matches(['F', 'H']).parse::<i64>().ok())
        .filter(|s| *s > 0);
    out.push(
        &DEV_SPEED,
        inst(),
        match speed {
            Some(s) => Reading::Value(Value::Int(s)),
            // Ethernet, bridge, VLAN and bond devices (netifd reports a
            // `devtype`) have a speed again when the link comes up; PPP and
            // tunnel devices never do.
            None if status.get("devtype").is_some() => {
                Reading::missing("the link is down or reports no speed")
            }
            None => Reading::impossible("the device has no link speed"),
        },
    );
    out.push(
        &DEV_CARRIER,
        inst(),
        match status.get("carrier").and_then(Json::as_bool) {
            Some(c) => Reading::Value(Value::Bool(c)),
            None => Reading::missing("no carrier information"),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::config::Config;

    #[test]
    fn counters_and_rates() {
        let src = fixture("primary");
        let cfg = Config::default();
        let mut state = CollectorState::default();
        let first = run(Group::Traffic, &src, &cfg, &mut state, 100.0);
        assert!(matches!(
            value_of(&first, "traffic.rx_rate", "eth0", ""),
            Some(Reading::Unavailable { .. })
        ));
        assert!(matches!(
            value_of(&first, "traffic.rx_bytes", "eth0", ""),
            Some(Reading::Value(Value::Int(_)))
        ));
        assert!(matches!(
            value_of(&first, "traffic.speed", "eth0", ""),
            Some(Reading::Value(Value::Int(1000)))
        ));
        assert!(matches!(
            value_of(&first, "traffic.speed", "pppoe-wan", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
        // Same counters 10 s later: rate 0.
        let second = run(Group::Traffic, &src, &cfg, &mut state, 110.0);
        assert_eq!(
            value_of(&second, "traffic.rx_rate", "eth0", ""),
            Some(Reading::Value(Value::Float(0.0, 3)))
        );
    }

    #[test]
    fn rate_math() {
        assert_eq!(rate_mbit(0, 1_250_000, 1.0), Some(10.0));
        assert_eq!(rate_mbit(10, 5, 1.0), None);
        assert_eq!(rate_mbit(0, 5, 0.0), None);
    }

    // ---- audit regression tests ------------------------------------------

    #[test]
    fn a_missing_link_speed_is_permanent_only_for_devices_without_one() {
        let s = crate::collect::tests::Scratch::new("primary", "speed");
        s.edit_json("network_device_status.json", |doc| {
            doc["eth0"].as_object_mut().unwrap().remove("speed");
        });
        let mut cfg = Config::default();
        cfg.select.devices = vec!["eth0".into(), "pppoe-wan".into()];
        let r = run(
            Group::Traffic,
            &s.source,
            &cfg,
            &mut CollectorState::default(),
            0.0,
        );
        // An Ethernet port whose link is down will have a speed again.
        assert!(matches!(
            value_of(&r, "traffic.speed", "eth0", ""),
            Some(Reading::Unavailable {
                permanent: false,
                ..
            })
        ));
        // A PPP interface never has one.
        assert!(matches!(
            value_of(&r, "traffic.speed", "pppoe-wan", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
    }
}
