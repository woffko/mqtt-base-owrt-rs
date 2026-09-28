#!/usr/bin/env python3
"""Compare the legacy script's retained topics with mqtt-metrics output.

Usage:
  parity-check.py --legacy LEGACY.txt --host HOSTNAME (--dump DUMP.jsonl | --shadow SHADOW.txt --shadow-prefix PREFIX)

LEGACY.txt and SHADOW.txt are `mosquitto_sub -v` captures ("topic payload"
per line). DUMP.jsonl is the output of `mqtt-metricsd --dump`, which is what
the daemon would publish with the legacy-compatible defaults. In shadow mode
only state topics are compared, after mapping PREFIX/<host> to
openwrt/<host>.

Allowed differences (documented in docs/TOPICS.md):
- legacy entities that can never have a value are cleared by mqtt-metrics;
- discovery payloads gain fields; every legacy field must keep its value;
- system/uptime and values listed with --volatile may change between
  captures.
Exit status 0 means no unexpected difference.
"""

import argparse
import json
import sys

REMOVABLE = ("ipv4", "ipv6", "gateway4", "gateway6")


def read_retained(path, host):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if " " not in line:
                topic, payload = line, ""
            else:
                topic, payload = line.split(" ", 1)
            if host in topic:
                out[topic] = payload
    return out


def read_dump(path):
    published, cleared = {}, set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            item = json.loads(line)
            if item["payload"]:
                published[item["topic"]] = item["payload"]
            else:
                cleared.add(item["topic"])
    return published, cleared


def removable(topic):
    last = topic.rstrip("/").split("/")
    field = last[-2] if last[-1] == "config" else last[-1]
    return any(field.endswith(f) for f in REMOVABLE)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--legacy", required=True)
    ap.add_argument("--host", required=True)
    ap.add_argument("--dump")
    ap.add_argument("--shadow")
    ap.add_argument("--shadow-prefix", default="openwrt-shadow")
    ap.add_argument("--volatile", action="append", default=["/system/uptime"])
    args = ap.parse_args()

    legacy = read_retained(args.legacy, args.host)
    problems, notes = [], []
    if args.dump:
        new, cleared = read_dump(args.dump)
    elif args.shadow:
        raw = read_retained(args.shadow, args.host)
        prefix = args.shadow_prefix + "/"
        new = {"openwrt/" + t[len(prefix):]: p for t, p in raw.items() if t.startswith(prefix)}
        new.pop("openwrt/%s/availability" % args.host, None)
        legacy = {t: p for t, p in legacy.items() if not t.endswith("/config")}
        cleared = set()
    else:
        ap.error("--dump or --shadow is required")

    for topic, old in sorted(legacy.items()):
        cur = new.get(topic)
        if cur is None:
            if removable(topic) and (args.shadow or topic in cleared):
                notes.append("removed (documented): %s" % topic)
            else:
                problems.append("missing: %s" % topic)
            continue
        if topic.endswith("/config"):
            old_json, cur_json = json.loads(old), json.loads(cur)
            for key, value in old_json.items():
                if cur_json.get(key) != value:
                    problems.append("%s: %s %r != %r" % (topic, key, cur_json.get(key), value))
        elif cur != old:
            if any(v in topic for v in args.volatile):
                notes.append("volatile: %s %s -> %s" % (topic, old, cur))
            else:
                problems.append("%s: %r != legacy %r" % (topic, cur, old))
    for topic in sorted(set(new) - set(legacy)):
        if topic.endswith("/availability"):
            continue
        problems.append("extra: %s" % topic)

    for n in notes:
        print("note  " + n)
    for p in problems:
        print("DIFF  " + p)
    print("compared %d legacy topics: %d differences" % (len(legacy), len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
