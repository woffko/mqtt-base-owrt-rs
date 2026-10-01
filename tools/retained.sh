#!/bin/sh
# Print the retained topics this router owns on the broker, sorted: the state
# topics under the base topic and the router's discovery configs.
#
# It runs mosquitto_sub on the router with the broker settings of
# /etc/config/mqtt-metrics. The password is read from
# /etc/mqtt-metrics/secret into a temporary options file (mode 0600) that
# mosquitto_sub reads; it never appears on a command line or in the output.
#
# Usage: SSHPASS=... tools/retained.sh <router> [<wait-seconds>]
set -eu

host=$1
wait=${2:-4}

sshpass -e ssh -o ConnectTimeout=10 "root@$host" "WAIT=$wait sh -s" <<'REMOTE'
set -eu
command -v mosquitto_sub >/dev/null || { echo "mosquitto_sub is not installed" >&2; exit 2; }
H=$(uci -q get system.@system[0].hostname || cat /proc/sys/kernel/hostname)
base=$(uci -q get mqtt-metrics.identity.base_topic || echo openwrt)
node=$(uci -q get mqtt-metrics.identity.node || true)
prefix=$(uci -q get mqtt-metrics.identity.discovery_prefix || true)
device=$(uci -q get mqtt-metrics.identity.device_id || true)
[ -n "$node" ] || node=$H
[ -n "$prefix" ] || prefix=homeassistant
[ -n "$device" ] || device=openwrt_$(printf %s "$H" | tr -c 'A-Za-z0-9_-' '_')

d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
umask 077
{
	echo "-h $(uci -q get mqtt-metrics.broker.host)"
	echo "-p $(uci -q get mqtt-metrics.broker.port || echo 1883)"
	user=$(uci -q get mqtt-metrics.broker.username || true)
	[ -z "$user" ] || echo "-u $user"
	# printf is an ash builtin: the password is not on any command line.
	[ ! -s /etc/mqtt-metrics/secret ] || printf -- '-P %s\n' "$(cat /etc/mqtt-metrics/secret)"
} > "$d/mosquitto_sub"
sub() { XDG_CONFIG_HOME=$d timeout $((WAIT + 6)) mosquitto_sub -v -W "$WAIT" -t "$1" 2>/dev/null || true; }
{
	sub "$base/$node/#"
	sub "$prefix/+/+/config" | grep "^$prefix/[a-z_]*/${device}_" || true
} | grep -v '^$' | sort
REMOTE
