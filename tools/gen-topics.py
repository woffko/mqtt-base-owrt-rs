#!/usr/bin/env python3
"""Regenerate the metric reference table in docs/TOPICS.md from
`mqtt-metricsd --templates` output (read from stdin)."""
import json
import sys

BEGIN = "<!-- BEGIN GENERATED TEMPLATES -->"
END = "<!-- END GENERATED TEMPLATES -->"
GROUPS = ["system", "packages", "interface", "traffic", "mwan3", "ha", "dhcp", "wireguard", "wifi", "service"]


def main():
    rows = [json.loads(l) for l in sys.stdin if l.strip()]
    out = [BEGIN, ""]
    for g in GROUPS:
        items = [r for r in rows if r["group"] == g]
        if not items:
            continue
        out.append("### Group `%s`" % g)
        out.append("")
        out.append("| Template | Object id | State topic suffix | Name | Type | Unit / class | Default |")
        out.append("| --- | --- | --- | --- | --- | --- | --- |")
        for r in items:
            cls = ", ".join(x for x in [r["unit"], r["device_class"], r["state_class"]] if x) or "-"
            flags = "on (legacy)" if r["legacy"] else ("recommended" if r["recommended"] else "off")
            if r["diagnostic"]:
                flags += ", diagnostic"
            if r["default_interval"]:
                flags += ", every %d s" % r["default_interval"]
            out.append("| `%s` | `%s` | `%s` | %s | %s | %s | %s |" % (
                r["id"], r["object"], r["topic"], r["name"], r["component"], cls, flags))
        out.append("")
    out.append(END)
    path = sys.argv[1]
    text = open(path, encoding="utf-8").read()
    start, stop = text.index(BEGIN), text.index(END) + len(END)
    open(path, "w", encoding="utf-8").write(text[:start] + "\n".join(out) + text[stop:])


if __name__ == "__main__":
    main()
