# Release notes

## 0.1.0 (unreleased)

First version of `mqtt-metrics` and `luci-app-mqtt-metrics` for OpenWrt 25.12.

### Defaults

- Every 300 seconds the system identity and uptime and the state, device,
  addresses and gateways of the `wan*`/`wwan*` interfaces are published. Their
  topics, Home Assistant `unique_id`s, names, icons and classes stay the same
  between versions, so Home Assistant keeps the history.
- No link-local IPv6 address for PPPoE interfaces, no entities that can never
  have a value, discovery only on connect, change and Home Assistant birth,
  availability through a Last Will.

### Daemon

- One persistent MQTT 3.1.1 connection; QoS 0/1, bounded in-flight window,
  keepalive, reconnect with backoff (5 s doubling to 120 s).
- TLS with rustls: system or custom CA, optional client certificate,
  optional unverified mode for self-signed brokers.
- Read-only by design: the only subscription is the Home Assistant birth
  topic.
- Metric groups: system, network interfaces, traffic, mwan3, HA cluster
  (ha-cluster/keepalived), DHCP, WireGuard, Wi-Fi, services, packages; 69
  templates in total, twelve on by default plus the selected services.
- Intervals per metric, per group and global; optional change-only
  publishing with a maximum age.
- Retained topics owned by the router are tracked, so hidden metrics,
  deselected instances and renamed ids are removed from the broker and from
  Home Assistant automatically.
- The password lives in `/etc/mqtt-metrics/secret` (0600), outside UCI, and
  never appears in argv, logs, status files or rpcd replies.

### LuCI

- Status, Broker, Topics and device, Metrics and Diagnostics pages under
  **Services → MQTT Metrics**; desktop and mobile layouts.
- Per-metric Publish / Show in Home Assistant switches, name, object id,
  topic suffix, icon and interval overrides, "Recommended set" and "Reset to
  defaults".
- Connection test with unsaved values, discovery preview, service log.

### Measured on the x86_64 test routers

- Package `mqtt-metrics` 0.97 MB (binary 1.9 MB with TLS), LuCI package
  15 KB.
- Default set: RSS 2.4 MB. Every metric (164 on the primary) at a 30 s
  interval: RSS 2.9 MB and 0.16 % of one CPU core including the `ubus` and
  `uci` helper processes.

### Bug audit of 2026-09-29 (mqtt-metrics r7, luci-app-mqtt-metrics r6)

Details and evidence: `docs/evidence/audit-2026-09-29.md`.

- A value that goes away (a WAN goes down, a peer disappears) is cleared once
  from the broker, so Home Assistant shows it as unknown instead of the last
  value. A failed read (timeout, garbled answer) keeps the last value
  (ADR-0006). After a restart, sensors that need two samples show unknown for
  one interval.
- DS-Lite, MAP and 464XLAT interfaces keep their IPv4 address; an address
  that exists is never hidden by the protocol rule; `ip6assign` counts as
  IPv6 configuration.
- Entities whose names collide after sanitizing get numbered suffixes
  instead of overwriting each other.
- The link speed of an Ethernet port no longer removes and re-creates the
  entity when the link flaps.
- Boot time and other ages are not published before NTP has set the clock.
- A broker that denies the birth subscription no longer drops the connection;
  the Status page shows a warning. New option `birth` switches the
  subscription off.
- Reconnect delays grow to 120 s after dropped connections and reset after a
  stable minute or a configuration change.
- A second `mqtt-metricsd` is refused; `--purge` goes through a running
  service.
- Test connection sends the stored password only to the saved broker and
  honours the TLS switches.
- LuCI: Reset to defaults redraws every section; CPU and rate rows show the
  last value.

Found by the independent review of the same commit:

- A group whose source fails or is missing at a pass is read again at its
  interval instead of being dropped from the schedule; a full pass no longer
  removes the entities of a group that failed at that moment, and an entity
  removed by a full pass is announced again when it returns.
- Only groups with a metric switched on are read, so the default install no
  longer runs `apk list --upgradable` or queries `keepalived` at every
  connect.
- Topics cleared by a pass stay in `state.json` until the broker has
  acknowledged the batch, so a connection lost in between cannot orphan
  them.
- A malformed TLS key file no longer puts the key into the error message.
- The Metrics page shows the stored settings when the configuration has an
  error (it used to show defaults, and Save deleted every override); the
  Broker and Metrics forms reject the values the daemon refuses (port 0,
  decimal keepalive, names over 128 bytes).
- `set_password` without a password argument is an error instead of
  deleting the stored password.
- No more `Unknown Model`, `Unknown Platform` and `Unknown Version`
  placeholders; mwan3 zeros (no quality checks, no state), an empty package
  index and a stopped service are reported as what they are.
- `status.json` stays current while the service waits to reconnect; a
  helper that leaves a process behind cannot hold the service up.
- Every metric is published at its own interval (60 s and 90 s in one group
  used to become 60 s and 120 s).
- Disabling the service in LuCI and enabling it again works (the reload
  trigger was lost); a configuration that could not be read is read again;
  passes caused by Home Assistant announcements are at least 30 s apart; a
  publish waits at most 10 s for the broker to acknowledge earlier messages.
- Turning off "Retain states" clears the retained states left on the broker;
  the availability topic is cleared when the base topic or node changes while
  the service is stopped.
- LuCI: the identity warning survives a save; the Broker page says when it
  cannot know the password or TLS state; the password field is not filled by
  password managers; failed calls show their message.
- Tools: the browser scripts no longer print the router password in
  errors; `tools/e2e-smoke.sh` runs the daemon against a fake broker and
  `tools/luci-real.browser.js` runs the views on the real LuCI client code.

### Cleanup (mqtt-metrics r8, luci-app-mqtt-metrics r7)

- The migration from the shell script that served as the model for this
  daemon is gone (ADR-0007): the import of its broker settings, its
  detection on the Diagnostics page and the adoption of its retained topics.
  The option `identity.cleanup_legacy` is ignored if it is still set.
- A service selected on the Metrics page is published without a second
  switch.
- The package declares `libgcc` and `ca-bundle`, which the binary and the
  default TLS settings need.
