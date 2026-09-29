# Topics and entities

This is the reference for what `mqtt-metrics` publishes. With the default
configuration the output is compatible with the legacy `publish_metrics.sh`
script: the same state topics, the same Home Assistant `unique_id`s and the
same entity names.

## Topic layout

| Item | Default | Setting (LuCI page) |
| --- | --- | --- |
| State topic | `openwrt/<hostname>/<suffix>` | base topic and node (Topics and device), suffix per metric (Metrics) |
| Availability | `openwrt/<hostname>/availability`, retained `online`/`offline` | follows the base topic |
| Discovery config | `homeassistant/<sensor or binary_sensor>/<unique_id>/config` | discovery prefix (Topics and device) |
| `unique_id` | `openwrt_<hostname>_<object id>` | device ID (Topics and device), object id per metric (Metrics) |
| Device | name `OpenWrt <hostname>`, manufacturer `OpenWrt`, model `Router` | Topics and device |
| Birth topic (only subscription) | `homeassistant/status` | Topics and device |

`<hostname>` is read at runtime from `system.@system[0].hostname`. In ids it
is sanitized like the legacy script did (`tr -c 'A-Za-z0-9_-' '_'`); in
topics only `+`, `#` and `/` are replaced.

States and discovery configs are retained. States use the configured QoS
(default 1); discovery and removals always use QoS 1.

Instance names come from the system (interfaces, devices, services, WireGuard
peer descriptions). In ids they are sanitized to letters, digits, `_` and
`-`; in topics `+`, `#` and `/` are replaced by `_`. When two instances would
end up with the same id (two peers with the same description, `eth0.1` and
`eth0_1`), the later ones get a numeric suffix (`_2`, `_3`) so that each keeps
its own entity.

## Payloads

- Text and numbers are published as plain strings (`123`, `0.42`, `online`).
- Legacy interface status uses `up`/`down`; every other binary sensor uses
  `ON`/`OFF`.
- A value that is not available is never published as a placeholder. What
  happens to the retained state depends on why there is no value:
  - the value is **absent** (an interface is down and has no address, a
    tracking target does not answer, an instance disappeared): a retained
    value published earlier is cleared once with an empty retained message,
    like the legacy script did, so Home Assistant shows it as unknown instead
    of a stale value that looks current;
  - the value **could not be read** (a `ubus` call timed out, an answer was
    garbled): the last published value is kept, because the value did not go
    away. The next successful read replaces it;
  - the value **cannot exist** on this router (see below): it is not
    announced at all.
- Timestamps (`system.boot_time`) are RFC 3339 in UTC.

Discovery configs contain every field the legacy script sent, with the same
values for legacy entities, plus:

- `availability_topic`, so an unreachable router shows as unavailable;
- `state_class` (`measurement` or `total_increasing`) where it applies, which
  enables long-term statistics;
- `entity_category: diagnostic` for diagnostic metrics;
- `enabled_by_default: false` when that is chosen in LuCI;
- `origin` (`mqtt-metrics` and its version);
- `configuration_url` in the device block when it is set.

## When discovery is sent

Discovery is sent after connecting, after a configuration change, and when
Home Assistant publishes a non-retained `online` on the birth topic. It is
not repeated every cycle.

## Hiding and removal

- **Publish off**: the metric is not collected. Its discovery config and
  retained state are cleared with empty retained messages, so Home Assistant
  removes the entity.
- **Show in Home Assistant off**: the state is still published (for other
  consumers such as Node-RED), the discovery config is cleared.
- **Rename** (device ID, prefixes, node, object id or topic suffix): the new
  entities are announced and the old retained topics are cleared. History is
  not carried over by Home Assistant.

The daemon records every retained topic it owns in
`/etc/mqtt-metrics/state.json` and clears the ones that are no longer wanted
on each full pass. The file changes only when that set changes.

## Differences from the legacy script

| Legacy behaviour | mqtt-metrics |
| --- | --- |
| Six discovery configs per WAN interface, even for values the interface can never have | Only values the interface protocol can provide are announced; the rest are removed once (see below) |
| Link-local `fe80::` address published as the IPv6 of a PPPoE interface | Only global and ULA addresses are published |
| Empty retained payloads for every missing value, including failed reads | A value that goes away is cleared once; a failed read keeps the last value |
| No availability | Last Will and `availability_topic` |
| Discovery every 5 minutes | Discovery on connect, on changes and on HA birth |
| About 60 `mosquitto_pub` processes per run, password on the command line | One persistent connection; the password is read from a root-only file |

Values treated as impossible (never announced, legacy leftovers removed)
when the interface has no such address:

- IPv4 address and gateway on `dhcpv6`, `6in4`, `6to4` and `6rd`
  interfaces. DS-Lite, MAP and 464XLAT are not in this list: they carry IPv4
  over IPv6 and their interface has an IPv4 address;
- IPv6 address and gateway on `dhcp`, `pppoe`, `pppoa`, `pptp` and `l2tp`
  interfaces, and on `static` interfaces without `ip6addr`, `ip6gw`,
  `ip6prefix` or `ip6assign` (their IPv6 is carried by a separate DHCPv6
  interface).

The rule only decides whether a *missing* address is permanent. An address
that exists is always published, whatever the protocol.

A link speed is impossible only for devices that never have one (PPP,
tunnels). Ethernet, bridge, VLAN and bond devices report a speed again when
the link comes up, so a missing speed there is an ordinary absent value.

## Metric templates

Placeholders: `{p}`/`{P}` is `wan`/`WAN` for interfaces matched by the legacy
rule (`wan*`, `wwan*`) and `iface`/`Interface` for other interfaces; `{n}` is
the raw and `{i}` the sanitized instance name (interface, device, peer,
service); `{m}`/`{j}` are the raw and sanitized sub-instance (mwan3 tracking
target, WireGuard peer). The key of a metric, used for its UCI `metric`
section, is the expanded default object id with `-` replaced by `_`.

"Default" is what a fresh install publishes: only the legacy set. The
"Recommended set" button on the Metrics page switches on the metrics marked
recommended.

<!-- BEGIN GENERATED TEMPLATES -->

### Group `system`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `system.hostname` | `system_hostname` | `system/hostname` | Hostname | sensor | - | on (legacy) |
| `system.model` | `system_model` | `system/model` | Model | sensor | - | on (legacy) |
| `system.target_platform` | `system_target_platform` | `system/target_platform` | Target Platform | sensor | - | on (legacy) |
| `system.architecture` | `system_architecture` | `system/architecture` | Architecture | sensor | - | on (legacy) |
| `system.version` | `system_version` | `system/version` | Version | sensor | - | on (legacy) |
| `system.uptime` | `system_uptime` | `system/uptime` | Uptime | sensor | s, duration | on (legacy) |
| `system.revision` | `system_revision` | `system/revision` | Firmware Revision | sensor | - | recommended, diagnostic |
| `system.kernel` | `system_kernel` | `system/kernel` | Kernel Version | sensor | - | recommended, diagnostic |
| `system.boot_time` | `system_boot_time` | `system/boot_time` | Boot Time | sensor | timestamp | off, diagnostic |
| `system.load_1` | `system_load_1` | `system/load_1` | Load (1 min) | sensor | measurement | recommended |
| `system.load_5` | `system_load_5` | `system/load_5` | Load (5 min) | sensor | measurement | recommended |
| `system.load_15` | `system_load_15` | `system/load_15` | Load (15 min) | sensor | measurement | recommended |
| `system.cpu_usage` | `system_cpu_usage` | `system/cpu_usage` | CPU Usage | sensor | %, measurement | recommended |
| `system.cpu_core_usage` | `system_cpu{i}_usage` | `system/cpu{n}_usage` | CPU {n} Usage | sensor | %, measurement | off |
| `system.memory_used` | `system_memory_used` | `system/memory_used` | Memory Used | sensor | %, measurement | recommended |
| `system.memory_available` | `system_memory_available` | `system/memory_available` | Memory Available | sensor | MiB, data_size, measurement | recommended |
| `system.root_used` | `system_root_used` | `system/root_used` | Root Filesystem Used | sensor | %, measurement | recommended |
| `system.tmp_used` | `system_tmp_used` | `system/tmp_used` | Temporary Filesystem Used | sensor | %, measurement | recommended |
| `system.swap_used` | `system_swap_used` | `system/swap_used` | Swap Used | sensor | %, measurement | off |
| `system.conntrack_count` | `system_conntrack_count` | `system/conntrack_count` | Conntrack Entries | sensor | measurement | recommended |
| `system.conntrack_used` | `system_conntrack_used` | `system/conntrack_used` | Conntrack Used | sensor | %, measurement | recommended |
| `system.temperature` | `system_temp_{i}` | `system/temp_{i}` | Temperature {n} | sensor | °C, temperature, measurement | recommended |

### Group `packages`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `packages.upgradable` | `system_upgradable_packages` | `system/upgradable_packages` | Upgradable Packages | sensor | measurement | off, every 86400 s |

### Group `interface`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `iface.status` | `{p}_{i}_status` | `{p}-{n}/status` | {P} {n} Status | binary_sensor | connectivity | on (legacy) |
| `iface.device` | `{p}_{i}_device` | `{p}-{n}/device` | {P} {n} Device | sensor | - | on (legacy) |
| `iface.ipv4` | `{p}_{i}_ipv4` | `{p}-{n}/ipv4` | {P} {n} IPv4 | sensor | - | on (legacy) |
| `iface.ipv6` | `{p}_{i}_ipv6` | `{p}-{n}/ipv6` | {P} {n} IPv6 | sensor | - | on (legacy) |
| `iface.gateway4` | `{p}_{i}_gateway4` | `{p}-{n}/gateway4` | {P} {n} Gateway4 | sensor | - | on (legacy) |
| `iface.gateway6` | `{p}_{i}_gateway6` | `{p}-{n}/gateway6` | {P} {n} Gateway6 | sensor | - | on (legacy) |
| `iface.proto` | `{p}_{i}_proto` | `{p}-{n}/proto` | {P} {n} Protocol | sensor | - | recommended, diagnostic |
| `iface.uptime` | `{p}_{i}_uptime` | `{p}-{n}/uptime` | {P} {n} Uptime | sensor | s, duration | recommended |
| `iface.dns` | `{p}_{i}_dns` | `{p}-{n}/dns` | {P} {n} DNS Servers | sensor | - | recommended, diagnostic |
| `iface.prefix` | `{p}_{i}_prefix` | `{p}-{n}/prefix` | {P} {n} IPv6 Prefix | sensor | - | recommended, diagnostic |

### Group `traffic`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `traffic.rx_bytes` | `dev_{i}_rx_bytes` | `dev-{n}/rx_bytes` | Device {n} RX Bytes | sensor | B, data_size, total_increasing | recommended |
| `traffic.tx_bytes` | `dev_{i}_tx_bytes` | `dev-{n}/tx_bytes` | Device {n} TX Bytes | sensor | B, data_size, total_increasing | recommended |
| `traffic.rx_rate` | `dev_{i}_rx_rate` | `dev-{n}/rx_rate` | Device {n} RX Rate | sensor | Mbit/s, data_rate, measurement | recommended |
| `traffic.tx_rate` | `dev_{i}_tx_rate` | `dev-{n}/tx_rate` | Device {n} TX Rate | sensor | Mbit/s, data_rate, measurement | recommended |
| `traffic.rx_errors` | `dev_{i}_rx_errors` | `dev-{n}/rx_errors` | Device {n} RX Errors | sensor | total_increasing | off, diagnostic |
| `traffic.tx_errors` | `dev_{i}_tx_errors` | `dev-{n}/tx_errors` | Device {n} TX Errors | sensor | total_increasing | off, diagnostic |
| `traffic.rx_dropped` | `dev_{i}_rx_dropped` | `dev-{n}/rx_dropped` | Device {n} RX Dropped | sensor | total_increasing | off, diagnostic |
| `traffic.tx_dropped` | `dev_{i}_tx_dropped` | `dev-{n}/tx_dropped` | Device {n} TX Dropped | sensor | total_increasing | off, diagnostic |
| `traffic.speed` | `dev_{i}_speed` | `dev-{n}/speed` | Device {n} Link Speed | sensor | Mbit/s, data_rate | off, diagnostic |
| `traffic.carrier` | `dev_{i}_carrier` | `dev-{n}/carrier` | Device {n} Carrier | binary_sensor | connectivity | off, diagnostic |

### Group `mwan3`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `mwan3.status` | `mwan3_{i}_status` | `mwan3-{n}/status` | MWAN3 {n} Status | sensor | - | recommended |
| `mwan3.online` | `mwan3_{i}_online` | `mwan3-{n}/online` | MWAN3 {n} Online | binary_sensor | connectivity | recommended |
| `mwan3.score` | `mwan3_{i}_score` | `mwan3-{n}/score` | MWAN3 {n} Score | sensor | measurement | recommended |
| `mwan3.online_time` | `mwan3_{i}_online_time` | `mwan3-{n}/online_time` | MWAN3 {n} Online Time | sensor | s, duration | recommended |
| `mwan3.lost` | `mwan3_{i}_lost` | `mwan3-{n}/lost` | MWAN3 {n} Lost Probes | sensor | measurement | off, diagnostic |
| `mwan3.track_latency` | `mwan3_{i}_{j}_latency` | `mwan3-{n}/{j}/latency` | MWAN3 {n} {m} Latency | sensor | ms, duration, measurement | recommended |
| `mwan3.track_loss` | `mwan3_{i}_{j}_loss` | `mwan3-{n}/{j}/loss` | MWAN3 {n} {m} Packet Loss | sensor | %, measurement | recommended |
| `mwan3.track_status` | `mwan3_{i}_{j}_status` | `mwan3-{n}/{j}/status` | MWAN3 {n} {m} Status | sensor | - | off, diagnostic |

### Group `ha`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `ha.role` | `ha_role` | `ha/role` | HA Role | sensor | - | recommended |
| `ha.state` | `ha_state` | `ha/state` | HA State | sensor | - | recommended |
| `ha.peer_online` | `ha_peer_{i}_online` | `ha/peer_{i}/online` | HA Peer {n} Online | binary_sensor | connectivity | recommended |
| `ha.service_running` | `ha_service_{i}_running` | `ha/service_{i}/running` | HA Service {n} | binary_sensor | running | recommended |
| `ha.vrrp_state` | `ha_vrrp_{i}_state` | `ha/vrrp_{i}/state` | HA VRRP {n} State | sensor | - | recommended |
| `ha.config_sync_age` | `ha_config_sync_age` | `ha/config_sync_age` | HA Config Sync Age | sensor | s, duration | off, diagnostic |
| `ha.lease_sync_age` | `ha_lease_sync_age` | `ha/lease_sync_age` | HA Lease Sync Age | sensor | s, duration | off, diagnostic |

### Group `dhcp`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `dhcp.leases_ipv4` | `dhcp_leases_ipv4` | `dhcp/leases_ipv4` | DHCP Leases IPv4 | sensor | measurement | recommended |
| `dhcp.leases_ipv6` | `dhcp_leases_ipv6` | `dhcp/leases_ipv6` | DHCP Leases IPv6 | sensor | measurement | recommended |

### Group `wireguard`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `wireguard.peers` | `wg_{i}_peers` | `wg-{n}/peers` | WireGuard {n} Peers | sensor | measurement | off |
| `wireguard.peer_connected` | `wg_{i}_{j}_connected` | `wg-{n}/{j}/connected` | WireGuard {n} {m} Connected | binary_sensor | connectivity | off |
| `wireguard.peer_handshake_age` | `wg_{i}_{j}_handshake_age` | `wg-{n}/{j}/handshake_age` | WireGuard {n} {m} Handshake Age | sensor | s, duration | off |
| `wireguard.peer_rx` | `wg_{i}_{j}_rx_bytes` | `wg-{n}/{j}/rx_bytes` | WireGuard {n} {m} RX Bytes | sensor | B, data_size, total_increasing | off |
| `wireguard.peer_tx` | `wg_{i}_{j}_tx_bytes` | `wg-{n}/{j}/tx_bytes` | WireGuard {n} {m} TX Bytes | sensor | B, data_size, total_increasing | off |

### Group `wifi`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `wifi.clients` | `wifi_{i}_clients` | `wifi-{n}/clients` | Wi-Fi {n} Clients | sensor | measurement | recommended |
| `wifi.channel` | `wifi_{i}_channel` | `wifi-{n}/channel` | Wi-Fi {n} Channel | sensor | - | off, diagnostic |
| `wifi.txpower` | `wifi_{i}_txpower` | `wifi-{n}/txpower` | Wi-Fi {n} TX Power | sensor | dBm | off, diagnostic |

### Group `service`

| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |
| --- | --- | --- | --- | --- | --- | --- |
| `service.running` | `service_{i}_running` | `service-{n}/running` | Service {n} | binary_sensor | running | off |

<!-- END GENERATED TEMPLATES -->

Regenerate the table with:

```sh
mqtt-metricsd --templates | tools/gen-topics.py docs/TOPICS.md
```
