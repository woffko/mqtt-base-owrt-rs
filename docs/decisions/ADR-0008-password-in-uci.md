# ADR-0008: Broker password in UCI

Status: accepted, 2026-10-03. Supersedes ADR-0004.

## Context

ADR-0004 kept the broker password in `/etc/mqtt-metrics/secret`, outside
UCI, because an rpcd ACL grants a whole configuration to LuCI and the
password would reach the browser. That protects the password from LuCI
sessions that should not see it. On OpenWrt only root logs in to LuCI: rpcd
has one `login` section, for `root`, with read and write access to
everything, and root can read the file anyway. Every other OpenWrt password
(PPPoE, Wi-Fi keys, ddns, WireGuard private keys) is a UCI option shown in a
concealed field.

The separate file had a cost: a write-only rpcd method (`set_password`), a
"Remove stored password" switch, a keep.d entry, and a connection test that
had to substitute the stored password on the router and keep it from being
sent to another host (a defect the audit of 2026-09-29 found).

## Decision

- The password is `broker.password` in `/etc/config/mqtt-metrics`. The file
  stays mode 0600; the init script enforces it.
- LuCI shows it in a concealed password field; the connection test sends the
  password in the form.
- The daemon keeps it in a type whose debug output is `<set>`. It never
  appears in the log, `status.json`, the rpcd `status`, `catalog` and
  `preview` replies, or a configuration error message.
- The package's postinst moves an existing `/etc/mqtt-metrics/secret` into
  UCI once (through `uci batch` on stdin, so not on a command line) and
  deletes the file.

## Consequences

- The rpcd method `set_password`, the secret file, its keep.d entry and the
  stored-password logic of the connection test are gone.
- A backup of `/etc/config` contains the password, as it does for every other
  OpenWrt password.
- `mosquitto_sub` still takes a password only as an argument, so the
  development tool `tools/retained.sh` passes it through a temporary 0600
  options file.
