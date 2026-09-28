# mqtt-base-owrt-rs

OpenWrt packages that publish router state to an MQTT broker with Home
Assistant discovery:

- **`mqtt-metrics`**: a Rust daemon (`/usr/sbin/mqtt-metricsd`) with one
  persistent MQTT connection, Last Will availability, optional TLS and
  change-only publishing;
- **`luci-app-mqtt-metrics`**: the LuCI application under
  **Services → MQTT Metrics** (Status, Broker, Topics and device, Metrics,
  Diagnostics).

The integration is read-only. The router publishes; it never acts on
messages received over MQTT. The only subscription is the Home Assistant
birth topic, which makes the daemon resend discovery after Home Assistant
restarts.

The packages replace the `publish_metrics.sh` cron script. With the default
configuration they publish the same state topics and Home Assistant
`unique_id`s, so existing entities keep their history. See
[docs/MIGRATION.md](docs/MIGRATION.md).

## What is published

A fresh install publishes the legacy set: hostname, model, target platform,
architecture, firmware version and uptime, plus status, device, addresses
and gateways of every `wan*`/`wwan*` interface. Everything else is opt-in on
the Metrics page:

| Group | Metrics |
| --- | --- |
| System | firmware revision, kernel, boot time, load, CPU usage (total and per core), memory, root and tmp usage, swap, conntrack, temperatures |
| Network interfaces | any interface: status, device, IPv4, global IPv6, gateways, protocol, uptime, DNS servers, delegated prefix |
| Traffic | per device: byte counters, rates, errors, drops, link speed, carrier |
| Multi-WAN | mwan3 status, score, online time, per-target latency and loss |
| HA cluster | role, cluster state, peer and service health, VRRP states, sync age |
| DHCP | active IPv4 and IPv6 leases |
| WireGuard | peers per interface; per peer handshake age, connected, traffic |
| Wi-Fi | clients, channel and TX power per radio |
| Services | running state of selected procd services |
| Packages | number of upgradable packages (from the local package index) |

Values a router cannot provide are shown in LuCI with the reason and are
never published as placeholders. The full reference, including topics and
entity ids, is in [docs/TOPICS.md](docs/TOPICS.md).

## Install

Build the packages in an OpenWrt 25.12 SDK (see below), copy them to the
router and install:

```sh
apk add --allow-untrusted mqtt-metrics-*.apk luci-app-mqtt-metrics-*.apk
```

Then open **Services → MQTT Metrics → Broker**, enter the broker and save.
The service stays idle until a broker host is set. The
[user guide](docs/USER_GUIDE.md) describes every page.

## Build

The repository is an OpenWrt package feed:

```text
package/mqtt-metrics/             daemon (Rust crate in src/)
package/luci-app-mqtt-metrics/    LuCI application
```

With an SDK whose `rust/host` is built:

```sh
tools/sdk-build.sh /path/to/openwrt-sdk    # packages land in dist/
```

or add the feed from `feeds.conf.example` and run
`make package/mqtt-metrics/compile package/luci-app-mqtt-metrics/compile`.

Development checks (formatting, clippy, Rust and LuCI tests):

```sh
tools/check.sh
```

## Documentation

- [User guide](docs/USER_GUIDE.md)
- [Topics and entities](docs/TOPICS.md)
- [Migration from publish_metrics.sh](docs/MIGRATION.md)
- [Release notes](docs/RELEASE_NOTES.md)
- [Work plan](docs/plan/PLAN.md) and [design decisions](docs/decisions/)

## Licence

The daemon (`package/mqtt-metrics`) is GPL-2.0-or-later, the LuCI
application (`package/luci-app-mqtt-metrics`) is Apache-2.0. Licence texts
are in [LICENSES/](LICENSES/).
