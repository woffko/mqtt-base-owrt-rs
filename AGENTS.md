# mqtt-base-owrt-rs - Agent Instructions

## Scope

This repository provides an OpenWrt 25.12 package feed with:

- `mqtt-metrics` (`package/mqtt-metrics`) - a Rust daemon,
  `/usr/sbin/mqtt-metricsd`, that publishes router metrics to MQTT with Home
  Assistant discovery; UCI config `/etc/config/mqtt-metrics`;
- `luci-app-mqtt-metrics` (`package/luci-app-mqtt-metrics`) - the LuCI
  application that configures the daemon and selects or hides published
  metrics.

The repository name is `mqtt-base-owrt-rs` (private on GitHub for now).

It replaces the legacy `/usr/bin/publish_metrics.sh` cron script that is
installed from `/etc/rc.local` on the test routers. The authoritative work
plan is `docs/plan/PLAN.md`; record deviations from it as ADRs under
`docs/decisions/`.

## Language

Use English only for all content intended for GitHub: source-code comments,
documentation, README files, commit messages, pull requests, issues, release
notes, and repository descriptions. Chat with the owner may be in Russian.

## Skills

Project skills live in `.agents/skills/` (`.claude/skills` is a symlink to
it):

- `openwrt-package` - package layout, Makefile, procd, UCI, rpcd, LuCI, SDK
  build and on-router acceptance rules. Apply it to every change under
  `package/`.
- `rust` - Rust rules. This is an I/O-bound daemon, not a compute kernel:
  apply the determinism, error-handling, dependency, unsafe, and style rules
  fully; treat the "hot path" rules as applying to the periodic collect and
  publish loop (no unbounded allocation growth, no per-tick re-validation of
  configuration that was validated at load time).
- `documentation-text-writing` - for README, docs, and release notes.

## Hard invariants

- No secret (MQTT password, TLS key, router password) may appear in the
  repository, argv, logs, `ubus` replies, LuCI page source, test fixtures, or
  delegated prompts. Fixtures captured from routers must be sanitized
  (TEST-NET addresses, documentation IPv6 prefix `2001:db8::/32`, fake
  hostnames).
- Package scripts must not edit `/etc/rc.local`, crontabs, or other packages'
  configuration. Migration from the legacy script is a documented, reversible
  manual step.
- A missing data source (no `mwan3`, no `keepalived`, no thermal sensor, no
  Wi-Fi) must make the metric unavailable, never publish a fake value.
- The integration is read-only: no command entities, no command topics, and
  no code path from an incoming MQTT message to an action on the router. The
  only subscription is the HA birth topic, which only triggers a discovery
  resend.
- Defaults after install reproduce the legacy script: same metrics, interface
  selection, topics, `unique_id`s, names and 300 s interval. Everything else
  is configurable in LuCI and new metrics are opt-in.

## Rust rules

- Do not introduce `unwrap()` or `expect()` outside tests. Propagate errors or
  handle them explicitly.
- `unsafe` is forbidden unless an ADR justifies it.
- `Cargo.lock` is committed; dependencies are pinned and licence-compatible
  (no GPL/LGPL/AGPL crates). Prefer `std`; every new crate needs a reason in
  the commit message.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test` must pass before a commit.

## Editing hygiene

- Do not introduce formatting-only changes or reflow unrelated code.
- Keep diffs limited to the task. Bump `PKG_RELEASE` whenever a package that
  will be installed on a router changes.

## Test equipment

- Test routers, both OpenWrt 25.12-SNAPSHOT x86_64 forming a
  keepalived/owsync HA pair and carrying real traffic:
  - primary `root@10.0.77.1` (HA role `MASTER`);
  - secondary `root@10.0.77.2` (HA role `BACKUP`), same credentials.
- The owner supplies the router password in chat. Never write it to files,
  memory, Git, logs, or delegated prompts; use `sshpass -e` with the
  `SSHPASS` environment variable.
- Broker credentials are copied on the router from
  `/usr/bin/publish_metrics.sh` with `mqtt-metricsd --import-legacy`: host,
  port and user go to `/etc/config/mqtt-metrics`, the password to
  `/etc/mqtt-metrics/secret` (ADR-0004). Nothing is printed.
- Read-only inspection is the default. Installing and removing this project's
  packages on both routers is authorized as part of the Goal. Changes to
  `/etc/rc.local`, `/etc/crontabs/root`, and the cutover from the legacy
  script require the owner's explicit confirmation in chat, per router.
- The owner checks Home Assistant itself; the agent verifies only the MQTT
  side (retained topics, availability).
- Never restart the firewall, force a WAN failover, signal `ubusd`, or reboot.
  After each install, check `ubus list | wc -l`, LuCI `/ubus`, and `logread`.
