#!/usr/bin/env python3
"""Evaluate a soak-monitor.sh CSV: restarts, RSS growth, CPU use, errors.

Usage: soak-report.py samples.csv [--clock-ticks 100]
Exit status 0 when there were no restarts, no publish errors, no warning or
error log lines, and RSS grew by less than 10 % after the first hour.
"""
import argparse
import csv
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("csv")
    ap.add_argument("--clock-ticks", type=int, default=100)
    args = ap.parse_args()
    samples = list(csv.DictReader(open(args.csv)))
    # A sample without a pid was taken while the daemon was not running.
    down = [r for r in samples if not r["pid"]]
    rows = [r for r in samples if r["pid"]]
    if len(rows) < 2:
        print("not enough samples")
        return 1
    t0, t1 = int(rows[0]["time"]), int(rows[-1]["time"])
    hours = (t1 - t0) / 3600
    pids = sorted({r["pid"] for r in rows})
    rss = [int(r["rss_kb"]) for r in rows if r["rss_kb"]]
    warm = [int(r["rss_kb"]) for r in rows if r["rss_kb"] and int(r["time"]) >= t0 + 3600] or rss
    same_pid = [r for r in rows if r["pid"] == rows[-1]["pid"] and r["cpu_ticks"]]
    cpu_pct = 0.0
    if len(same_pid) >= 2:
        dt = int(same_pid[-1]["time"]) - int(same_pid[0]["time"])
        dticks = int(same_pid[-1]["cpu_ticks"]) - int(same_pid[0]["cpu_ticks"])
        cpu_pct = dticks / args.clock_ticks / dt * 100 if dt else 0.0
    perr = int(rows[-1]["publish_errors"] or 0) - int(rows[0]["publish_errors"] or 0)
    reco = int(rows[-1]["reconnects"] or 0) - int(rows[0]["reconnects"] or 0)
    # The monitor records the change of a count taken over the bounded system
    # log; lines that age out make the change negative, and must not cancel
    # the lines that were added.
    problems = sum(max(0, int(r["log_problems"] or 0)) for r in samples[1:])
    states = sorted({r["state"] for r in rows})
    growth = (warm[-1] - warm[0]) / warm[0] * 100 if warm and warm[0] else 0.0

    print("duration: %.1f h, samples: %d" % (hours, len(rows)))
    print("pids: %s (restarts: %d)" % (", ".join(pids), len(pids) - 1))
    print("states seen: %s" % ", ".join(states))
    print("RSS: min %d kB, max %d kB, first-hour-excluded growth %.1f %%" % (min(rss), max(rss), growth))
    print("CPU: %.3f %% of one core on average" % cpu_pct)
    print("messages: %s -> %s" % (rows[0]["messages"], rows[-1]["messages"]))
    print("publish errors: %d, reconnects: %d, warning/error log lines: %d" % (perr, reco, problems))
    print("samples without a running daemon: %d" % len(down))
    ok = (
        len(pids) == 1
        and not down
        and perr == 0
        and problems == 0
        and growth < 10
        and states == ["connected"]
    )
    print("RESULT: %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
