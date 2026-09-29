# User guide

`mqtt-metrics` publishes router state to an MQTT broker and announces it to
Home Assistant. Everything is configured under
**Services → MQTT Metrics** in LuCI or in `/etc/config/mqtt-metrics`.

After installation the service runs but stays idle until a broker host is
set. The defaults then reproduce the legacy `publish_metrics.sh` script: the
same metrics, topics, entity ids and names, published every 300 seconds.

## Status

Shows the connection (state, broker, last connect, last publish, message and
error counters, last error), the device identity (device id and name, base
topic, availability topic, client id, whether discovery is on) and a table
of everything currently published with its last value, topic and time.

- **Test connection** connects to the saved broker with a separate client id
  and disconnects. Nothing is published.
- **Republish discovery** resends every discovery config and state.

The page refreshes every 5 seconds.

## Broker

| Field | Notes |
| --- | --- |
| Enabled | Off stops the service. |
| Log level | Errors, warnings, information (default) or debug. The service logs to syslog as `mqtt-metrics`. |
| Host, Port | Broker address; the port defaults to 1883. |
| User name, Password | The password is stored in `/etc/mqtt-metrics/secret` (root only) and is never shown again. Leave the field empty to keep the stored password; tick **Remove stored password** to delete it. |
| Client ID | Must be unique on the broker. Default `mqtt-metrics-<hostname>`. |
| Keepalive | Seconds between keepalive packets, 5-3600 (default 60). |
| QoS for states, Retain states | Defaults 1 and on, as in the legacy script. Discovery always uses QoS 1 and retain. |
| Use TLS | Encrypts the connection. **CA certificate file** defaults to the system bundle `/etc/ssl/certs/ca-certificates.crt` (package `ca-bundle`). Turning off **Verify the broker certificate** accepts self-signed certificates without authenticating the broker. A client certificate and key can be set for certificate authentication. |

**Connection test** uses the values on the page, including unsaved changes,
so settings can be checked before saving.

When the legacy script is installed, the **Legacy script** section shows its
broker and **Import broker settings** copies host, port, user and password.
The password is copied on the router and not shown.

## Topics and device

| Field | Default | Notes |
| --- | --- | --- |
| Base topic | `openwrt` | State topics are `<base topic>/<node>/<suffix>`. |
| Node | hostname | One topic level. |
| MQTT discovery | on | Off is shadow mode: states are published, nothing is announced, and entities announced earlier are removed. |
| Discovery prefix | `homeassistant` | |
| Resend discovery when Home Assistant restarts | on | Subscribes to the birth topic. Turn it off if the broker does not allow the subscription; the Status page shows a warning when the broker refuses it. |
| Birth topic | `homeassistant/status` | The only topic the service subscribes to. An `online` message makes it resend discovery. |
| Device ID | `openwrt_<hostname>` | Prefix of every `unique_id`. |
| Device name, Manufacturer, Model | `OpenWrt <hostname>`, `OpenWrt`, `Router` | Shown on the Home Assistant device page. |
| Configuration URL | empty | Link on the Home Assistant device page. |
| Remove legacy leftovers | on | See below. |

Changing the base topic, node, discovery prefix or device ID creates new
entities in Home Assistant; their history does not carry over. The page
shows a warning as soon as one of these fields changes. The old entities and
retained topics are removed automatically.

**Remove legacy leftovers**: on the first pass with discovery on, the
service adopts the legacy script's topics for this router and clears every
one of them that this configuration no longer publishes: the entities that
can never have a value (the IPv4 address of a DHCPv6 interface, the IPv6
address of a PPPoE interface) and, when the device ID, base topic or
discovery prefix differs from the legacy layout, all the old entities. While
the legacy script is still scheduled in `/etc/crontabs/root` the service
leaves its topics alone, because the script would publish them again; the
adoption happens on the first pass after the cron line is removed.

## Metrics

### Publishing

- **Global interval**: seconds between publications (5-86400, default 300).
- **Publish only changes**: values are still read at their interval but
  sent only when they change, and at least once per **Maximum age**.

A group is read as often as its shortest interval. A metric with a longer
interval is published at the first read after that interval has passed, so it
is effectively rounded up to a multiple of the group's shortest interval (with
60 s and 90 s in one group, the 90 s metric is published every 120 s).

### Selection

- **Interfaces**: automatic selects the `wan*` and `wwan*` interfaces, like
  the legacy script; manual selects any interfaces. Interfaces matched by
  the legacy rule (marked `*`) keep the legacy entity ids, others are named
  `iface_<name>_...`.
- **Traffic devices**: devices for the traffic group. None selected means the
  devices of the selected interfaces.
- **Services**: procd services that get a "running" entity.

Metrics of newly selected instances appear after **Save & Apply**.

### Groups and metrics

Each group (System, Network interfaces, Traffic, Multi-WAN, HA cluster,
DHCP, WireGuard, Wi-Fi, Services, Packages) can be collapsed and has:

- **Collect this group**: off stops reading the source entirely;
- **Group interval**: overrides the global interval for the group;
- **Publish all / Publish none**;
- the list of metrics that this router cannot provide, with the reason.

Per metric:

| Column | Meaning |
| --- | --- |
| Publish | Collect and publish the value. Off removes the entity from Home Assistant and clears the retained state. |
| HA | Announce the entity to Home Assistant. Off keeps publishing the state (for other consumers) and removes the entity. |
| Name | Entity name; empty uses the default shown as placeholder. |
| Value | Current value, or the reason why there is none. |
| Interval | Per-metric interval; empty uses the group or global interval. |
| … | Object id, topic suffix, icon, "enabled by default in HA", "diagnostic entity", the resulting `unique_id` and state topic, and a discovery preview. |

**Recommended set** switches on a useful selection (load, CPU, memory,
storage, conntrack, interface details, traffic of the WAN devices, mwan3,
HA cluster, DHCP). **Reset to legacy defaults** removes every override,
group setting and selection. **Filter** and **Only published** narrow the
list. Nothing changes on the router until **Save & Apply**.

## Diagnostics

- **Legacy publish_metrics.sh**: whether the script, cron entry,
  `/etc/rc.local` block and marker file are present, with the migration
  steps. The page does not change those files.
- **Actions**: reload configuration, republish discovery and states, and
  **Reset retained topics**, which clears every retained topic this router
  owns on the broker and publishes everything again.
- **Discovery preview**: the exact discovery message of any metric.
- **Service log**: the last lines logged by the service.

## Command line

```sh
ubus call mqtt-metrics status          # same data as the Status page
mqtt-metricsd --check-config           # validate /etc/config/mqtt-metrics
mqtt-metricsd --dump                   # print one full publish pass (nothing is sent)
mqtt-metricsd --catalog                # every metric with value and settings
mqtt-metricsd --import-legacy          # copy broker settings from publish_metrics.sh
mqtt-metricsd --purge                  # clear every retained topic this router owns
/etc/init.d/mqtt-metrics reload        # apply a configuration committed with uci
```

`--purge` asks a running service to clear its topics and publish everything
again; without a running service it connects once and only clears.

Only one `mqtt-metricsd` can run at a time (a lock in
`/var/run/mqtt-metrics/daemon.lock`). After a lost connection the service
waits 5 s, 10 s, ... up to 120 s before reconnecting; a connection that held
for a minute starts over from 5 s, and a configuration change reconnects at
once.

`uci commit` from the command line does not notify procd; run
`/etc/init.d/mqtt-metrics reload` afterwards. LuCI does this automatically.

## Files

| Path | Content |
| --- | --- |
| `/etc/config/mqtt-metrics` | configuration (mode 0600) |
| `/etc/mqtt-metrics/secret` | broker password (mode 0600) |
| `/etc/mqtt-metrics/state.json` | retained topics owned on the broker; written only when that set changes |
| `/var/run/mqtt-metrics/status.json` | runtime status (tmpfs) |
| `/var/run/mqtt-metrics/control.sock` | local control socket for LuCI (root only) |

`/etc/mqtt-metrics/secret` and `state.json` are kept across sysupgrade.
