# mqtt-base-owrt-rs - Analysis and Work Plan

Status: v2, 2026-09-28, owner decisions from section 11 applied. Source of
truth for the implementation Goal (`docs/plan/GOAL.md`). Deviations are
recorded as ADRs in `docs/decisions/`.

## 1. Goal

Replace the legacy MQTT metrics script on OpenWrt routers with two packages
built from the repository `mqtt-base-owrt-rs`:

- `mqtt-metrics` - a Rust daemon (`/usr/sbin/mqtt-metricsd`) that keeps one
  persistent MQTT connection, collects router state from ubus, `/proc` and
  `/sys`, and publishes it with Home Assistant (HA) MQTT discovery;
- `luci-app-mqtt-metrics` - a full LuCI application to configure the broker,
  topics, device identity and intervals, choose which metrics are published,
  hide metrics from HA, and see live status.

The integration is read-only by design: the router publishes state and never
accepts commands over MQTT.

This is the base package with system-level metrics that every router has.
Application-specific publishers (CAKE autorate, NeuroGuard, and so on) can be
added later as separate `mqtt-metrics-<source>` packages through the
extension point in section 6.6.

Names: `mqtt-metrics` is used for the package, UCI config
(`/etc/config/mqtt-metrics`), init script, rpcd object, and runtime paths.
The implementation language is not part of the package name because no other
package in the feeds uses these names (checked against the 25.12.5 SDK feeds).

## 2. Legacy setup (verified 2026-09-28)

### 2.1 How it is installed

`/etc/rc.local` contains a one-time installer block. If
`/etc/.publish_metrics_installed` does not exist, it writes
`/usr/bin/publish_metrics.sh` from a heredoc, adds
`*/5 * * * * /usr/bin/publish_metrics.sh` to `/etc/crontabs/root`, restarts
cron, and creates the marker. Broker address, user and password are
hard-coded in the heredoc. The same `rc.local` also installs an unrelated HA
config-sync script (out of scope for this project).

Both HA nodes run the identical script (same checksum), cron line and
`rc.local` block:

| Node | Address | HA role |
| --- | --- | --- |
| primary | 10.0.77.1 | MASTER |
| secondary | 10.0.77.2 | BACKUP |

### 2.2 What it publishes

Every 5 minutes, for the device `openwrt_<hostname>`:

| Group | Topic (`openwrt/<hostname>/...`) | HA entity |
| --- | --- | --- |
| System | `system/hostname`, `system/model`, `system/target_platform`, `system/architecture`, `system/version` | sensor |
| System | `system/uptime` | sensor, `duration`, `s` |
| Per WAN interface | `wan-<iface>/status` (`up`/`down`) | binary_sensor, `connectivity` |
| Per WAN interface | `wan-<iface>/device`, `ipv4`, `ipv6`, `gateway4`, `gateway6` | sensor |

WAN interfaces are UCI interfaces whose names match `wan*` or `wwan*`. On the
test routers these are `wan`, `wan_6`, `wanb`, `wanb_6`, so the script creates
6 system entities and 24 WAN entities per router. Discovery topics are
`homeassistant/{sensor,binary_sensor}/openwrt_<host>_<object>/config`; state
and discovery messages are retained with QoS 1. Device name is
`OpenWrt <hostname>`, manufacturer `OpenWrt`, model `Router`.

### 2.3 Defects to fix

1. About 60 `mosquitto_pub` processes and TCP connections per run
   (12 system messages plus 12 per WAN interface).
2. The broker password is passed on the command line (`-P`) and is visible in
   `ps` and `/proc/*/cmdline`; it is also stored in plain text in `rc.local`
   and therefore in every sysupgrade backup.
3. No availability topic or Last Will. When a router is down, HA keeps showing
   the last retained values as current.
4. Empty values are published as retained empty payloads, which deletes the
   retained state; HA shows `unknown`. Entities are created that can never
   have a value (IPv4 of `wan_6`, IPv6 gateway of an IPv4-only interface).
5. For PPPoE `wan` the IPv6 value is a link-local `fe80::` address.
6. Discovery is re-sent every 5 minutes; nothing is ever removed from HA.
7. No `state_class`, so HA keeps no long-term statistics; no
   `entity_category`, no `configuration_url`, no `sw_version`.
8. Hostname is not sanitized for topic use; JSON escaping handles only `\`
   and `"`.
9. Interface selection is a fixed regular expression; there is no way to add
   another interface or hide one.
10. If the marker file is lost (for example after a sysupgrade that does not
    keep it), `rc.local` reinstalls the script and cron line.
11. Busybox crond logs every run at `cron.err`: 288 syslog lines per day.

## 3. Data sources available on the test routers

Verified with read-only calls on OpenWrt 25.12-SNAPSHOT x86_64:

| Source | Content |
| --- | --- |
| `ubus call system board` / `system info` | model, board, kernel, release, uptime, load, memory, root/tmp usage, swap |
| `/proc/stat`, `/proc/loadavg` | CPU usage (4 CPUs), runnable/total tasks |
| `ubus call network.interface dump` | per-interface up, proto, device, addresses, routes, DNS, uptime, delegated prefixes |
| `ubus call network.device status` | per-device carrier, speed, rx/tx bytes, packets, errors, drops |
| `ubus call mwan3 status` | per-WAN status, score, uptime, tracking, per-target latency and loss |
| `ubus call ha-cluster status` | role, VRRP instance states, peer online, cluster state, service health, last sync |
| `ubus call keepalived dump` | VRRP instance details |
| `ubus call luci-rpc getDHCPLeases` | DHCPv4/DHCPv6 leases |
| `ubus call luci.wireguard getWgInstances` | WireGuard interfaces and peers |
| `ubus call iwinfo devices` | wireless devices (none on these x86 VMs) |
| `ubus call service list` | procd service state |
| `/proc/sys/net/netfilter/nf_conntrack_{count,max}` | conntrack usage |
| `/sys/class/hwmon`, `/sys/class/thermal` | temperatures (none on these VMs) |

Every collector must detect a missing source and report the metric as
unavailable with a reason; it must never publish a placeholder.

## 4. Metric catalog

Defaults reproduce the legacy script: after a fresh install only the metrics
marked `L` are on, with the legacy interface selection rule, topics, IDs and
interval. Everything else is available in LuCI and off until the owner turns
it on. The Metrics page offers a "Recommended set" action that turns on the
metrics marked `R` in one step.

### 4.1 System

| Metric | Unit / class | Flags |
| --- | --- | --- |
| hostname, model, target platform (board), architecture, version | text | L |
| uptime | s, `duration` | L |
| firmware revision, kernel version | text, diagnostic | R |
| boot time | `timestamp` | |
| load 1/5/15 min | `measurement` | R |
| CPU usage total (per-core optional) | %, `measurement` | R |
| memory used %, available bytes | %, B | R |
| root and tmp filesystem used % | % | R |
| swap used % | % | |
| conntrack entries, conntrack used % | count, % | R |
| temperature per sensor | °C, `temperature` | R if a sensor exists |
| upgradable packages | count, daily interval | |

### 4.2 Network interfaces (per selected logical interface)

Default selection is the legacy `wan*`/`wwan*` rule; any interface can be
added or removed in LuCI.

| Metric | Flags |
| --- | --- |
| status (binary `connectivity`) | L |
| device, IPv4, IPv6 (global only), gateway IPv4, gateway IPv6 | L; created only for the address family the interface has |
| protocol, interface uptime, DNS servers, delegated IPv6 prefix | R |

### 4.3 Traffic (per selected L3 device)

rx/tx bytes (`data_size`, `total_increasing`), rx/tx rate (bit/s, computed
from counter deltas with wrap and reset handling), errors and drops, link
speed, carrier. Rates and byte counters are `R` for the devices of the
selected WAN interfaces.

### 4.4 Multi-WAN (`mwan3`, when installed)

Per WAN: status (`online`/`offline`/`disabled`), score, online uptime, and per
tracking target latency and packet loss. `R` when `mwan3` is present.

### 4.5 HA cluster (`ha-cluster`/`keepalived`, when installed)

Role (`MASTER`/`BACKUP`), cluster state, peer online, per-service running
flag, VRRP instance states, age of the last config sync. `R` when present.

### 4.6 Other groups

- DHCP: active IPv4 and IPv6 lease counts (`R`).
- WireGuard: per interface peer count; per peer last-handshake age and rx/tx.
- Wi-Fi (when `iwinfo` reports devices): per radio client count, channel,
  TX power.
- Services: running/stopped for procd services selected in LuCI.

### 4.7 Read-only by design

The project never controls the router from HA or MQTT: no `switch`, `button`,
`select`, `number` or other command entities, no command topics, and no
code path that turns an MQTT message into an action on the router. The only
subscription is the HA birth topic (`homeassistant/status`, configurable),
which only triggers a discovery resend. This is a permanent scope decision,
not a v1 limitation.

## 5. Topic, identity and interval design

Everything below is configurable in LuCI; the defaults are the legacy values.

| Setting | Default (legacy) |
| --- | --- |
| Base topic prefix | `openwrt` |
| Node path | runtime hostname |
| Discovery prefix | `homeassistant` |
| Device ID | `openwrt_<sanitized hostname>` |
| Device name | `OpenWrt <hostname>` |
| Device manufacturer / model | `OpenWrt` / `Router` |
| Entity `unique_id` | `<device id>_<object id>` |
| State topic per metric | legacy path (`system/uptime`, `wan-<iface>/ipv4`, ...) |
| Entity name | legacy name (`Uptime`, `WAN <iface> IPv4`, ...) |
| QoS / retain | 1 / on |
| Interval | 300 s for all groups |

- The hostname is read at runtime, so a configuration synced between HA
  nodes still gives each node its own device.
- Per metric, LuCI can override the object ID, state topic suffix, entity
  name, icon, and interval. Changing the device ID, prefix or object ID
  creates new HA entities and loses history; LuCI warns before saving and
  offers to clear the old retained discovery topics.
- Intervals: one global interval (default 300 s, allowed 5-86400 s), optional
  per-group and per-metric overrides, and an optional "publish only on
  change" mode with a maximum age (off by default, so the default behaviour
  is a full publish every interval as today). Slow sources (package updates)
  have their own default of 24 h.
- Availability: `<prefix>/<node>/availability`, retained `online` after
  CONNECT and `offline` as the Last Will and on clean shutdown. Every
  discovery config references it. This topic is new; it does not change any
  legacy topic.
- Discovery: per-entity config topics
  `<discovery prefix>/<component>/<unique_id>/config`, retained, QoS 1, with
  `device`, `origin`, `state_class`, `device_class`,
  `unit_of_measurement`, `entity_category`, and `enabled_by_default` where
  appropriate. Discovery is sent on connect, on configuration change, and
  when HA publishes `online` on its birth topic, not on every cycle.
- Hide semantics:
  - "Publish" off: the metric is not collected; its discovery config and
    retained state are cleared with empty retained payloads, so HA removes the
    entity.
  - "Show in Home Assistant" off: state is still published (for Node-RED or
    other consumers), discovery is cleared.
- Empty or unavailable values are never sent as empty retained payloads.
- Migration cleanup: on first start the daemon clears discovery configs for
  legacy entities that can never have a value (section 2.3, item 4). It is
  shown on the Diagnostics page and can be disabled.

## 6. Architecture

### 6.1 Daemon

One statically linked musl binary, `std` only plus a minimal, pinned
dependency set (`serde`/`serde_json` for ubus JSON and discovery payloads;
`rustls` behind an optional `tls` feature). No async runtime.

Modules:

- `config` - reads UCI through `uci -q show mqtt-metrics` at start and on
  reload, validates everything once, and produces an immutable `Config`.
- `catalog` - static metric definitions (id, group, component, unit, classes,
  icon, default and recommended flags, legacy mapping) plus dynamic instances
  per interface, device, WAN, peer, and service.
- `collect` - one collector per source; ubus calls run `ubus -t <timeout>
  call` with a bounded output size; `/proc` and `/sys` are read directly.
  Collectors return typed samples, never pre-formatted strings.
- `mqtt` - in-house MQTT 3.1.1 client over `TcpStream` (optional TLS):
  CONNECT with Last Will, PUBLISH QoS 0/1 with PUBACK tracking, SUBSCRIBE for
  the HA birth topic only, PINGREQ keepalive, clean DISCONNECT, reconnect
  with bounded exponential backoff. Incoming PUBLISH packets on any other
  topic are dropped. The codec is fuzzed and unit-tested against captured
  packets.
- `ha` - discovery payload builder and the publish scheduler.
- `control` - Unix socket in `/var/run/mqtt-metrics/` for local commands
  from rpcd (republish, purge, test connection); status is written
  atomically to `/var/run/mqtt-metrics/status.json`.

Decision to confirm in Phase 0 (ADR-0001): in-house MQTT client versus the
`rumqttc` crate. The plan assumes in-house because `rumqttc` pulls in `tokio`
and increases binary size several times on small targets.

### 6.2 Process integration

- `/etc/init.d/mqtt-metrics` - procd, `respawn`,
  `procd_add_reload_trigger mqtt-metrics`, starts after network (`START=95`).
- `/etc/config/mqtt-metrics` - UCI, mode 0600, listed in `conffiles`.
  Sections: `broker`, `identity` (prefixes, device ID and name), `publish`
  (interval, change-only mode, max age, QoS, retain), `group` (per group
  on/off and interval), `metric` (per-metric overrides: publish, show in HA,
  object ID, topic suffix, name, icon, interval), and
  `interface`/`device`/`service` selections.
- `/usr/libexec/rpcd/mqtt-metrics` - symlink to the daemon binary, which
  implements the rpcd exec protocol (`list`, `call <method>`).

### 6.3 rpcd methods

| Method | ACL | Purpose |
| --- | --- | --- |
| `status` | read | connection state, last error, counters, last publish time |
| `catalog` | read | every metric on this router with current value, availability and reason, effective flags, topics and IDs |
| `preview` | read | discovery JSON for one metric (password-free) |
| `test_connection` | write | connect with candidate settings; uses the stored password when the field was not changed |
| `republish` | write | resend discovery and full state |
| `purge` | write | clear retained topics of this device (all, or the old IDs after a rename) |

These methods are for the local LuCI session only. The password is
write-only: `catalog`, `status`, and LuCI never return it.

### 6.4 LuCI application (`Services -> MQTT Metrics`)

1. **Status** - broker connection, availability, last publish, message and
   error counters, device identity; grouped table of published metrics with
   live values (poll); buttons "Test connection" and "Republish discovery".
2. **Broker** - host, port, TLS (CA upload, verify toggle, optional client
   certificate), username, write-only password, client ID, keepalive, QoS,
   retain.
3. **Topics and device** - base prefix, node path, discovery prefix, HA
   birth topic, device ID, device name, manufacturer and model, with the
   rename warning and old-topic cleanup from section 5.
4. **Metrics** - global interval and change-only mode; groups with bulk
   toggles and group interval; per metric "Publish", "Show in Home
   Assistant", "Enabled by default in HA", "Diagnostic", object ID, topic
   suffix, name, icon, interval; current value next to each row;
   unavailable sources greyed out with the reason; pickers for interfaces,
   devices, WANs, WireGuard peers and services populated from the router;
   "Recommended set" and "Reset to legacy defaults" actions.
5. **Diagnostics** - discovery preview per metric, recent daemon log, purge
   retained topics, legacy-script detection (shows whether the cron line,
   `/usr/bin/publish_metrics.sh` and the `rc.local` block exist) with the
   documented migration steps. It does not edit those files.

All strings use `_()`. Pages must work on mobile width.

### 6.5 Resource budget (x86_64 test router, measured in Phase 4)

- Binary without TLS at most 1.5 MB stripped; with TLS at most 3 MB.
- RSS at most 6 MB.
- Average CPU below 0.5 % of one core at a 30 s interval with all groups on.
- One TCP connection to the broker.

### 6.6 Extension point (after v1)

Other packages can drop declarative collectors into
`/usr/share/mqtt-metrics/collectors.d/*.json` (ubus object, method, JSON
pointer, metric metadata). The daemon and the Metrics page pick them up
without code changes. v1 ships the loader and schema only if Phase 2 finishes
early; otherwise it is an ADR for v1.1. Extensions are read-only like the
base package.

## 7. Repository layout

```text
AGENTS.md, CLAUDE.md, README.md, LICENSES/
.agents/skills/{openwrt-package,rust,documentation-text-writing}
feeds.conf.example
package/mqtt-metrics/{Makefile,files/,src/}
package/luci-app-mqtt-metrics/{Makefile,htdocs/,root/,tests/}
tests/fixtures/            sanitized ubus and /proc captures
tools/                     sdk-build.sh, deploy.sh, parity-check.sh, soak-monitor.sh
docs/plan/, docs/decisions/, docs/evidence/
docs/USER_GUIDE.md, docs/TOPICS.md, docs/MIGRATION.md
```

Licences: `mqtt-metrics` is GPL-2.0-or-later, `luci-app-mqtt-metrics` is
Apache-2.0 (same split as cake-autorate-rs).

## 8. Build and test environment

- Target: x86_64 only for now (both test routers are x86_64). Other
  architectures come later through the same package Makefiles.
- SDK: `/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64`
  already has `rust/host` built. Use the overlay workflow: symlink the two
  package directories into `<sdk>/package/` and build with
  `make package/<pkg>/compile V=s -j1`. Do not modify other packages or clean
  that SDK.
- Fast loop: `cargo build --release --target x86_64-unknown-linux-musl` (the
  target is installed on the host).
- Unit tests: codec, discovery JSON (golden files), collectors against
  sanitized fixtures, counter-wrap rate math, config validation, and a test
  that no incoming MQTT message except the birth topic reaches any handler.
- Integration test: an in-process fake broker in `tests/` records packets; no
  system Mosquitto is required.
- Legacy parity: `tools/parity-check.sh` subscribes on the router to the
  legacy topics and to the daemon's shadow prefix and compares topic sets and
  values.
- LuCI: Node tests for view logic and ACL coverage; Playwright screenshots
  (desktop and mobile) with the local installation in
  `/home/w0w/.local/share/playwright` (no `npx` downloads).
- Broker settings for shadow mode are copied on the router from
  `/usr/bin/publish_metrics.sh` into `/etc/config/mqtt-metrics` with a
  router-side command that never prints them; they never pass through the
  agent's output, the repository, or chat.

## 9. Phases

Each phase ends with a commit on `main`, green `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and the phase's
acceptance checks.

### Phase 0 - Bootstrap

- `git init -b main`, feed skeleton, Cargo project, `feeds.conf.example`,
  licence files, `tools/sdk-build.sh`.
- Capture and sanitize fixtures from both routers (read-only; the secondary
  gives the `BACKUP` role fixtures).
- ADR-0001 (MQTT client), ADR-0002 (topic and ID scheme with configurable
  overrides), ADR-0003 (daemon-to-rpcd status channel).
- Acceptance: an SDK build of a minimal daemon and LuCI package produces two
  `.apk` files.

### Phase 1 - Legacy parity

- UCI config with legacy defaults, procd init, catalog with legacy metrics,
  collectors for system and interfaces, MQTT client with Last Will, discovery
  with legacy IDs, rpcd `status`.
- Shadow mode: separate base prefix and discovery off, for side-by-side
  comparison.
- Acceptance: parity check on 10.0.77.1 in shadow mode shows the same topics
  and values as the legacy script (except documented fixes: no empty
  payloads, no link-local IPv6).

### Phase 2 - Extended metrics

- Collectors for sections 4.1-4.6, per-group and per-metric intervals,
  change-only mode, availability, HA birth subscription, hide semantics and
  migration cleanup.
- Acceptance: all groups verified on 10.0.77.1 (`MASTER`) and 10.0.77.2
  (`BACKUP`); missing sources (Wi-Fi, thermal) reported as unavailable;
  toggling a metric adds or removes its retained discovery config.

### Phase 3 - LuCI application

- Five pages from section 6.4, rpcd methods from 6.3, ACL, menu, tests,
  screenshots.
- Acceptance: every setting round-trips through UCI; "Reset to legacy
  defaults" restores the legacy configuration; the password never appears in
  page source or rpcd replies; screenshots reviewed.

### Phase 4 - Hardening and soak

- TLS feature, reconnect backoff, fault injection (broker down, wrong
  password, ubus timeout, `mwan3` removed, oversized ubus output), MQTT codec
  fuzzing, resource measurements against section 6.5, secret-leak checks
  (`ps`, `logread`, `/proc/*/cmdline`, rpcd replies).
- Soak: install the final candidate in shadow mode on both routers and start
  `tools/soak-monitor.sh` on each (samples service state, restarts, RSS, CPU
  and error log lines every 5 minutes into `/tmp/mqtt-metrics-soak/`). Record
  the start time and the next check time (start + 24 h, router local time
  and time zone) in `docs/evidence/soak-<date>.md`.
- While the soak runs, do only work that does not touch the routers:
  Phase 5 preparation (backup and rollback scripts, exact commands) and
  Phase 6 documentation. Do not reinstall the package during the soak.
- Then stop and report the next check time. The work continues when the
  owner resumes the session after that time.
- Acceptance (after resume): 24 h without restarts, memory growth or errors
  on both routers.

### Phase 5 - Cutover (each router needs owner confirmation)

Primary 10.0.77.1 first, then secondary 10.0.77.2:

1. Back up `/etc/rc.local`, `/etc/crontabs/root`,
   `/usr/bin/publish_metrics.sh`.
2. Switch the daemon from shadow mode to the legacy prefix with discovery on.
3. Remove the `publish_metrics.sh` cron line and restart cron.
4. With explicit approval, remove the MQTT installer block from
   `/etc/rc.local` (otherwise a lost marker reinstalls the script).
5. MQTT-side check by the agent: retained discovery and state topics match
   the legacy IDs, availability turns `offline` when the service stops.
   HA-side check (entity history, new entities) is done by the owner.
6. Rollback test: stop the daemon, restore the cron line, confirm the legacy
   script publishes again, then return to the new daemon.

### Phase 6 - Documentation and release

- README, USER_GUIDE, TOPICS (topic and entity reference), MIGRATION (legacy
  cutover and rollback), release notes.
- The GitHub repository `mqtt-base-owrt-rs` is private for now. Creating it,
  pushing, or making a release requires the owner's explicit approval.

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| Test routers carry real traffic and form the HA pair | read-only by default; package install only; no firewall restart, no ubusd signals; checks after every install |
| HA entity history lost at cutover | legacy topics and IDs are the defaults; parity check before cutover; rename warning in LuCI |
| Secrets leak | write-only password in LuCI, 0600 UCI file, no argv, router-side credential copy, secret-leak tests |
| Remote control through MQTT | read-only design, only the birth topic is subscribed, test that other messages are dropped |
| ubus output format changes between releases | typed parsing with fixtures per release; unknown fields ignored; missing fields make metrics unavailable |
| Broker unreachable | bounded backoff, availability via Last Will, no unbounded queues |
| SDK shared with cake-autorate | overlay only this project's packages; no clean commands |

## 11. Owner decisions (2026-09-28)

1. Licences: GPL-2.0-or-later for the daemon, Apache-2.0 for LuCI.
2. Intervals are configurable in LuCI; the default is the legacy 300 s.
3. Topics, IDs, names and prefixes are configurable in LuCI; the defaults are
   exactly what the legacy script publishes.
4. No control of the router from HA at all; the integration is read-only.
5. The secondary router is 10.0.77.2 with the same access as the primary.
6. Repository `mqtt-base-owrt-rs`, private for now. Package names are chosen
   by the agent: `mqtt-metrics` and `luci-app-mqtt-metrics`.
7. The 24 h soak does not block the session: start it, finish router-free
   work, stop, and report the next check time.
8. The owner checks Home Assistant; the agent checks the MQTT side.
9. Broker settings are copied on the router from the legacy script into UCI
   without passing through the agent.
10. x86_64 only for now; other architectures later.

## 12. Security notes outside this project

- `/etc/rc.local` on both routers stores the MQTT password and the HA peer's
  root password in plain text; both are copied into every sysupgrade backup.
  The cutover removes the MQTT part. The HA config-sync part is not touched
  by this project, but it should move to key-based SSH.
- Both routers present the same SSH host key (ED25519), probably from a
  cloned image or synced dropbear keys, so a client cannot tell them apart by
  key.
