# Phase 2 evidence: extended metrics (2026-09-28)

Package `mqtt-metrics-0.1.0-r2` on both routers, shadow prefix
`openwrt-shadow`. Discovery tests used an isolated identity (discovery
prefix `homeassistant-mmtest`, device id `mmtest_primary`,
`cleanup_legacy = 0`) so production Home Assistant entities were never
touched; the 30 legacy discovery configs of the primary stayed on the broker
throughout.

## Metric groups (live catalog)

| Group | Primary (MASTER) | Secondary (BACKUP) |
| --- | --- | --- |
| system | 18 of 24 available; no swap, no temperature sensors | 18 of 28; same |
| interface | 27 of 40; impossible families marked | 15 of 40; `wan` down, dynamic `wan_6` absent |
| traffic | 15 of 20; PPPoE has no link speed | 8 of 10 |
| mwan3 | 56 of 56 | 50 of 56; targets of the down WAN do not answer |
| ha | 13 of 13; role MASTER, VRRP MASTER, peer online | 13 of 13; role BACKUP, VRRP BACKUP |
| dhcp | 2 of 2 | 2 of 2 |
| wireguard | 5 of 5 | 4 of 5; peer never connected to this node |
| wifi | missing: no wireless devices | same |
| service | missing until services are selected | same |
| packages | 1 of 1 | 1 of 1 |

A catalog request took about 0.55 s on both routers.

## Behaviour checks on the primary

- Recommended set (89 metrics written as UCI overrides, as LuCI does):
  111 discovery configs announced under the test prefix, states published
  for all groups.
- Publish off (`system_load_1`): discovery config and retained state
  removed. Show in Home Assistant off (`system_memory_used`): config
  removed, state still published.
- Intervals: traffic group at 10 s gave 4 publications in 42 s; a metric
  override of 20 s gave 2.
- Change-only mode with a 10 s system interval: only changing values
  (uptime, CPU, load, memory, conntrack) were republished within 35 s.
- Discovery off removed all 109 test configs; on again re-announced them
  without reconnecting. A non-retained `online` on the test birth topic
  triggered a resend; `offline` was ignored.
- Upgrade r1 → r2: service restarted by the post-upgrade hook, UCI kept
  (new defaults as `.apk-new`), ubus object count unchanged, LuCI `/ubus`
  answering.

## Fault injection on the primary

| Fault | Result |
| --- | --- |
| Broker port changed to a closed one | state `disconnected`, one warning, backoff; reconnected after restoring the port |
| Wrong password via rpcd `set_password` | `broker refused the connection: not authorized`; reconnected after `--import-legacy` |
| TLS against the plain port | `unexpected end of file`, clean fallback after disabling TLS |
| Invalid `publish.interval` | `--check-config` names the field; the daemon goes `offline` until fixed |

The broker has no TLS listener (port 8883 refused), so TLS handshakes are
covered by `tests/mqtt_tls.rs` against an in-process rustls server
(verified CA, unknown issuer, unverified mode).

Bugs found and fixed during this phase: the birth topic was not subscribed
when discovery was enabled by a reload; the status file lagged up to 15 s
after a reload with an invalid configuration.
