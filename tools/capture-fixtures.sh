#!/bin/sh
# Capture raw, unsanitized data sources from a router for test fixtures.
# Usage: SSHPASS=... tools/capture-fixtures.sh <router-address> <output-dir>
# The output directory must be outside the repository; run
# tools/sanitize-fixtures.py on it before anything reaches tests/fixtures/.
set -eu

host=$1
out=$2
mkdir -p "$out"

sshpass -e ssh -o ConnectTimeout=10 "root@$host" '
set -u
d=/tmp/mqtt-metrics-capture.$$
mkdir -p "$d" && cd "$d" || exit 1
ubus call system board > system_board.json
ubus call system info > system_info.json
ubus call network.interface dump > network_interface_dump.json
ubus call network.device status > network_device_status.json
ubus call mwan3 status > mwan3_status.json 2>/dev/null || true
ubus call ha-cluster status > ha_cluster_status.json 2>/dev/null || true
ubus call keepalived dump > keepalived_dump.json 2>/dev/null || true
ubus call luci-rpc getDHCPLeases > dhcp_leases.json 2>/dev/null || true
ubus call luci.wireguard getWgInstances > wireguard.json 2>/dev/null || true
ubus call iwinfo devices > iwinfo_devices.json 2>/dev/null || true
ubus call service list > service_list.json
cat /proc/stat > proc_stat
cat /proc/loadavg > proc_loadavg
cat /proc/uptime > proc_uptime
cat /proc/sys/net/netfilter/nf_conntrack_count > conntrack_count 2>/dev/null || true
cat /proc/sys/net/netfilter/nf_conntrack_max > conntrack_max 2>/dev/null || true
cat /etc/openwrt_release > openwrt_release
cat /tmp/sysinfo/model > sysinfo_model 2>/dev/null || true
cat /tmp/sysinfo/board_name > sysinfo_board_name 2>/dev/null || true
uname -m > uname_m
uci -q get system.@system[0].hostname > hostname || hostname > hostname
uci show network > uci_network
cat /lib/functions/network.sh > network.sh
df -k / /tmp > df 2>/dev/null || true
tar -cf - .
cd / && rm -rf "$d"
' | tar -xmf - -C "$out"
echo "captured $(ls "$out" | wc -l) files from $host into $out"
