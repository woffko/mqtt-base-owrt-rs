#!/bin/sh
# Soak monitor, run on the router: every INTERVAL seconds append one CSV line
# with the daemon's pid, RSS, CPU ticks and status counters, and the number of
# warning/error log lines since the previous sample. Stops after DURATION.
# Output: /tmp/mqtt-metrics-soak/samples.csv (tmpfs, removed on reboot).
#
# Start: sh /tmp/mqtt-metrics-soak/monitor.sh [DURATION_S] [INTERVAL_S] &
set -u
DURATION=${1:-90000}
INTERVAL=${2:-300}
DIR=/tmp/mqtt-metrics-soak
OUT=$DIR/samples.csv
mkdir -p "$DIR"
echo $$ > "$DIR/monitor.pid"
[ -f "$OUT" ] || echo "time,pid,rss_kb,cpu_ticks,state,messages,publish_errors,reconnects,log_problems" > "$OUT"
end=$(( $(date +%s) + DURATION ))
last_log=0
while [ "$(date +%s)" -lt "$end" ]; do
	pid=$(pidof mqtt-metricsd | awk '{print $1}')
	rss=""; cpu=""
	if [ -n "$pid" ] && [ -r "/proc/$pid/status" ]; then
		rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status")
		cpu=$(awk '{print $14 + $15}' "/proc/$pid/stat")
	fi
	status=/var/run/mqtt-metrics/status.json
	state=$(jsonfilter -i "$status" -e '@.state' 2>/dev/null)
	msgs=$(jsonfilter -i "$status" -e '@.messages' 2>/dev/null)
	perr=$(jsonfilter -i "$status" -e '@.publish_errors' 2>/dev/null)
	reco=$(jsonfilter -i "$status" -e '@.reconnects' 2>/dev/null)
	problems=$(logread -e mqtt-metrics | grep -cE 'error:|warning:')
	echo "$(date +%s),$pid,$rss,$cpu,$state,$msgs,$perr,$reco,$((problems - last_log))" >> "$OUT"
	last_log=$problems
	sleep "$INTERVAL"
done
rm -f "$DIR/monitor.pid"
