# Release notes

## 0.1.0 (unreleased)

First version of `mqtt-metrics` and `luci-app-mqtt-metrics` for OpenWrt 25.12.
It replaces the `publish_metrics.sh` cron script.

### Compatibility with publish_metrics.sh

- With the default configuration the same state topics, Home Assistant
  `unique_id`s, entity names, icons and classes are published every 300
  seconds, so existing entities keep their history.
- Fixed legacy defects: no empty retained payloads, no link-local IPv6
  address for PPPoE interfaces, no entities that can never have a value (they
  are removed once), discovery only on connect, change and Home Assistant
  birth, availability through a Last Will.
- `mqtt-metricsd --import-legacy` (or **Import broker settings** in LuCI)
  copies the broker settings from the legacy script, the password on the
  router only.

### Daemon

- One persistent MQTT 3.1.1 connection; QoS 0/1, bounded in-flight window,
  keepalive, reconnect with backoff (5 s doubling to 120 s).
- TLS with rustls: system or custom CA, optional client certificate,
  optional unverified mode for self-signed brokers.
- Read-only by design: the only subscription is the Home Assistant birth
  topic.
- Metric groups: system, network interfaces, traffic, mwan3, HA cluster
  (ha-cluster/keepalived), DHCP, WireGuard, Wi-Fi, services, packages; 69
  templates in total, the legacy twelve on by default.
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
  legacy defaults".
- Connection test with unsaved values, discovery preview, service log,
  legacy-script detection with migration steps.

### Measured on the x86_64 test routers

- Package `mqtt-metrics` 0.97 MB (binary 1.9 MB with TLS), LuCI package
  15 KB.
- Legacy set: RSS 2.4 MB. Every metric (164 on the primary) at a 30 s
  interval: RSS 2.9 MB and 0.16 % of one CPU core including the `ubus` and
  `uci` helper processes.
