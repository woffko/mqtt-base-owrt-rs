# Phase 4 evidence: hardening (2026-09-28)

## TLS

Built into the package (ADR-0005). `tests/mqtt_tls.rs` connects to an
in-process rustls server with a certificate generated at test time:
verified CA passes, an unknown issuer is rejected, unverified mode accepts a
self-signed certificate. The test broker has no TLS listener; enabling TLS
against its plain port fails cleanly (`unexpected end of file`).

## Faults

Covered on the primary (see phase2-extended-metrics.md): closed broker
port, wrong password, TLS mismatch, invalid configuration. Covered by unit
tests: helper timeout (`exec::kills_on_timeout`), oversized helper output
(`exec::truncates_large_output`), missing sources for every group
(`mwan3`, `ha-cluster`, `luci-rpc`, `luci.wireguard`, `iwinfo`, package
manager), mwan3's 999999 ms latency, counter resets, missing PUBACKs, broker
refusal, truncated and oversized MQTT packets and 20 000 pseudo-random
packets for the decoder.

## Resources (primary, x86_64)

| Budget (PLAN.md 6.5) | Measured |
| --- | --- |
| Binary with TLS at most 3 MB | 1.9 MB (SDK build, dynamic musl); static musl builds: 0.91 MB without TLS, 1.83 MB with TLS |
| RSS at most 6 MB | 2.4 MB with the legacy set; 2.95 MB with all 164 metrics at 30 s |
| CPU below 0.5 % of one core at 30 s, all groups | 0.16 % over 3 minutes including `ubus`/`uci` helper processes (daemon alone 0.10 % over 10 minutes) |
| One TCP connection | one connection per router |

10-minute run with all 164 metrics at 30 s: 2936 messages, 0 publish
errors, RSS 2944-2952 kB, no restart (`logs/resource-all-metrics-30s.csv`).

## Secret handling (primary)

`grep -F -f /etc/mqtt-metrics/secret` found nothing in: syslog, the status
and state files, the UCI file, the command lines of all mqtt-metrics
processes, and the replies of rpcd `status`, `catalog`, `legacy`, `log` and
`preview`. The browser capture of phase 3 was clean as well. Files and the
control socket are mode 0600.

## Cutover preparation

`tools/cutover.sh <router> sandbox-test` applied the cron and `rc.local`
edits and the rollback to copies in `/tmp` on both routers:

| | Primary | Secondary |
| --- | --- | --- |
| crontab | 5 → 4 lines, no `publish_metrics` left | 4 → 3 lines |
| rc.local | 365 → 146 lines (installer block, lines 4-222), HA config-sync block and `exit 0` kept, `sh -n` ok | 224 → 5 lines, `exit 0` kept, `sh -n` ok |
| Rollback | byte-identical to the originals | byte-identical |
| Live files | checksums unchanged | unchanged |

## Soak

Started on both routers; see soak-2026-09-28.md.
