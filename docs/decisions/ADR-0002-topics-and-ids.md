# ADR-0002: Topic, identity and ownership scheme

Status: accepted, 2026-09-28

## Context

Existing Home Assistant entities were created by the legacy script and must
keep their history. The owner also wants every topic, id and name to be
configurable in LuCI, and hiding a metric must remove its entity.

## Decision

- Metric templates (`src/catalog.rs`) expand placeholders for an instance:
  `{p}`/`{P}` family prefix (`wan`/`WAN` for interfaces matched by the
  legacy `wan*`/`wwan*` rule, `iface`/`Interface` otherwise), `{n}` raw and
  `{i}` sanitized instance name, `{m}`/`{j}` for sub-instances.
- The twelve legacy templates reproduce the script: object id, state topic,
  name, icon and classes. With the default identity
  (`openwrt/<hostname>`, `homeassistant`, `openwrt_<host>`, model `Router`)
  every legacy `unique_id` and state topic is unchanged.
- A metric's key is its default object id restricted to `[A-Za-z0-9_]`; it
  names the UCI `metric` section that holds overrides (publish, show in HA,
  object id, topic suffix, name, icon, interval). Keys do not change when the
  object id is overridden.
- The daemon records every retained topic it owns in
  `/etc/mqtt-metrics/state.json`. A full pass (connect, reload, HA birth)
  clears owned topics that are no longer wanted. This removes hidden
  metrics, deselected interfaces and the old ids after a rename, without a
  separate cleanup step.
- On the first full pass with discovery on, the legacy script's topics for
  this hostname are adopted into the owned set, so legacy entities that can
  never have a value are removed. Shadow mode (discovery off) never touches
  discovery topics and never adopts legacy topics.
- Values that the interface protocol can never have (IPv4 on DHCPv6-only
  interfaces, global IPv6 on PPPoE, DHCP or static interfaces without IPv6
  configuration) are "permanent" unavailable readings: they are neither
  announced nor given a state.

## Consequences

- The file changes only when the owned set changes, so steady operation does
  not write to flash. It is listed in `/lib/upgrade/keep.d/mqtt-metrics`.
- Losing the file only means stale topics from before the loss are not
  cleaned automatically; LuCI "Reset retained topics" clears what is known.
- `tests/legacy_parity.rs` compares a full pass on the primary router
  fixture with the retained topics the legacy script left on the broker.
