# Broker password moved into UCI (2026-10-03)

Packages `mqtt-metrics-0.1.0-r9` and `luci-app-mqtt-metrics-0.1.0-r8`
(ADR-0008), installed on both routers after the owner's go-ahead, secondary
first.

## Checks before the installation

- `E2E=1 tools/check.sh`: all pass (95 unit tests, the topic compatibility
  test, 5 client and 4 TLS tests, Node tests, the LuCI tests on the stand-in
  and on the real LuCI code, 5 end-to-end scenarios). The real-LuCI test
  checks that the Broker page shows the UCI password concealed, sends it with
  the connection test and writes a changed one to UCI.
- The postinst migration was run on a router against a scratch UCI
  directory with fake passwords containing quotes, spaces, `$`, backticks,
  backslashes and double quotes: every value arrived unchanged and the old
  file was removed.

## Installation

| | Secondary 10.0.77.2 | Primary 10.0.77.1 |
| --- | --- | --- |
| `ubus` objects before / after | 50 / 50 | 57 / 57 |
| LuCI `/ubus`, rpcd object | ok, present, no `set_password` | ok, present |
| `/etc/mqtt-metrics/secret` | removed by the postinst | removed |
| Password in UCI | same checksum as the old file (compared on the router) | same |
| `/etc/config/mqtt-metrics` | mode 0600 | mode 0600 |
| Password in command lines, `status`, `catalog`, `preview`, log, `status.json` | none (checked with `grep -F -f` and a 0600 file) | none |
| Retained topics against the r8 capture | 17 states, 22 configs, none added or removed, no discovery config changed | 26 states, 25 configs, the same |
| After the first tick | connected, 0 reconnects, 0 publish errors, no warning | the same; HA role MASTER, state HEALTHY |

The capture "before" the secondary's installation came back empty: the new
`tools/retained.sh` already read the password from UCI, which did not have
it yet. The capture after the r8 installation of 2026-10-01 served as the
reference instead.

One check on the secondary briefly put the password on a command line
(`grep -F "$(uci get ...)"` while reading `/proc/*/cmdline`); it was repeated
with `grep -F -f` and a 0600 temporary file, which is how the remaining
checks were done.
