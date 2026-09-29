# Phase 5 evidence: cutover (2026-09-29)

Confirmed by the owner in chat ("делаем переключение") for both routers.
Production configuration: the legacy defaults (legacy metric set, 300 s,
automatic WAN selection), following the owner's decision that defaults
match the legacy script. The recommended set used during the soak was
removed before the switch; it can be switched on in LuCI.

Steps per router with `tools/cutover.sh`: capture of the production
retained topics, `backup`, metric overrides removed, `switch`, comparison,
`rclocal`, availability check, `verify`.

| | Primary 10.0.77.1 | Secondary 10.0.77.2 |
| --- | --- | --- |
| Backup | `/root/mqtt-metrics-migration` with MD5SUMS | same |
| Legacy topics compared after the switch | 53, 0 differences | 46, 0 differences |
| Removed as documented | 8 impossible discovery configs, link-local `wan-wan/ipv6` state | 6 impossible configs; 2 more for the absent `wan_6` after r5 (below) |
| Discovery configs now | 22, with `availability_topic` | 22 |
| Shadow topics left | 0 | 0 |
| Cron entry | removed | removed |
| `/etc/rc.local` | installer block removed; current file equals the backup minus lines 4-222; HA config-sync block and the boot-time sync kept; `sh -n` ok | only the MQTT block existed; current file equals the backup minus lines 3-221 (header and `exit 0` remain) |
| Availability | `offline` after stop, `online` after start | same |
| Service at boot | enabled | enabled |
| Packages | mqtt-metrics r5, luci-app-mqtt-metrics r5 | same |
| ubus objects / LuCI `/ubus` | 57 / ok | 50 / ok |

The legacy script was stopped first (cron line removed, running instance
awaited) so it could not re-announce the removed entities after the
daemon's first production pass.

## Fix during the cutover (r5)

On the secondary the dynamic `wan_6` interface is absent from netifd while
`wan` is down. The collector did not apply the protocol rule to absent
interfaces, so `wan_6` IPv4 and IPv4 gateway were still announced (as the
legacy script did). r5 takes the protocol from UCI for absent interfaces;
after the upgrade both configs were removed.

## Not done

- A live rollback on production was not run: it would make every entity
  flap and the legacy script would re-create the removed entities. The
  rollback was verified byte for byte on copies (`sandbox-test`) and the
  backups are in place.
- `/usr/bin/publish_metrics.sh` and `/etc/.publish_metrics_installed` are
  still on both routers (optional removal, docs/MIGRATION.md step 4).
- Home Assistant itself is checked by the owner.

## Follow-up (2026-09-29)

- The owner checked Home Assistant after the cutover: OK.
- On the owner's instruction the backups in `/root/mqtt-metrics-migration`
  were deleted on both routers (their `rc.local` copies contained plain-text
  passwords). `tools/cutover.sh rollback` needs a backup and no longer works
  on these routers; a rollback would mean restoring the legacy script from
  `/usr/bin/publish_metrics.sh`, which is still installed, and re-adding the
  cron line by hand.
- The soak monitor files in `/tmp/mqtt-metrics-soak` were removed; the
  samples are kept locally under `logs/`.
