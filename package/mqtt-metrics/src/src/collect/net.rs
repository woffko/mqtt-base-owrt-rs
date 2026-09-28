// SPDX-License-Identifier: GPL-2.0-or-later
//! Network view shared by the interface and traffic collectors: UCI
//! interface sections plus the netifd `network.interface dump`.

use std::collections::BTreeMap;
use std::net::Ipv6Addr;

use serde_json::Value;

use crate::catalog::Family;
use crate::config::Config;
use crate::source::{Source, SourceError, SourceResult};
use crate::uci;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UciIface {
    pub name: String,
    pub proto: Option<String>,
    pub device: Option<String>,
    /// Static IPv6 configuration (`ip6addr`, `ip6gw`, `ip6prefix`).
    pub static_v6: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NetView {
    pub uci: Vec<UciIface>,
    pub dump: BTreeMap<String, Value>,
    /// Selected interfaces in publish order.
    pub selected: Vec<(String, Family)>,
}

/// The legacy script's rule: `^(wan([A-Za-z0-9_]+)?|wwan([A-Za-z0-9_]+)?)$`.
pub fn legacy_wan(name: &str) -> bool {
    let rest = name
        .strip_prefix("wwan")
        .or_else(|| name.strip_prefix("wan"));
    rest.is_some_and(|r| r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub fn family(name: &str) -> Family {
    if legacy_wan(name) {
        Family::Wan
    } else {
        Family::Iface
    }
}

pub fn view(source: &dyn Source, cfg: &Config) -> SourceResult<NetView> {
    let text = source.command("uci", &["-q", "show", "network"])?;
    let pkg = uci::parse("network", &text);
    let uci_ifaces: Vec<UciIface> = pkg
        .sections_of_type("interface")
        .map(|s| UciIface {
            name: s.name.clone(),
            proto: s.get("proto").map(str::to_string),
            device: s.get("device").map(str::to_string),
            static_v6: ["ip6addr", "ip6gw", "ip6prefix"]
                .iter()
                .any(|o| s.get(o).is_some()),
        })
        .collect();

    let dump = source.ubus("network.interface", "dump", None)?;
    let list = dump
        .get("interface")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Failed("network.interface dump: no interface list".to_string())
        })?;
    let mut by_name = BTreeMap::new();
    for item in list {
        if let Some(name) = item.get("interface").and_then(Value::as_str) {
            by_name.insert(name.to_string(), item.clone());
        }
    }

    let mut selected: Vec<String> = if cfg.select.interfaces_auto {
        let mut names: Vec<String> = uci_ifaces
            .iter()
            .filter(|i| legacy_wan(&i.name))
            .map(|i| i.name.clone())
            .collect();
        names.sort();
        names
    } else {
        cfg.select.interfaces.clone()
    };
    let mut seen = std::collections::BTreeSet::new();
    selected.retain(|n| seen.insert(n.clone()));

    Ok(NetView {
        selected: selected.iter().map(|n| (n.clone(), family(n))).collect(),
        uci: uci_ifaces,
        dump: by_name,
    })
}

impl NetView {
    pub fn uci_iface(&self, name: &str) -> Option<&UciIface> {
        self.uci.iter().find(|i| i.name == name)
    }

    pub fn proto(&self, name: &str) -> Option<String> {
        self.dump
            .get(name)
            .and_then(|d| d.get("proto"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.uci_iface(name).and_then(|i| i.proto.clone()))
    }

    /// Layer-3 devices of the selected interfaces, without duplicates.
    pub fn selected_devices(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (name, _) in &self.selected {
            if let Some(dev) = self.dump.get(name).and_then(l3_device) {
                if !out.contains(&dev) {
                    out.push(dev);
                }
            }
        }
        out
    }
}

const V6_ONLY: &[&str] = &["dhcpv6", "6in4", "6to4", "6rd", "dslite", "map", "464xlat"];
const V4_ONLY: &[&str] = &["dhcp", "pppoe", "pppoa", "pptp", "l2tp"];

/// Address families an interface can never have, derived from its protocol.
/// Returns `(ipv4_impossible, ipv6_impossible)`.
pub fn impossible_families(view: &NetView, name: &str) -> (bool, bool) {
    let proto = view.proto(name).unwrap_or_default();
    let v4 = V6_ONLY.contains(&proto.as_str());
    let static_v6 = view.uci_iface(name).is_some_and(|i| i.static_v6);
    let v6 = V4_ONLY.contains(&proto.as_str()) || (proto == "static" && !static_v6);
    (v4, v6)
}

pub fn l3_device(iface: &Value) -> Option<String> {
    iface
        .get("l3_device")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn is_up(iface: &Value) -> bool {
    iface.get("up").and_then(Value::as_bool).unwrap_or(false)
}

pub fn first_ipv4(iface: &Value) -> Option<String> {
    iface
        .get("ipv4-address")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|a| a.get("address"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn link_local(addr: &str) -> bool {
    addr.parse::<Ipv6Addr>()
        .map(|a| (a.segments()[0] & 0xffc0) == 0xfe80)
        .unwrap_or(false)
}

/// Global and ULA IPv6 addresses in netifd order: `ipv6-address`, then the
/// local addresses of prefix assignments (as `network_get_ipaddrs6`), without
/// link-local addresses.
pub fn ipv6_addresses(iface: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(list) = iface.get("ipv6-address").and_then(Value::as_array) {
        out.extend(
            list.iter()
                .filter_map(|a| a.get("address").and_then(Value::as_str))
                .map(str::to_string),
        );
    }
    if let Some(list) = iface
        .get("ipv6-prefix-assignment")
        .and_then(Value::as_array)
    {
        out.extend(
            list.iter()
                .filter_map(|a| {
                    a.get("local-address")
                        .and_then(|l| l.get("address"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string),
        );
    }
    out.retain(|a| !link_local(a));
    out
}

/// First active default-route next hop for `target` (`0.0.0.0` or `::`) in
/// the main table, as `network_get_gateway`/`network_get_gateway6`.
pub fn gateway(iface: &Value, target: &str) -> Option<String> {
    iface
        .get("route")
        .and_then(Value::as_array)?
        .iter()
        .find(|r| {
            r.get("target").and_then(Value::as_str) == Some(target) && r.get("table").is_none()
        })
        .and_then(|r| r.get("nexthop"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn string_list(iface: &Value, key: &str) -> Vec<String> {
    iface
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn prefixes(iface: &Value) -> Vec<String> {
    iface
        .get("ipv6-prefix")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let addr = p.get("address").and_then(Value::as_str)?;
                    let mask = p.get("mask").and_then(Value::as_u64)?;
                    Some(format!("{addr}/{mask}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::fixture;

    #[test]
    fn legacy_rule() {
        for name in ["wan", "wan_6", "wanb", "wanb_6", "wwan", "wwan0"] {
            assert!(legacy_wan(name), "{name}");
        }
        for name in ["lan", "wireguard", "wa", "xwan"] {
            assert!(!legacy_wan(name), "{name}");
        }
    }

    #[test]
    fn auto_selection_matches_legacy_script() {
        let v = view(&fixture("primary"), &Config::default()).unwrap();
        let names: Vec<&str> = v.selected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["wan", "wan_6", "wanb", "wanb_6"]);
        assert!(v.selected.iter().all(|(_, f)| *f == Family::Wan));
        assert_eq!(v.selected_devices(), vec!["pppoe-wan", "eth0"]);
    }

    #[test]
    fn impossible_families_by_protocol() {
        let v = view(&fixture("primary"), &Config::default()).unwrap();
        assert_eq!(impossible_families(&v, "wan"), (false, true));
        assert_eq!(impossible_families(&v, "wan_6"), (true, false));
        assert_eq!(impossible_families(&v, "wanb"), (false, true));
        assert_eq!(impossible_families(&v, "lan"), (false, false));
        assert_eq!(impossible_families(&v, "wireguard"), (false, false));
    }

    #[test]
    fn address_helpers() {
        let v = view(&fixture("primary"), &Config::default()).unwrap();
        let wan = &v.dump["wan"];
        assert!(ipv6_addresses(wan).is_empty(), "link-local only");
        assert!(gateway(wan, "0.0.0.0").is_some());
        let wan6 = &v.dump["wan_6"];
        assert_eq!(ipv6_addresses(wan6).len(), 2);
        assert!(gateway(wan6, "::").unwrap().starts_with("fe80::"));
        assert_eq!(first_ipv4(wan6), None);
    }
}
