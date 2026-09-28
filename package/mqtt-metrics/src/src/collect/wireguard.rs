// SPDX-License-Identifier: GPL-2.0-or-later
//! WireGuard interfaces and peers from `luci.wireguard getWgInstances`.
//! Only names, handshake times and byte counters are used; keys, endpoints
//! and allowed IPs are never published.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

/// A peer counts as connected when its last handshake is younger than this.
/// WireGuard re-keys every 120 s while traffic flows.
const CONNECTED_WINDOW_S: u64 = 180;

fn number(v: Option<&Json>) -> Option<u64> {
    match v? {
        Json::String(s) => s.parse().ok(),
        Json::Number(n) => n.as_u64(),
        _ => None,
    }
}

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let instances = match ctx.source.ubus("luci.wireguard", "getWgInstances", None) {
        Ok(i) => i,
        Err(SourceError::Missing(_)) => {
            out.miss_group(Group::WireGuard, "luci-proto-wireguard is not installed");
            return;
        }
        Err(e) => {
            out.miss_group(Group::WireGuard, &e.to_string());
            return;
        }
    };
    let Some(ifaces) = instances.as_object().filter(|m| !m.is_empty()) else {
        out.miss_group(Group::WireGuard, "no WireGuard interfaces");
        return;
    };
    for (iface, body) in ifaces {
        let peers = body
            .get("peers")
            .and_then(Json::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        out.push(
            &WG_PEERS,
            Instance::named(iface),
            Reading::Value(Value::Int(i64::try_from(peers.len()).unwrap_or(i64::MAX))),
        );
        for (index, peer) in peers.iter().enumerate() {
            let name = peer
                .get("name")
                .and_then(Json::as_str)
                .filter(|n| !n.trim().is_empty())
                .map_or_else(|| format!("peer{}", index + 1), str::to_string);
            let inst = || Instance::named(iface).with_sub(&name);
            let handshake = number(peer.get("latest_handshake")).filter(|t| *t > 0);
            match handshake {
                Some(ts) => {
                    let age = ctx.now.saturating_sub(ts);
                    out.push(
                        &WG_PEER_HANDSHAKE,
                        inst(),
                        Reading::Value(Value::Int(i64::try_from(age).unwrap_or(i64::MAX))),
                    );
                    out.push(
                        &WG_PEER_CONNECTED,
                        inst(),
                        Reading::Value(Value::Bool(age < CONNECTED_WINDOW_S)),
                    );
                }
                None => {
                    out.push(
                        &WG_PEER_HANDSHAKE,
                        inst(),
                        Reading::missing("no handshake yet"),
                    );
                    out.push(
                        &WG_PEER_CONNECTED,
                        inst(),
                        Reading::Value(Value::Bool(false)),
                    );
                }
            }
            for (def, key) in [(&WG_PEER_RX, "transfer_rx"), (&WG_PEER_TX, "transfer_tx")] {
                let reading = match number(peer.get(key)) {
                    Some(v) => Reading::Value(Value::Int(i64::try_from(v).unwrap_or(i64::MAX))),
                    None => Reading::missing("no counter"),
                };
                out.push(def, inst(), reading);
            }
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
    fn peers_and_handshakes() {
        let src = fixture("primary").with_now(1_790_619_900);
        let r = run(
            Group::WireGuard,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert_eq!(
            value_of(&r, "wireguard.peers", "wireguard", ""),
            Some(Reading::Value(Value::Int(1)))
        );
        assert_eq!(
            value_of(&r, "wireguard.peer_handshake_age", "wireguard", "peer-1"),
            Some(Reading::Value(Value::Int(77)))
        );
        assert_eq!(
            value_of(&r, "wireguard.peer_connected", "wireguard", "peer-1"),
            Some(Reading::Value(Value::Bool(true)))
        );
    }
}
