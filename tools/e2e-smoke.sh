#!/bin/sh
# End-to-end smoke test of the daemon on this machine, without a router.
#
# It builds a copy of the crate whose absolute paths (state, secret, run
# directory, configuration file) point into a scratch directory,
# puts fake `uci` and `ubus` commands (answering from tests/fixtures/primary)
# in front of PATH and runs the real daemon against an in-process fake broker
# (tools/e2e/e2e.py). It covers what the unit tests cannot: the daemon loop,
# reconnects, signals and the state file.
#
# Usage: tools/e2e-smoke.sh [<scenario>]     (default: all)
set -eu

[ "$(id -u)" != 0 ] || { echo "do not run this as root" >&2; exit 1; }
REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=${E2E_DIR:-$(mktemp -d)}
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)

rm -rf "$WORK/crate"
mkdir -p "$WORK/crate" "$WORK/bin" "$WORK/fix" "$WORK/etc" "$WORK/run" "$WORK/etcconfig"
(cd "$REPO/package/mqtt-metrics/src" && tar --exclude=target -cf - .) | tar -xf - -C "$WORK/crate"
: > "$WORK/etcconfig/mqtt-metrics"

patch_path() { # file old new
	sed -i "s#\"$2\"#\"$3\"#" "$WORK/crate/src/$1"
	grep -q "\"$3\"" "$WORK/crate/src/$1" || { echo "path $2 not found in $1" >&2; exit 1; }
}
patch_path config.rs /etc/mqtt-metrics/secret "$WORK/etc/secret"
patch_path config.rs /etc/mqtt-metrics/state.json "$WORK/etc/state.json"
patch_path config.rs /var/run/mqtt-metrics "$WORK/run"
sed -i "s#\"/etc/config/{}\"#\"$WORK/etcconfig/{}\"#" "$WORK/crate/src/rpcd.rs"
grep -q "$WORK/etcconfig" "$WORK/crate/src/rpcd.rs" || { echo "config path not found in rpcd.rs" >&2; exit 1; }

for f in system_board.json system_info.json network_interface_dump.json uci_network hostname; do
	cp "$REPO/tests/fixtures/primary/$f" "$WORK/fix/$f"
done
cat > "$WORK/bin/ubus" <<'FAKE'
#!/bin/sh
[ "$1" = "-t" ] && shift 2
[ "$1" = "call" ] && shift
case "$1 $2" in
"system board") cat "$FIX/system_board.json" ;;
"system info") cat "$FIX/system_info.json" ;;
"network.interface dump") [ -f "$E2E/ubus.fail" ] && exit 1; cat "$FIX/network_interface_dump.json" ;;
*) exit 4 ;;
esac
FAKE
cat > "$WORK/bin/uci" <<'FAKE'
#!/bin/sh
case "$*" in
"-q show mqtt-metrics") [ -f "$E2E/uci.fail" ] && exit 1; cat "$E2E/mqtt-metrics.uci" ;;
"-q show network") cat "$FIX/uci_network" ;;
"-q get system.@system[0].hostname") cat "$FIX/hostname" ;;
*) exit 1 ;;
esac
FAKE
chmod +x "$WORK/bin/ubus" "$WORK/bin/uci"

(cd "$WORK/crate" && CARGO_TARGET_DIR="$WORK/target" cargo build --quiet)

E2E_DIR=$WORK E2E_BIN="$WORK/target/debug/mqtt-metricsd" python3 "$REPO/tools/e2e/e2e.py" "${1:-all}"
