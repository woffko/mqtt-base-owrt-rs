// SPDX-License-Identifier: GPL-2.0-or-later
//! Per-interface metrics. The six legacy values follow `network.sh`
//! (`network_is_up`, `network_get_device`, `network_get_ipaddr`,
//! `network_get_ipaddrs6`, `network_get_gateway[6]`) with two fixes:
//! link-local IPv6 addresses are dropped, and values that the interface's
//! protocol can never have are marked permanent so they are not announced.

use serde_json::Value as Json;

use super::net::{self, NetView};
use super::{reason, Context, GroupResult, Reading, RoundCache, Value};
use crate::catalog::*;

pub fn collect(ctx: &Context, cache: &mut RoundCache, out: &mut GroupResult) {
    let view = match cache.net(ctx) {
        Ok(v) => v,
        Err(e) => {
            out.miss_group(Group::Interface, &reason(e));
            return;
        }
    };
    if view.selected.is_empty() {
        out.miss_group(Group::Interface, "no interfaces are selected");
        return;
    }
    for (name, family) in &view.selected {
        interface(view, name, *family, out);
    }
}

fn interface(view: &NetView, name: &str, family: Family, out: &mut GroupResult) {
    let inst = || Instance::iface(family, name);
    let (no_v4, no_v6) = net::impossible_families(view, name);
    let proto = view.proto(name).unwrap_or_default();
    let Some(dump) = view.dump.get(name) else {
        // Dynamic interfaces (for example `wan_6` while its parent is down)
        // are absent from netifd; their protocol still comes from UCI.
        out.push(&IFACE_STATUS, inst(), Reading::Value(Value::Bool(false)));
        for (def, impossible, family) in [
            (&IFACE_IPV4, no_v4, "IPv4"),
            (&IFACE_GW4, no_v4, "IPv4"),
            (&IFACE_IPV6, no_v6, "global IPv6"),
            (&IFACE_GW6, no_v6, "global IPv6"),
        ] {
            let reading = if impossible {
                Reading::impossible(format!("{proto} interfaces have no {family}"))
            } else {
                Reading::missing("interface is not known to netifd")
            };
            out.push(def, inst(), reading);
        }
        for def in [
            &IFACE_DEVICE,
            &IFACE_PROTO,
            &IFACE_UPTIME,
            &IFACE_DNS,
            &IFACE_PREFIX,
        ] {
            out.push(
                def,
                inst(),
                Reading::missing("interface is not known to netifd"),
            );
        }
        return;
    };
    let up = net::is_up(dump);
    out.push(&IFACE_STATUS, inst(), Reading::Value(Value::Bool(up)));
    out.push(
        &IFACE_DEVICE,
        inst(),
        Reading::non_empty(
            net::l3_device(dump).unwrap_or_default(),
            "no layer-3 device",
        ),
    );

    let v4 = |value: Option<String>, what: &str| {
        if no_v4 {
            Reading::impossible(format!(
                "{} interfaces have no IPv4",
                view.proto(name).unwrap_or_default()
            ))
        } else {
            Reading::non_empty(value.unwrap_or_default(), what)
        }
    };
    let v6 = |value: Option<String>, what: &str| {
        if no_v6 {
            Reading::impossible(format!(
                "{} interfaces carry no global IPv6",
                view.proto(name).unwrap_or_default()
            ))
        } else {
            Reading::non_empty(value.unwrap_or_default(), what)
        }
    };
    out.push(
        &IFACE_IPV4,
        inst(),
        v4(net::first_ipv4(dump), "no IPv4 address"),
    );
    let addrs = net::ipv6_addresses(dump);
    out.push(
        &IFACE_IPV6,
        inst(),
        v6(
            (!addrs.is_empty()).then(|| addrs.join(",")),
            "no global IPv6 address",
        ),
    );
    out.push(
        &IFACE_GW4,
        inst(),
        v4(net::gateway(dump, "0.0.0.0"), "no IPv4 default route"),
    );
    out.push(
        &IFACE_GW6,
        inst(),
        v6(net::gateway(dump, "::"), "no IPv6 default route"),
    );

    out.push(
        &IFACE_PROTO,
        inst(),
        Reading::non_empty(view.proto(name).unwrap_or_default(), "no protocol"),
    );
    let uptime = dump.get("uptime").and_then(Json::as_i64).filter(|_| up);
    out.push(
        &IFACE_UPTIME,
        inst(),
        match uptime {
            Some(u) => Reading::Value(Value::Int(u)),
            None => Reading::missing("interface is down"),
        },
    );
    out.push(
        &IFACE_DNS,
        inst(),
        Reading::non_empty(
            net::string_list(dump, "dns-server").join(","),
            "no DNS servers",
        ),
    );
    out.push(
        &IFACE_PREFIX,
        inst(),
        Reading::non_empty(net::prefixes(dump).join(","), "no delegated IPv6 prefix"),
    );
}

#[cfg(test)]
mod tests {
    use crate::catalog::{Group, Kind};
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::{CollectorState, Reading};
    use crate::config::Config;

    fn payload(r: Option<Reading>) -> Option<String> {
        match r {
            Some(Reading::Value(v)) => Some(v.payload(Kind::Binary {
                on: "up",
                off: "down",
            })),
            _ => None,
        }
    }

    #[test]
    fn wan_values_match_legacy_output() {
        let src = fixture("primary");
        let r = run(
            Group::Interface,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            payload(value_of(&r, "iface.status", "wan", "")).as_deref(),
            Some("up")
        );
        assert_eq!(
            payload(value_of(&r, "iface.device", "wan", "")).as_deref(),
            Some("pppoe-wan")
        );
        assert_eq!(
            payload(value_of(&r, "iface.device", "wanb_6", "")).as_deref(),
            Some("eth0")
        );
        assert!(payload(value_of(&r, "iface.ipv4", "wanb", "")).is_some());
        assert!(payload(value_of(&r, "iface.gateway4", "wanb", "")).is_some());
        let v6 = payload(value_of(&r, "iface.ipv6", "wan_6", "")).unwrap();
        assert_eq!(v6.split(',').count(), 2);
        assert!(v6.starts_with("2001:db8:"));
        // Fixed legacy defects: no link-local address on PPPoE, and
        // impossible fields are permanent.
        assert!(matches!(
            value_of(&r, "iface.ipv6", "wan", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
        assert!(matches!(
            value_of(&r, "iface.ipv4", "wan_6", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
        assert!(matches!(
            value_of(&r, "iface.gateway6", "wanb", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
    }

    #[test]
    fn secondary_down_wan() {
        let src = fixture("secondary");
        let r = run(
            Group::Interface,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            payload(value_of(&r, "iface.status", "wan", "")).as_deref(),
            Some("down")
        );
        assert!(matches!(
            value_of(&r, "iface.device", "wan", ""),
            Some(Reading::Unavailable {
                permanent: false,
                ..
            })
        ));
        // `wan_6` is absent from netifd while `wan` is down: its protocol
        // comes from UCI and IPv4 stays impossible.
        assert!(!r.samples.is_empty());
        assert!(matches!(
            value_of(&r, "iface.ipv4", "wan_6", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
        assert!(matches!(
            value_of(&r, "iface.ipv6", "wan_6", ""),
            Some(Reading::Unavailable {
                permanent: false,
                ..
            })
        ));
    }
}
