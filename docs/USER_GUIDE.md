# User guide

`mqtt-metrics` publishes router state to an MQTT broker and announces it to
Home Assistant. Everything is configured under
**Services → MQTT Metrics** in LuCI or in `/etc/config/mqtt-metrics`.

After installation the service runs but stays idle until a broker host is
set. It then publishes the default set every 300 seconds: the system
identity and uptime, and the state and addresses of the WAN interfaces.

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
| QoS for states, Retain states | Defaults 1 and on. Discovery always uses QoS 1 and retain. Turning retain off clears the retained states once. |
| Use TLS | Encrypts the connection. **CA certificate file** defaults to the system bundle `/etc/ssl/certs/ca-certificates.crt` (package `ca-bundle`). Turning off **Verify the broker certificate** accepts self-signed certificates without authenticating the broker. A client certificate and key can be set for certificate authentication. |

**Connection test** uses the values on the page, including unsaved changes,
so settings can be checked before saving.

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

The device ID (and an object ID changed on the Metrics page) is part of the
`unique_id` of an entity. Changing it creates new entities in Home Assistant;
their history does not carry over. Changing the base topic or node moves the
state topics: the entities are updated in place, but anything else that
reads the old topics must follow. Home Assistant has to listen on the
discovery prefix. The Topics page shows a warning as soon as one of these
fields changes, and the old retained topics are removed automatically.

## Metrics

### Publishing

- **Global interval**: seconds between publications (5-86400, default 300).
- **Publish only changes**: values are still read at their interval but
  sent only when they change, and at least once per **Maximum age**.

Every metric is published at its own interval. A group is read whenever its
earliest metric is due; the other metrics of the group wait for their turn
(with 60 s and 90 s in one group, the group is read at 60 s, 90 s, 120 s and
so on, and each metric is published at its own multiples).

A group is read only when something in it is published: a metric that is on
by default, or one that you switched on. Groups you do not use cost nothing
(no `apk` run, no `keepalived` query). A group whose source is missing or
fails at a read (`mwan3` not running yet, `ubus` answering with an error) is
read again at its interval, so its metrics appear as soon as the source does.
Values published earlier stay in place while a read fails.

### Selection

- **Interfaces**: automatic selects the `wan*` and `wwan*` interfaces;
  manual selects any interfaces. The entities of interfaces matched by the
  automatic rule (marked `*`) are named `wan_<name>_...`, others
  `iface_<name>_...`.
- **Traffic devices**: devices for the traffic group. None selected means the
  devices of the selected interfaces.
- **Services**: procd services that get a "running" entity. A selected
  service is published without further settings; its entity reads ON while
  every instance of the service runs.

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
HA cluster, DHCP). **Reset to defaults** removes every override,
group setting and selection. **Filter** and **Only published** narrow the
list. Nothing changes on the router until **Save & Apply**.

## Diagnostics

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

A configuration with an error (for example a metric name longer than 128
bytes set with `uci`) stops publishing until it is fixed. The Status page and
`mqtt-metricsd --check-config` list the errors, and the LuCI pages keep
showing the stored settings that could be read.

## Files

| Path | Content |
| --- | --- |
| `/etc/config/mqtt-metrics` | configuration (mode 0600) |
| `/etc/mqtt-metrics/secret` | broker password (mode 0600) |
| `/etc/mqtt-metrics/state.json` | retained topics owned on the broker; written only when that set changes |
| `/var/run/mqtt-metrics/status.json` | runtime status (tmpfs) |
| `/var/run/mqtt-metrics/control.sock` | local control socket for LuCI (root only) |

`/etc/mqtt-metrics/secret` and `state.json` are kept across sysupgrade.
