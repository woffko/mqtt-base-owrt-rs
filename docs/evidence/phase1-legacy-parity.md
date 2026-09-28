# Phase 1 evidence: legacy parity (2026-09-28)

Router: primary of the HA pair (10.0.77.1), OpenWrt 25.12-SNAPSHOT x86_64.
Package: `mqtt-metrics-0.1.0-r1.apk` from the 25.12.5 SDK (TLS build).

## Offline parity (fixtures)

`tests/legacy_parity.rs` runs one full pass on the sanitized primary fixture
and compares it with the retained topics the legacy script left on the
broker: every legacy state is equal (except `system/uptime`), every legacy
discovery field keeps its value, nothing extra is published, and the eight
impossible entities plus the link-local `wan-wan/ipv6` state are cleared.

## Dry run on both routers

`mqtt-metricsd --dump` (nothing is sent) compared with the broker's legacy
topics by `tools/parity-check.py --dump`:

| Router | Legacy topics compared | Differences |
| --- | --- | --- |
| primary | 53 | 0 |
| secondary | 46 | 0 |

Notes reported by the tool: `system/uptime` changed between captures; the
documented removals listed in docs/TOPICS.md.

## Shadow mode on the primary

Configuration: `--import-legacy` (password copied on the router), base topic
`openwrt-shadow`, discovery off. The legacy script kept running.

- Log: configuration loaded, connected; no warnings or errors.
- `tools/parity-check.py --shadow`: 23 legacy state topics compared, 0
  differences (uptime volatile; link-local IPv6 of the PPPoE interface
  removed as documented).
- Availability: `online` while running, `offline` after
  `/etc/init.d/mqtt-metrics stop`, `online` again after start.
- `ubus call`-equivalent `mqtt-metricsd call status`: state `connected`.
- Process: 1 thread, 5 file descriptors, RSS 2.4 MB; the command line is
  `/usr/sbin/mqtt-metricsd` only (no credentials).
- Router checks after install: ubus objects 56 before and after, LuCI
  `/ubus` answers.
