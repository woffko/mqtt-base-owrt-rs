# ADR-0005: Ship the package with TLS

Status: accepted, 2026-09-28

## Context

TLS is implemented behind the `tls` Cargo feature (rustls 0.23 with the ring
provider; ADR-0001). Measured static musl x86_64 release binaries: 0.91 MB
without TLS and 1.83 MB with TLS, both within the budget in PLAN.md section
6.5 (1.5 MB and 3 MB). The only architecture built now is x86_64.

## Decision

- The `mqtt-metrics` package is built with `--features tls`.
- Trust comes from the configured CA file or the system bundle
  (`/etc/ssl/certs/ca-certificates.crt`, package `ca-bundle`). `ca-bundle`
  is not a hard dependency: without it and without a CA file a TLS
  connection fails with a clear error, and plain MQTT needs neither.
- `tls_verify = 0` accepts any server certificate for self-signed brokers;
  LuCI explains that the broker is then not authenticated.
- Client certificates are supported (`tls_cert`, `tls_key`).

## Consequences

- One package, so the LuCI TLS options always work.
- When small targets are added, a `-nossl` variant (like `mosquitto-nossl`)
  can be split off with `PROVIDES`/`CONFLICTS`; the code already builds
  without the feature and CI covers both feature sets.
