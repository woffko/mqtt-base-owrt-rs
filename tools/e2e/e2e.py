#!/usr/bin/env python3
"""Loopback smoke tests for the real mqtt-metricsd: a build whose path
constants point into a scratch directory, fake `uci` and `ubus` on PATH, and a
fake MQTT broker in this process. tools/e2e-smoke.sh prepares the directory
and calls this script.

usage: e2e.py SCENARIO   (lost_batch | birth_flood | config_retry |
                          disable_enable | late_netifd | all)

Environment: E2E_DIR (the prepared directory), E2E_BIN (the daemon binary).
"""
import json
import os
import signal
import socket
import subprocess
import sys
import threading
import time

E = os.environ["E2E_DIR"]
BIN = os.environ["E2E_BIN"]
HOST = "router-primary"
AVAIL = "openwrt/%s/availability" % HOST
T0 = time.monotonic()


def log(msg):
    print("[%6.1f] %s" % (time.monotonic() - T0, msg), flush=True)


class Broker:
    def __init__(self):
        self.lock = threading.Lock()
        self.retained = {}
        self.socks = []
        self.connects = 0
        self.avail_online = 0
        self.avail_offline = 0
        self.publishes = 0
        self.drop_after = None
        self.changed = time.monotonic()
        self.srv = socket.socket()
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", 0))
        self.srv.listen(5)
        self.port = self.srv.getsockname()[1]
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while True:
            try:
                s, _ = self.srv.accept()
            except OSError:
                return
            with self.lock:
                self.connects += 1
                self.socks.append(s)
            threading.Thread(target=self.serve, args=(s,), daemon=True).start()

    @staticmethod
    def read_exact(s, n):
        buf = b""
        while len(buf) < n:
            chunk = s.recv(n - len(buf))
            if not chunk:
                raise EOFError
            buf += chunk
        return buf

    def packet(self, s):
        first = self.read_exact(s, 1)[0]
        mult, length = 1, 0
        while True:
            b = self.read_exact(s, 1)[0]
            length += (b & 0x7F) * mult
            if not b & 0x80:
                break
            mult *= 128
        return first, (self.read_exact(s, length) if length else b"")

    def serve(self, s):
        will = None
        try:
            first, body = self.packet(s)
            assert first >> 4 == 1
            n = (body[0] << 8) | body[1]
            i = 2 + n
            flags = body[i + 1]
            i += 4
            n = (body[i] << 8) | body[i + 1]
            i += 2 + n
            if flags & 0x04:
                n = (body[i] << 8) | body[i + 1]
                wt = body[i + 2:i + 2 + n].decode()
                i += 2 + n
                n = (body[i] << 8) | body[i + 1]
                wp = body[i + 2:i + 2 + n]
                will = (wt, wp, bool(flags & 0x20))
            s.sendall(bytes([0x20, 2, 0, 0]))
            while True:
                first, body = self.packet(s)
                kind = first >> 4
                if kind == 3:
                    qos = (first >> 1) & 3
                    retain = bool(first & 1)
                    n = (body[0] << 8) | body[1]
                    topic = body[2:2 + n].decode()
                    j = 2 + n
                    pid = None
                    if qos:
                        pid = body[j:j + 2]
                        j += 2
                    payload = body[j:]
                    with self.lock:
                        self.publishes += 1
                        self.changed = time.monotonic()
                        if retain:
                            if payload:
                                self.retained[topic] = payload.decode(errors="replace")
                            else:
                                self.retained.pop(topic, None)
                        if topic == AVAIL:
                            if payload == b"online":
                                self.avail_online += 1
                            elif payload == b"offline":
                                self.avail_offline += 1
                        drop = False
                        if self.drop_after is not None:
                            self.drop_after -= 1
                            drop = self.drop_after <= 0
                            if drop:
                                self.drop_after = None
                    if drop:
                        log("broker: dropping the connection without acknowledging")
                        s.close()
                        return
                    if qos:
                        s.sendall(bytes([0x40, 2]) + pid)
                elif kind == 8:
                    pid = body[:2]
                    s.sendall(bytes([0x90, 3]) + pid + bytes([0]))
                elif kind == 12:
                    s.sendall(bytes([0xD0, 0]))
                elif kind == 14:
                    will = None
                    return
        except (EOFError, OSError, AssertionError):
            pass
        finally:
            if will and will[2]:
                with self.lock:
                    self.retained[will[0]] = will[1].decode()
            try:
                s.close()
            except OSError:
                pass

    def send_birth(self, count):
        topic = b"homeassistant/status"
        msg = bytes([0x30]) + bytes([2 + len(topic) + 6]) + len(topic).to_bytes(2, "big") + topic + b"online"
        with self.lock:
            socks = list(self.socks)
        for s in socks[-1:]:
            for _ in range(count):
                try:
                    s.sendall(msg)
                except OSError:
                    pass

    def snapshot(self):
        with self.lock:
            return dict(self.retained)

    def quiet_for(self, seconds):
        with self.lock:
            return time.monotonic() - self.changed >= seconds


def wait_for(cond, timeout, what):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.1)
    log("TIMEOUT waiting for " + what)
    return False


class Daemon:
    def __init__(self, port):
        self.port = port
        self.proc = None
        for d in ("etc", "run"):
            for f in os.listdir(os.path.join(E, d)):
                try:
                    os.remove(os.path.join(E, d, f))
                except OSError:
                    pass
        self.write_uci()

    def write_uci(self, **identity):
        lines = [
            "mqtt-metrics.main=main",
            "mqtt-metrics.main.enabled='%s'" % identity.pop("enabled", "1"),
            "mqtt-metrics.broker=broker",
            "mqtt-metrics.broker.host='127.0.0.1'",
            "mqtt-metrics.broker.port='%d'" % self.port,
            "mqtt-metrics.broker.keepalive='30'",
            "mqtt-metrics.identity=identity",
            "mqtt-metrics.identity.cleanup_legacy='0'",
            "mqtt-metrics.publish=publish",
            "mqtt-metrics.publish.interval='%s'" % os.environ.get("E2E_INTERVAL", "300"),
        ]
        for k, v in identity.items():
            lines.append("mqtt-metrics.identity.%s='%s'" % (k, v))
        with open(E + "/mqtt-metrics.uci", "w") as f:
            f.write("\n".join(lines) + "\n")

    def start(self):
        env = dict(os.environ)
        env["PATH"] = E + "/bin:" + env["PATH"]
        env["FIX"] = E + "/fix"
        env["E2E"] = E
        self.out = open(E + "/daemon.log", "w")
        self.proc = subprocess.Popen([BIN], env=env, stdout=self.out, stderr=self.out)

    def hup(self):
        self.proc.send_signal(signal.SIGHUP)

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def status(self):
        try:
            with open(E + "/run/status.json") as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    def state_topics(self):
        try:
            with open(E + "/etc/state.json") as f:
                return set(json.load(f).get("topics", []))
        except (OSError, ValueError):
            return set()


def scenario_lost_batch():
    b = Broker()
    d = Daemon(b.port)
    d.start()
    try:
        assert wait_for(lambda: len(b.snapshot()) >= 20 and b.quiet_for(1.5), 20, "first pass")
        old = {t for t in b.snapshot() if "openwrt_router-primary_" in t}
        log("first pass: %d retained topics, %d with the old id" % (len(b.snapshot()), len(old)))
        assert old, "expected discovery configs with the default device id"

        b.drop_after = 5
        d.write_uci(device_id="gw1")
        d.hup()
        assert wait_for(lambda: b.connects >= 2, 30, "reconnect after the dropped batch")
        assert wait_for(lambda: b.quiet_for(2.0) and any("gw1_" in t for t in b.snapshot()), 30, "second pass")
        snap = b.snapshot()
        left = sorted(t for t in snap if "openwrt_router-primary_" in t)
        ledger_old = sorted(t for t in d.state_topics() if "openwrt_router-primary_" in t)
        log("after the retry: %d retained topics, old-id topics on the broker: %d, in the ledger: %d"
            % (len(snap), len(left), len(ledger_old)))
        assert not left, "old topics were orphaned: %s" % left[:3]
        assert not ledger_old, "ledger still lists old topics"
        assert any("gw1_" in t for t in snap)
        log("lost_batch: ok")
    finally:
        d.stop()


def scenario_birth_flood():
    b = Broker()
    d = Daemon(b.port)
    d.start()
    try:
        assert wait_for(lambda: len(b.snapshot()) >= 20 and b.quiet_for(1.5), 20, "first pass")
        base = b.avail_online
        b.send_birth(25)
        time.sleep(4)
        first = b.avail_online - base
        log("passes 4 s after 25 births: %d" % first)
        assert first == 1, first
        assert wait_for(lambda: b.avail_online - base >= 2, 40, "the deferred pass")
        time.sleep(3)
        total = b.avail_online - base
        log("passes after the spacing: %d" % total)
        assert total == 2, total
        log("birth_flood: ok")
    finally:
        d.stop()


def scenario_config_retry():
    b = Broker()
    d = Daemon(b.port)
    open(E + "/uci.fail", "w").close()
    d.start()
    try:
        time.sleep(3)
        assert d.status().get("state") == "config_error", d.status().get("state")
        os.remove(E + "/uci.fail")
        t = time.monotonic()
        assert wait_for(lambda: b.connects >= 1, 30, "connect after the retry")
        log("connected %.1f s after uci recovered" % (time.monotonic() - t))
        assert time.monotonic() - t < 25
        log("config_retry: ok")
    finally:
        if os.path.exists(E + "/uci.fail"):
            os.remove(E + "/uci.fail")
        d.stop()


def scenario_disable_enable():
    b = Broker()
    d = Daemon(b.port)
    d.start()
    try:
        assert wait_for(lambda: len(b.snapshot()) >= 20 and b.quiet_for(1.5), 20, "first pass")
        d.write_uci(enabled="0")
        d.hup()
        assert wait_for(lambda: d.status().get("state") == "disabled", 3, "status disabled within 3 s")
        assert wait_for(lambda: b.snapshot().get(AVAIL) == "offline", 3, "retained offline")
        d.write_uci(enabled="1")
        d.hup()
        assert wait_for(lambda: b.snapshot().get(AVAIL) == "online" and b.connects >= 2, 30, "back online")
        log("disable_enable: ok")
    finally:
        d.stop()


def scenario_late_netifd():
    os.environ["E2E_INTERVAL"] = "10"
    b = Broker()
    d = Daemon(b.port)
    open(E + "/ubus.fail", "w").close()
    d.start()
    try:
        assert wait_for(lambda: b.connects >= 1, 10, "connect")
        time.sleep(2)
        snap = b.snapshot()
        log("first pass with netifd failing: %d retained topics, wan status: %s"
            % (len(snap), snap.get("openwrt/%s/wan-wan/status" % HOST)))
        assert snap.get("openwrt/%s/wan-wan/status" % HOST) is None
        os.remove(E + "/ubus.fail")
        assert wait_for(lambda: b.snapshot().get("openwrt/%s/wan-wan/status" % HOST) == "up", 25,
                        "the interface group to come back at its interval")
        log("late_netifd: ok")
    finally:
        if os.path.exists(E + "/ubus.fail"):
            os.remove(E + "/ubus.fail")
        os.environ["E2E_INTERVAL"] = "300"
        d.stop()


SCENARIOS = {
    "late_netifd": scenario_late_netifd,
    "lost_batch": scenario_lost_batch,
    "birth_flood": scenario_birth_flood,
    "config_retry": scenario_config_retry,
    "disable_enable": scenario_disable_enable,
}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    names = list(SCENARIOS) if which == "all" else [which]
    failed = 0
    for name in names:
        log("=== " + name)
        try:
            SCENARIOS[name]()
        except AssertionError as e:
            failed += 1
            log("FAILED %s: %s" % (name, e))
    sys.exit(1 if failed else 0)
