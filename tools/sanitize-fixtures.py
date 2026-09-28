#!/usr/bin/env python3
"""Turn raw router captures into sanitized test fixtures.

Usage: sanitize-fixtures.py <raw-root> <fixtures-root> <node> [<node> ...]

Each <node> is a directory name under <raw-root> produced by
tools/capture-fixtures.sh. One mapping is shared by all nodes, so an address
that appears on both routers maps to the same replacement.

Replacements:
- IPv4 addresses outside 0/8, 127/8, 224/3 -> TEST-NET-1/2/3;
- global and ULA IPv6 addresses -> 2001:db8::/32 and fd00:db8::/32,
  link-local stays link-local with a new interface identifier;
- MAC addresses -> 02:00:00:00:xx:yy;
- router hostnames -> router-<node>; DHCP client names -> client-N;
- WireGuard keys and peer names, VRRP auth data, search domains.

Files that could carry secrets are reduced to the fields the tests need.
The script ends with a leak check and exits non-zero if it finds anything.
"""

import ipaddress
import json
import re
import sys
from pathlib import Path

SKIP_FILES = {"network.sh"}
IPV4_RE = re.compile(r"(?<![\w.])(\d{1,3}(?:\.\d{1,3}){3})(?![\w.])")
IPV6_RE = re.compile(r"(?<![\w:])([0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7})(?![\w:])")
MAC_RE = re.compile(r"\b([0-9A-Fa-f]{2}(?::[0-9A-Fa-f]{2}){5})\b")
WG_KEY_RE = re.compile(r"\b[A-Za-z0-9+/]{43}=")


class Mapper:
    def __init__(self):
        self.v4 = {}
        self.v6 = {}
        self.mac = {}
        self.names = {}
        self.keys = {}

    def ipv4(self, text):
        try:
            addr = ipaddress.IPv4Address(text)
        except ValueError:
            return text
        first = int(str(addr).split(".")[0])
        if first == 0 or first == 127 or first >= 224:
            return text
        if text not in self.v4:
            n = len(self.v4)
            nets = ["192.0.2", "198.51.100", "203.0.113"]
            self.v4[text] = "%s.%d" % (nets[n // 250], n % 250 + 1)
        return self.v4[text]

    def ipv6(self, text):
        try:
            addr = ipaddress.IPv6Address(text)
        except ValueError:
            return text
        if addr.is_unspecified or addr.is_loopback or addr.is_multicast:
            return text
        if text not in self.v6:
            n = len(self.v6) + 1
            if addr.is_link_local:
                self.v6[text] = "fe80::%x" % n
            elif addr in ipaddress.IPv6Network("fc00::/7"):
                self.v6[text] = "fd00:db8:%x::1" % n
            else:
                # Keep network addresses (prefixes) recognisable.
                tail = "" if int(addr) & 0xFFFFFFFFFFFFFFFF == 0 else "1"
                self.v6[text] = "2001:db8:%x::%s" % (n, tail)
        return self.v6[text]

    def mac_addr(self, text):
        key = text.lower()
        if key not in self.mac:
            n = len(self.mac) + 1
            self.mac[key] = "02:00:00:00:%02x:%02x" % (n // 256, n % 256)
        return self.mac[key]

    def name(self, kind, text):
        key = (kind, text)
        if key not in self.names:
            n = sum(1 for k in self.names if k[0] == kind) + 1
            self.names[key] = "%s-%d" % (kind, n)
        return self.names[key]

    def wg_key(self, text):
        if text not in self.keys:
            n = len(self.keys) + 1
            body = ("TESTKEY%02d" % n).ljust(43, "A")
            self.keys[text] = body + "="
        return self.keys[text]

    def text(self, value):
        value = WG_KEY_RE.sub(lambda m: self.wg_key(m.group(0)), value)
        value = MAC_RE.sub(lambda m: self.mac_addr(m.group(1)), value)
        value = IPV4_RE.sub(lambda m: self.ipv4(m.group(1)), value)
        value = IPV6_RE.sub(lambda m: self.ipv6(m.group(1)), value)
        return value


def walk(value, fn):
    if isinstance(value, dict):
        return {k: walk(v, fn) for k, v in value.items()}
    if isinstance(value, list):
        return [walk(v, fn) for v in value]
    if isinstance(value, str):
        return fn(value)
    return value


def reduce_json(name, doc, mapper):
    if name == "keepalived_dump.json":
        for item in doc.get("status", []):
            data = item.get("data", {})
            for secret in ("auth_data", "script_master", "script_backup", "script_fault", "track_script"):
                if secret in data:
                    data[secret] = "redacted"
    elif name == "service_list.json":
        doc = {
            svc: {
                "instances": {
                    inst: {"running": bool(info.get("running"))}
                    for inst, info in (body.get("instances") or {}).items()
                }
            }
            for svc, body in doc.items()
        }
    elif name == "dhcp_leases.json":
        for key in ("dhcp_leases", "dhcp6_leases"):
            for lease in doc.get(key) or []:
                if lease.get("hostname"):
                    lease["hostname"] = mapper.name("client", lease["hostname"])
                if lease.get("duid"):
                    lease["duid"] = mapper.name("duid", lease["duid"])
    elif name == "wireguard.json":
        for iface in doc.values():
            for peer in iface.get("peers", []):
                peer["name"] = mapper.name("peer", peer.get("name", ""))
    elif name == "network_interface_dump.json":
        for iface in doc.get("interface", []):
            if iface.get("dns-search"):
                iface["dns-search"] = ["isp.example" for _ in iface["dns-search"]]
            iface.pop("data", None)
    return doc


def reduce_uci_network(text):
    keep = []
    for line in text.splitlines():
        if re.match(r"^network\.[^.=]+=interface$", line) or re.match(
            r"^network\.[^.=]+\.(proto|device|ip6addr|ip6gw|gateway|ipv6)=", line
        ):
            keep.append(line)
    return "\n".join(keep) + "\n"


def main():
    if len(sys.argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    raw_root = Path(sys.argv[1])
    out_root = Path(sys.argv[2])
    nodes = sys.argv[3:]
    mapper = Mapper()
    hostnames = {}
    for node in nodes:
        hostnames[node] = (raw_root / node / "hostname").read_text().strip()

    def host_fix(value):
        for node, host in hostnames.items():
            value = value.replace(host, "router-" + node)
        return value

    written = []
    for node in nodes:
        src = raw_root / node
        dst = out_root / node
        dst.mkdir(parents=True, exist_ok=True)
        for path in sorted(src.iterdir()):
            if path.name in SKIP_FILES or not path.is_file():
                continue
            text = path.read_text()
            if path.name == "uci_network":
                text = reduce_uci_network(text)
            if path.suffix == ".json":
                doc = json.loads(text) if text.strip() else None
                if doc is None:
                    continue
                doc = reduce_json(path.name, doc, mapper)
                doc = walk(doc, lambda s: mapper.text(host_fix(s)))
                text = json.dumps(doc, indent="\t", ensure_ascii=False) + "\n"
            else:
                text = mapper.text(host_fix(text))
            (dst / path.name).write_text(text)
            written.append(dst / path.name)

    leaks = []
    for path in written:
        text = path.read_text()
        for host in hostnames.values():
            if host in text:
                leaks.append("%s: hostname" % path)
        for match in IPV4_RE.finditer(text):
            addr = match.group(1)
            first = int(addr.split(".")[0])
            if first in (0, 127) or first >= 224:
                continue
            if not addr.startswith(("192.0.2.", "198.51.100.", "203.0.113.")):
                leaks.append("%s: %s" % (path, addr))
        for match in IPV6_RE.finditer(text):
            try:
                addr = ipaddress.IPv6Address(match.group(1))
            except ValueError:
                continue
            if addr.is_link_local or addr.is_unspecified or addr.is_loopback or addr.is_multicast:
                continue
            if not (addr in ipaddress.IPv6Network("2001:db8::/32") or addr in ipaddress.IPv6Network("fd00:db8::/32")):
                leaks.append("%s: %s" % (path, addr))
        for match in WG_KEY_RE.finditer(text):
            if not match.group(0).startswith("TESTKEY"):
                leaks.append("%s: key" % path)
    for leak in leaks:
        print("LEAK " + leak, file=sys.stderr)
    print("wrote %d files, %d leaks" % (len(written), len(leaks)))
    return 1 if leaks else 0


if __name__ == "__main__":
    sys.exit(main())
