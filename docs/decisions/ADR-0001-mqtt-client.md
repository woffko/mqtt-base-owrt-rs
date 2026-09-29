# ADR-0001: In-house MQTT 3.1.1 client

Status: accepted, 2026-09-28

## Context

The daemon needs one persistent MQTT connection with a Last Will, QoS 0/1
publishing, one subscription (the Home Assistant birth topic) and keepalive.
It runs on routers with a few megabytes of free RAM and flash. The
candidates were the `rumqttc` crate, `paho-mqtt` (C library), and a small
client written for this project.

## Decision

Implement the subset of MQTT 3.1.1 that the daemon uses
(`src/mqtt/codec.rs`, `src/mqtt/client.rs`) on blocking `std::net`
sockets:

- CONNECT with clean session, Last Will, user name and password;
- PUBLISH QoS 0 and 1 with a bounded in-flight window (32) and PUBACK
  tracking; QoS 2 is refused;
- SUBSCRIBE at QoS 0, PINGREQ keepalive, DISCONNECT;
- incoming packets are limited to 64 KiB.

TLS is a separate transport behind the `tls` Cargo feature (rustls), so the
default build has no crypto code.

## Consequences

- No async runtime: `rumqttc` pulls in `tokio` and would make the binary
  several times larger; `paho-mqtt` adds a C dependency and `unsafe` FFI.
- The protocol code is ours to test: unit tests cover packet encoding, the
  remaining-length varint, partial input, oversized input and a
  deterministic garbage-input pass; an in-process fake broker tests connect,
  refusal, acknowledgements, subscriptions and keepalive.
- QoS 1 messages are not queued across reconnects. After every reconnect the
  daemon republishes discovery and the full state, so nothing is lost.

## Dependencies

AGENTS.md asks for a reason for every crate. The runtime crates are:

- `serde_json`: the `ubus` replies, the status and state files and the rpcd
  replies are JSON, and the standard library has no parser.
- `signal-hook`: SIGTERM, SIGHUP and SIGINT set atomic flags that the main
  loop polls. The standard library has no signal API, and installing a
  handler without this crate needs `unsafe` (forbidden without an ADR).
- `rustls` with the `ring` provider, only with the `tls` feature: ADR-0005.
