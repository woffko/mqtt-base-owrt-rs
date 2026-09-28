#!/bin/sh
# Install the packages from dist/ on a router and run the post-install checks
# required by AGENTS.md (ubus object count, LuCI /ubus, service state, log).
# Only this project's packages are installed; nothing else is changed.
#
# Usage: SSHPASS=... tools/deploy.sh <router> [--daemon-only]
set -eu

host=$1
only_daemon=${2:-}
REPO=$(cd "$(dirname "$0")/.." && pwd)
ssh_() { sshpass -e ssh -o ConnectTimeout=10 "root@$host" "$@"; }

files=$(ls "$REPO"/dist/mqtt-metrics-[0-9]*.apk)
[ "$only_daemon" = "--daemon-only" ] || files="$files $(ls "$REPO"/dist/luci-app-mqtt-metrics-[0-9]*.apk)"

before=$(ssh_ 'ubus list | wc -l')
names=""
for f in $files; do
	name=$(basename "$f")
	ssh_ "cat > /tmp/$name" < "$f"
	names="$names /tmp/$name"
done
echo "installing:$names"
ssh_ "apk add --allow-untrusted $names; rc=\$?; rm -f $names; exit \$rc"

sleep 3
ssh_ "
echo \"ubus objects: before=$before after=\$(ubus list | wc -l)\"
apk list -I 2>/dev/null | grep -E '^(luci-app-)?mqtt-metrics-' || true
echo \"service: \$(ubus call service list '{\"name\":\"mqtt-metrics\"}' | grep -c '\"running\": true') running instance(s)\"
if wget -q -T 5 --no-check-certificate -O- --header='Content-Type: application/json' \
	--post-data='{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"list\",\"params\":[\"00000000000000000000000000000000\",\"session\"]}' \
	https://127.0.0.1/ubus | grep -q '\"session\"'; then echo 'LuCI /ubus: ok'; else echo 'LuCI /ubus: FAILED'; fi
ubus -v list mqtt-metrics >/dev/null 2>&1 && echo 'rpcd object mqtt-metrics: present' || echo 'rpcd object mqtt-metrics: absent'
ls -l /etc/config/mqtt-metrics
logread -e mqtt-metrics | tail -n 15
"
