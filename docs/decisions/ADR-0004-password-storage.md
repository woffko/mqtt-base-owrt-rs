# ADR-0004: Broker password outside UCI

Status: accepted, 2026-09-28. Deviation from PLAN.md sections 6.2 and 11,
which assumed the password lives in `/etc/config/mqtt-metrics`.

## Context

LuCI reads UCI configurations through the rpcd `uci` object. An ACL grants
access to a whole configuration, not to single options, so any option in
`/etc/config/mqtt-metrics` reaches the browser. The requirement is that the
password never appears in LuCI page data or rpcd replies.

## Decision

- The password is stored in `/etc/mqtt-metrics/secret` (mode 0600, directory
  0700), outside UCI.
- LuCI can only replace it through the rpcd write method `set_password`; no
  method returns it. `catalog` reports only `password_set`.
- `import_legacy` copies the legacy script's password into the same file on
  the router without printing it.
- `/etc/config/mqtt-metrics` is still mode 0600; the init script enforces
  the modes on every start.
- The file is kept across sysupgrade through
  `/lib/upgrade/keep.d/mqtt-metrics`.

## Consequences

- A configuration backup of `/etc/config` alone does not contain the
  password; sysupgrade keeps it through the keep.d list.
