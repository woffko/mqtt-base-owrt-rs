# Phase 3 evidence: LuCI application (2026-09-28)

Packages: `mqtt-metrics-0.1.0-r4`, `luci-app-mqtt-metrics-0.1.0-r5` on both
routers.

## Installation

- The LuCI package adds one ubus object (`mqtt-metrics`, rpcd exec plugin):
  57 objects on the primary, 50 on the secondary; LuCI `/ubus` answers.
- `ubus call mqtt-metrics <method>` works for status, catalog (0.56 s through
  rpcd, including the nested `luci-rpc` call), preview, legacy, log,
  republish, purge and reload.

## Screenshots

`tools/luci-screenshots.browser.js` (read-only: UCI writes and write
methods blocked) took desktop (1440x900) and mobile (390x844) screenshots of
all five pages on both routers, full page and viewport, with no JavaScript
errors after login. The login page of this LuCI build logs a denied
`uci get luci` for the anonymous session; that happens before our pages load
and is excluded. Screenshots are kept outside Git because they show live
addresses:

- primary: `logs/screenshots-20260928T200740/`
- secondary: `logs/screenshots-20260928T200820/`

## Round trip through UCI (primary)

`tools/luci-roundtrip.browser.js` edited settings in the browser and pressed
Save & Apply; `uci show mqtt-metrics` over SSH afterwards showed:

| Page | Edited | UCI after apply |
| --- | --- | --- |
| Broker | keepalive 45, QoS 0, password field left empty | `broker.keepalive='45'`, `broker.qos='0'`, password file checksum unchanged |
| Topics and device | device name, model | `identity.device_name`, `identity.model` set |
| Metrics | global interval 120, change-only, System group interval 30, `system_load_15` unpublished, `system_uptime` name/interval/icon, service `dnsmasq` | all values present; the unpublished recommended metric fell back to its default (section removed) |
| Metrics | Reset to legacy defaults | no `metric` or `group` sections, interval 300, change-only 0, interfaces `auto`, no services |

The daemon reloaded on apply (discovery preview showed the new name, icon,
device name and model).

## Password

All 3.8 MB of responses the browser received during the edit run, plus the
rendered Broker page, were searched on the router with
`grep -F -f /etc/mqtt-metrics/secret`: no match. The catalog reports only
`password_set`.

## UI fixes made in this phase

Readable names for metrics without instances (`Wi-Fi Clients` instead of
`Wi-Fi {n} Clients`), byte values with a human-readable size, wider select
boxes, newest log lines first.
