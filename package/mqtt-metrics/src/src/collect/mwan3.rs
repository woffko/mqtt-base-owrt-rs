// SPDX-License-Identifier: GPL-2.0-or-later
//! Multi-WAN state from `ubus call mwan3 status`.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

const UNREACHABLE_LATENCY: i64 = 999_999;

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let status = match ctx.source.ubus("mwan3", "status", None) {
        Ok(s) => s,
        Err(SourceError::Missing(_)) => {
            out.miss_group(Group::Mwan3, "mwan3 is not installed");
            return;
        }
        Err(e) => {
            out.miss_group(Group::Mwan3, &e.to_string());
            return;
        }
    };
    let Some(ifaces) = status.get("interfaces").and_then(Json::as_object) else {
        out.miss_group(Group::Mwan3, "mwan3 reports no interfaces");
        return;
    };
    if ifaces.is_empty() {
        out.miss_group(Group::Mwan3, "mwan3 reports no interfaces");
        return;
    }
    let int = |v: &Json, key: &str| v.get(key).and_then(Json::as_i64);
    for (name, iface) in ifaces {
        let inst = || Instance::named(name);
        let status = iface.get("status").and_then(Json::as_str).unwrap_or("");
        out.push(
            &MWAN3_STATUS,
            inst(),
            Reading::non_empty(status.to_string(), "no status"),
        );
        out.push(
            &MWAN3_ONLINE,
            inst(),
            if status.is_empty() {
                Reading::missing("no status")
            } else {
                Reading::Value(Value::Bool(status == "online"))
            },
        );
        for (def, key) in [
            (&MWAN3_SCORE, "score"),
            (&MWAN3_ONLINE_TIME, "online"),
            (&MWAN3_LOST, "lost"),
        ] {
            out.push(
                def,
                inst(),
                match int(iface, key) {
                    Some(v) => Reading::Value(Value::Int(v)),
                    None => Reading::missing(format!("no {key} value")),
                },
            );
        }
        for track in iface
            .get("track_ip")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let Some(ip) = track.get("ip").and_then(Json::as_str) else {
                continue;
            };
            let tinst = || Instance::named(name).with_sub(ip);
            for (def, key) in [
                (&MWAN3_TRACK_LATENCY, "latency"),
                (&MWAN3_TRACK_LOSS, "packetloss"),
            ] {
                let reading = match int(track, key) {
                    // mwan3 reports 999999 ms when a target does not answer.
                    Some(v) if key == "latency" && v >= UNREACHABLE_LATENCY => {
                        Reading::missing("target does not answer")
                    }
                    Some(v) => Reading::Value(Value::Int(v)),
                    None => Reading::missing(format!("no {key} value")),
                };
                out.push(def, tinst(), reading);
            }
            let st = track
                .get("status")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            out.push(
                &MWAN3_TRACK_STATUS,
                tinst(),
                Reading::non_empty(st, "no status"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn parses_interfaces_and_tracks() {
        let src = fixture("primary");
        let r = run(
            Group::Mwan3,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            value_of(&r, "mwan3.status", "wanb", ""),
            Some(Reading::text("online"))
        );
        assert_eq!(
            value_of(&r, "mwan3.online", "wanb", ""),
            Some(Reading::Value(Value::Bool(true)))
        );
        let tracks: Vec<_> = r
            .samples
            .iter()
            .filter(|s| s.def.id == "mwan3.track_latency" && s.inst.name == "wan")
            .collect();
        assert_eq!(tracks.len(), 3);
        assert!(r.missing.is_empty());
    }

    #[test]
    fn unreachable_target_has_no_latency() {
        let doc = serde_json::json!({ "interfaces": { "wan": { "status": "online", "track_ip": [
            { "ip": "192.0.2.1", "status": "down", "latency": 999999, "packetloss": 100 }
        ] } } });
        let dir = std::env::temp_dir().join(format!("mqtt-metrics-mwan3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mwan3_status.json"), doc.to_string()).unwrap();
        let src = crate::source::FixtureSource::new(&dir);
        let r = run(
            Group::Mwan3,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert!(matches!(
            value_of(&r, "mwan3.track_latency", "wan", "192.0.2.1"),
            Some(Reading::Unavailable { .. })
        ));
        assert_eq!(
            value_of(&r, "mwan3.track_loss", "wan", "192.0.2.1"),
            Some(Reading::Value(Value::Int(100)))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
