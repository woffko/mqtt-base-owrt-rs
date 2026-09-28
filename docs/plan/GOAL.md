# Goal text

Objective for the implementation Goal. Use everything between the two lines
as the Goal text in an agent session started from `/home/w0w/mqtt-base-owrt-rs`.
Give the router password in the first chat message, not in the Goal text.

---

Build and deliver the OpenWrt 25.12 packages `mqtt-metrics` (Rust daemon,
`/usr/sbin/mqtt-metricsd`) and `luci-app-mqtt-metrics` (full LuCI app) in the
repository /home/w0w/mqtt-base-owrt-rs, replacing the legacy
`/usr/bin/publish_metrics.sh` cron script installed from `/etc/rc.local` on the
HA router pair 10.0.77.1 (primary) and 10.0.77.2 (secondary). Follow
`AGENTS.md`, the skills in `.agents/skills/` (`openwrt-package`, `rust`,
`documentation-text-writing`), and `docs/plan/PLAN.md` (v2, owner decisions in
section 11) as the design source of truth; record deviations as ADRs in
`docs/decisions/`. Execute the phases in PLAN.md section 9 in order and commit
each finished phase to `main`. Do not create the GitHub repository, push, or
publish without the owner's explicit OK; when approved, the repository is
`mqtt-base-owrt-rs` and private.

Acceptance criteria:
1. Both packages build as x86_64 `.apk` files in the 25.12.5 SDK
   (/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64)
   via package overlay, without cleaning or altering other packages in that
   SDK. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test`, and the LuCI tests pass. Other architectures are out of
   scope.
2. Defaults after install reproduce the legacy script: same metrics,
   interface selection, state topics, HA `unique_id`s, names, and 300 s
   interval; plus Last Will availability on one persistent MQTT connection.
   Empty retained values and link-local IPv6 as the interface address are
   never published. Discovery is sent on connect, on config change, and on
   HA birth only.
3. The integration is read-only: no command entities or command topics; the
   only subscription is the HA birth topic; a test proves that any other
   incoming message is dropped.
4. The extended metric groups of PLAN.md section 4 are implemented as opt-in
   metrics with a "Recommended set" action; sources absent on a router are
   reported as unavailable with a reason.
5. The LuCI app (Status, Broker, Topics and device, Metrics, Diagnostics)
   makes intervals (global, per group, per metric, change-only mode), topic
   prefixes, device ID and name, per-metric object ID, topic, name and icon
   configurable, with a warning and old-topic cleanup on renames and a
   "Reset to legacy defaults" action. Each metric's publishing and HA
   visibility can be turned on or off (hiding removes the HA entity);
   interfaces, devices and services can be selected; test connection,
   republish and purge work. Every setting round-trips through UCI. Desktop
   and mobile screenshots are saved.
6. Secrets: the MQTT password never appears in the repository, argv, logs,
   rpcd replies, LuCI page source, or the agent's output;
   `/etc/config/mqtt-metrics` is mode 0600; broker settings are copied on the
   router from the legacy script with a command that never prints them.
   Fixtures are sanitized.
7. On 10.0.77.1 in shadow mode (separate prefix, discovery off) the parity
   check matches the legacy output except for documented fixes. All metric
   groups are verified on both routers (MASTER and BACKUP roles). Binary
   size, RSS, and CPU are measured against PLAN.md section 6.5.
8. Soak: the final candidate runs in shadow mode on both routers with
   `tools/soak-monitor.sh`; the start time and the next check time
   (start + 24 h, router local time and time zone) are written to
   `docs/evidence/soak-<date>.md`. After finishing router-free work, stop and
   report the next check time. When resumed after that time, evaluate the
   soak: no restarts, errors, or memory growth.
9. Cutover (PLAN.md Phase 5) is prepared for each router with backups, exact
   commands, and a tested rollback. Performing the cutover and editing
   `/etc/rc.local` or `/etc/crontabs/root` wait for the owner's explicit
   confirmation in chat, per router. The agent verifies the MQTT side; the
   owner verifies Home Assistant. The Goal may complete with the cutover
   ready but not executed if confirmation has not been given.
10. README, docs/USER_GUIDE.md, docs/TOPICS.md, and docs/MIGRATION.md are
    written in English.

Constraints: both routers carry real traffic and form the HA pair. Only
install/remove this project's packages there; never restart the firewall,
force a WAN failover, signal ubusd, or reboot. Do not reinstall the package
during the soak. After each install check `ubus list | wc -l`, LuCI `/ubus`,
and `logread`. Use `sshpass -e` with `SSHPASS`; never write the password
anywhere.

---
