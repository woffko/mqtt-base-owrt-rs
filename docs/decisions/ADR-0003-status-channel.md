# ADR-0003: Daemon to rpcd communication

Status: accepted, 2026-09-28

## Context

LuCI needs live status, the full metric catalog (including metrics that are
switched off), and a few actions (republish, purge, reload, test
connection). The daemon is a single-threaded blocking loop without a ubus
connection of its own.

## Decision

- The same binary is the rpcd exec plugin
  (`/usr/libexec/rpcd/mqtt-metrics`, a symlink shipped by the LuCI package).
- Status: the daemon writes `/var/run/mqtt-metrics/status.json` (tmpfs,
  mode 0600) after every publish round and at least every 15 s; rpcd
  `status` reads it and checks that the recorded pid is alive.
- Catalog and preview: the rpcd plugin runs one collection round itself with
  the current configuration. It does not depend on the daemon running and
  shows metrics that the daemon does not collect. Last published values
  come from the status file.
- Actions: a Unix socket `/var/run/mqtt-metrics/control.sock` (mode 0600)
  accepts `reload`, `republish` and `purge` as one JSON line.
- Test connection runs in the rpcd process with a separate client id, so it
  never takes over the daemon's session.

## Consequences

- No libubus binding and no `unsafe` FFI.
- A catalog request spawns about a dozen short helper processes (ubus, uci);
  it takes well under a second on the test router.
- Nothing received over MQTT can reach these actions.
