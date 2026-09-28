# Migration from publish_metrics.sh

This procedure replaces the legacy MQTT script with `mqtt-metrics` on one
router. Repeat it per router; on an HA pair do the primary first. Every step
is reversible until the backups are deleted.

The legacy setup consists of:

- a one-time installer block in `/etc/rc.local` that writes
  `/usr/bin/publish_metrics.sh`, adds a cron line and creates the marker
  `/etc/.publish_metrics_installed`;
- the cron line `*/5 * * * * /usr/bin/publish_metrics.sh` in
  `/etc/crontabs/root`.

`mqtt-metrics` never edits these files. The LuCI page
**Services → MQTT Metrics → Diagnostics** shows which parts are present.

## Scripted procedure

`tools/cutover.sh <router> <step>` runs the steps below over SSH (password in
`SSHPASS`):

| Step | Changes live files | What it does |
| --- | --- | --- |
| `sandbox-test` | no | applies the cron and `rc.local` edits and the rollback to copies in `/tmp`, checks syntax and that the rollback is byte-identical |
| `backup` | no (writes `/root/mqtt-metrics-migration`) | copies `rc.local`, the crontab, the legacy script and the UCI config, with checksums |
| `switch` | yes | production topics with discovery on, removes the cron line |
| `rclocal` | yes | removes the installer block from `/etc/rc.local` |
| `verify` | no | service state, owned topics, leftovers |
| `rollback` | yes | stops and disables mqtt-metrics, restores crontab, `rc.local` and the script |

The manual commands follow.

## 1. Install and run in shadow mode

Shadow mode publishes states under a separate prefix and announces nothing
to Home Assistant, so the legacy script keeps working unchanged.

```sh
apk add --allow-untrusted /tmp/mqtt-metrics-*.apk /tmp/luci-app-mqtt-metrics-*.apk
mqtt-metricsd --import-legacy                 # broker host, port, user and password
uci set mqtt-metrics.identity.base_topic='openwrt-shadow'
uci set mqtt-metrics.identity.discovery='0'
uci commit mqtt-metrics
/etc/init.d/mqtt-metrics reload               # a CLI commit does not notify procd
```

`--import-legacy` copies the password from `/usr/bin/publish_metrics.sh` into
`/etc/mqtt-metrics/secret` (mode 0600) without printing it. The same action
is available on the Broker page.

Check the result:

```sh
ubus call mqtt-metrics status                 # "state": "connected"
mosquitto_sub -h <broker> -u <user> -P <password> -v -t 'openwrt-shadow/#' -W 5
```

Compare the shadow states with the legacy states from a workstation:

```sh
tools/parity-check.py --legacy legacy.txt --shadow shadow.txt \
    --shadow-prefix openwrt-shadow --host <hostname>
```

`legacy.txt` and `shadow.txt` are `mosquitto_sub -v` captures of
`openwrt/#` and `openwrt-shadow/#`. The only expected differences are the
documented fixes in [TOPICS.md](TOPICS.md) and changing values such as
`system/uptime`.

## 2. Back up

```sh
mkdir -p /root/mqtt-metrics-migration
cp -p /etc/rc.local /etc/crontabs/root /usr/bin/publish_metrics.sh \
      /root/mqtt-metrics-migration/
ls -l /root/mqtt-metrics-migration
```

## 3. Switch over

Stop the legacy script first, so it cannot re-announce the entities that
the daemon removes on its first production pass:

```sh
sed -i '\|/usr/bin/publish_metrics.sh|d' /etc/crontabs/root
/etc/init.d/cron restart
while pgrep -f publish_metrics.sh >/dev/null; do sleep 1; done
uci set mqtt-metrics.identity.base_topic='openwrt'
uci set mqtt-metrics.identity.discovery='1'
uci commit mqtt-metrics
/etc/init.d/mqtt-metrics reload
```

The metric selection and intervals stay as configured. A fresh install
publishes only the legacy set every 300 s; anything switched on in LuCI
(for example the recommended set) appears as new entities in Home
Assistant.

On its first pass with discovery on, the daemon adopts the legacy topics of
this router: entities that can never have a value are removed, all others
keep their `unique_id` and history. The retained `openwrt-shadow/...` states
are cleared at the same time.

## 4. Remove the installer block from rc.local

As long as the block stays in `/etc/rc.local`, a missing marker file (for
example after a sysupgrade that does not keep it) reinstalls the script and
the cron line. Delete the lines from `# OpenWrt MQTT metrics one-time
installer` through the matching `fi`, and leave every other block in the
file unchanged. Check the result with:

```sh
grep -n publish_metrics /etc/rc.local          # no output
sh -n /etc/rc.local && echo syntax-ok
```

Optionally remove `/usr/bin/publish_metrics.sh` and
`/etc/.publish_metrics_installed`.

## 5. Verify

- `ubus call mqtt-metrics status` reports `connected` and no errors.
- Retained discovery configs under `homeassistant/+/openwrt_<host>_*/config`
  contain `availability_topic`.
- Stopping the service turns the availability topic to `offline`:
  `/etc/init.d/mqtt-metrics stop`, check, then `/etc/init.d/mqtt-metrics start`.
- In Home Assistant the existing entities keep their history and show as
  available.

## Rollback

```sh
/etc/init.d/mqtt-metrics stop
/etc/init.d/mqtt-metrics disable
cp -p /root/mqtt-metrics-migration/root /etc/crontabs/root
cp -p /root/mqtt-metrics-migration/rc.local /etc/rc.local
cp -p /root/mqtt-metrics-migration/publish_metrics.sh /usr/bin/publish_metrics.sh
/etc/init.d/cron restart
```

The legacy script republishes its discovery configs and states within five
minutes. To remove the package as well:

```sh
mqtt-metricsd --purge        # optional: clear the retained topics it owns
apk del luci-app-mqtt-metrics mqtt-metrics
```

`--purge` removes the entities announced by `mqtt-metrics`; the legacy
script recreates its own on its next run.
