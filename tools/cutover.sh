#!/bin/sh
# Cutover from publish_metrics.sh to mqtt-metrics on one router
# (docs/MIGRATION.md). Every step runs on the router over SSH.
#
# Usage: SSHPASS=... tools/cutover.sh <router> <step>
#
# Steps:
#   sandbox-test  apply the cron and rc.local edits and the rollback to copies
#                 in /tmp and verify them; live files are not touched
#   backup        copy rc.local, crontab, the legacy script and the UCI config
#                 to /root/mqtt-metrics-migration (existing backup is kept)
#   switch        production topics with discovery on, remove the cron line
#   rclocal       remove the installer block from /etc/rc.local
#   verify        service state and the retained topics on the broker
#   rollback      stop and disable mqtt-metrics, restore crontab and rc.local
#
# switch, rclocal and rollback change live files and need the owner's
# confirmation for this router.
set -eu

host=$1
step=$2
ssh_() { sshpass -e ssh -o ConnectTimeout=10 "root@$host" "$@"; }

# Remote helpers. The rc.local block starts at the installer comment, skips
# the heredoc that contains the script (it has its own if/fi lines) and ends
# at the first top-level `fi` after the heredoc; one blank line after it is
# dropped as well.
HELPERS='
BACKUP=/root/mqtt-metrics-migration
remove_cron_line() { sed -i "\|/usr/bin/publish_metrics.sh|d" "$1"; }
strip_rclocal() {
	awk '"'"'
	BEGIN { s = 0; found = 0 }
	s == 0 && $0 == "# OpenWrt MQTT metrics one-time installer" { s = 1; found = 1; next }
	s == 1 && /<<'"'"'"'"'"'"'"'"'EOF'"'"'"'"'"'"'"'"'$/ { s = 2; next }
	s == 1 { next }
	s == 2 && $0 == "EOF" { s = 3; next }
	s == 2 { next }
	s == 3 && $0 == "fi" { s = 4; next }
	s == 3 { next }
	s == 4 && $0 == "" { s = 5; next }
	{ print; if (s == 4) s = 5 }
	END { if (!found || s < 4) exit 3 }
	'"'"' "$1" > "$1.new" || { rm -f "$1.new"; echo "installer block not found or incomplete" >&2; return 1; }
	sh -n "$1.new" || { rm -f "$1.new"; echo "result is not valid shell" >&2; return 1; }
	if grep -q publish_metrics "$1.new"; then rm -f "$1.new"; echo "publish_metrics still referenced" >&2; return 1; fi
	mv "$1.new" "$1"
}
'

case "$step" in
sandbox-test)
	ssh_ "$HELPERS"'
	set -e
	d=/tmp/mqtt-metrics-sandbox; rm -rf $d; mkdir -p $d
	cp -p /etc/rc.local $d/rc.local; cp -p /etc/crontabs/root $d/crontab
	cp -p $d/rc.local $d/rc.local.orig; cp -p $d/crontab $d/crontab.orig
	remove_cron_line $d/crontab
	strip_rclocal $d/rc.local
	echo "crontab: $(grep -c . $d/crontab.orig) -> $(grep -c . $d/crontab) lines, publish_metrics lines left: $(grep -c publish_metrics $d/crontab || true)"
	echo "rc.local: $(wc -l < $d/rc.local.orig) -> $(wc -l < $d/rc.local) lines, publish_metrics left: $(grep -c publish_metrics $d/rc.local || true), syntax: $(sh -n $d/rc.local && echo ok)"
	echo "other rc.local blocks kept: $(grep -c "^# OpenWrt HA config sync one-time installer" $d/rc.local) HA block, exit 0 line: $(grep -c "^exit 0" $d/rc.local)"
	start=$(grep -n "^# OpenWrt MQTT metrics one-time installer" $d/rc.local.orig | cut -d: -f1)
	removed=$(( $(wc -l < $d/rc.local.orig) - $(wc -l < $d/rc.local) ))
	echo "removed lines $start..$((start + removed - 1)) of the original"
	echo "original line before the block: $(sed -n "$((start - 1))p" $d/rc.local.orig | cut -c1-60)"
	echo "original line after the block:  $(sed -n "$((start + removed))p" $d/rc.local.orig | cut -c1-60)"
	echo "new line at the cut:            $(sed -n "${start}p" $d/rc.local | cut -c1-60)"
	# Rollback restores the originals byte for byte.
	cp -p $d/rc.local.orig $d/rc.local; cp -p $d/crontab.orig $d/crontab
	cmp $d/rc.local $d/rc.local.orig && cmp $d/crontab $d/crontab.orig && echo "rollback: files identical to the originals"
	rm -rf $d
	echo "live files untouched: $(md5sum /etc/rc.local /etc/crontabs/root | cut -c1-12 | tr "\n" " ")"
	'
	;;
backup)
	ssh_ '
	set -e
	BACKUP=/root/mqtt-metrics-migration
	if [ -d $BACKUP ]; then echo "backup exists, kept:"; ls -l $BACKUP; exit 0; fi
	mkdir -p $BACKUP; chmod 700 $BACKUP
	cp -p /etc/rc.local /etc/crontabs/root /usr/bin/publish_metrics.sh /etc/config/mqtt-metrics $BACKUP/
	(cd $BACKUP && md5sum rc.local root publish_metrics.sh mqtt-metrics > MD5SUMS)
	ls -l $BACKUP
	'
	;;
switch)
	ssh_ "$HELPERS"'
	set -e
	[ -f $BACKUP/MD5SUMS ] || { echo "run the backup step first" >&2; exit 1; }
	# Stop the legacy script first, so it cannot re-announce the entities
	# that the daemon removes on its first production pass.
	remove_cron_line /etc/crontabs/root
	/etc/init.d/cron restart
	n=0
	while pgrep -f publish_metrics.sh >/dev/null && [ $n -lt 60 ]; do sleep 1; n=$((n + 1)); done
	uci set mqtt-metrics.identity.base_topic="openwrt"
	uci set mqtt-metrics.identity.discovery="1"
	for o in discovery_prefix device_id birth_topic node; do uci -q delete mqtt-metrics.identity.$o || true; done
	uci commit mqtt-metrics
	/etc/init.d/mqtt-metrics reload
	sleep 10
	ubus call mqtt-metrics status | jsonfilter -e "@.state" -e "@.device.base_topic" -e "@.owned_topics"
	grep -c publish_metrics /etc/crontabs/root || true
	'
	;;
rclocal)
	ssh_ "$HELPERS"'
	set -e
	[ -f $BACKUP/MD5SUMS ] || { echo "run the backup step first" >&2; exit 1; }
	strip_rclocal /etc/rc.local
	echo "publish_metrics references left: $(grep -c publish_metrics /etc/rc.local || true); syntax: $(sh -n /etc/rc.local && echo ok)"
	'
	;;
verify)
	ssh_ '
	ubus call mqtt-metrics status | jsonfilter -e "@.state" -e "@.last_error" -e "@.device.base_topic" -e "@.device.discovery" -e "@.owned_topics"
	echo "cron entry: $(grep -c publish_metrics /etc/crontabs/root || true), rc.local references: $(grep -c publish_metrics /etc/rc.local || true)"
	'
	;;
rollback)
	ssh_ '
	set -e
	BACKUP=/root/mqtt-metrics-migration
	[ -f $BACKUP/MD5SUMS ] || { echo "no backup found" >&2; exit 1; }
	/etc/init.d/mqtt-metrics stop || true
	/etc/init.d/mqtt-metrics disable || true
	cp -p $BACKUP/root /etc/crontabs/root
	cp -p $BACKUP/rc.local /etc/rc.local
	cp -p $BACKUP/publish_metrics.sh /usr/bin/publish_metrics.sh
	/etc/init.d/cron restart
	(cd $BACKUP && md5sum -c MD5SUMS 2>/dev/null | grep -v mqtt-metrics) || true
	echo "legacy restored; it publishes again within five minutes"
	'
	;;
*)
	echo "unknown step $step" >&2
	exit 2
	;;
esac
