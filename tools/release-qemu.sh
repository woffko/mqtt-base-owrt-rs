#!/bin/sh
# Run the daemon of every release package under qemu-user on the router
# fixtures and compare its --dump and --catalog with a host build. This
# catches what a cross build can get wrong and still link: byte order, float
# ABI, missing CPU features.
#
# Usage: tools/release-qemu.sh <work-dir> [<arch>...]
#
# <work-dir> is the one of tools/release-build.sh (packages in out/<arch>/,
# the musl loader from each SDK's toolchain). $QEMU names the directory with
# qemu-<cpu>-static, default <work-dir>/qemu/usr/bin: the Debian or Ubuntu
# package qemu-user-static unpacked there with `dpkg-deb -x` is enough.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(cd "${1:?usage: release-qemu.sh <work-dir> [<arch>...]}" && pwd)
shift
QEMU=${QEMU:-$WORK/qemu/usr/bin}
FIX=$REPO/tests/fixtures/primary
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# duration_ms is the time a pass took; everything else must match.
norm() { sed -E 's/"duration_ms": *[0-9]+/"duration_ms":0/g'; }

(cd "$REPO/package/mqtt-metrics/src" && cargo build -q --release --features tls)
host=$REPO/package/mqtt-metrics/src/target/release/mqtt-metricsd
"$host" --dump --fixtures "$FIX" | sort > "$TMP/host.dump"
"$host" --catalog --fixtures "$FIX" | norm > "$TMP/host.catalog"
apk=$(ls "$WORK"/sdk/*/staging_dir/host/bin/apk | head -1)

[ $# -gt 0 ] || set -- $(ls "$WORK/out")
status=0
for arch in "$@"; do
	case $arch in
	x86_64) q= ;;
	aarch64_*) q=$QEMU/qemu-aarch64-static ;;
	arm_*) q=$QEMU/qemu-arm-static ;;
	mipsel_*) q=$QEMU/qemu-mipsel-static ;;
	mips_*) q=$QEMU/qemu-mips-static ;;
	*) echo "$arch: no emulator known"; status=1; continue ;;
	esac
	pkg=$(ls "$WORK/out/$arch"/mqtt-metrics-[0-9]*.apk 2>/dev/null | head -1)
	[ -n "$pkg" ] || { echo "$arch: no package"; status=1; continue; }
	tc=$(ls -d "$WORK/sdk/$arch"/staging_dir/toolchain-* | head -1)
	d=$TMP/$arch
	mkdir -p "$d"
	"$apk" extract --allow-untrusted --destination "$d" "$pkg" >/dev/null
	bin=$d/usr/sbin/mqtt-metricsd
	if [ -z "$q" ]; then
		run() { "$tc"/lib/ld-musl-*.so.1 --library-path "$tc/lib" "$bin" "$@"; }
	else
		run() { "$q" -L "$tc" "$bin" "$@"; }
	fi
	run --dump --fixtures "$FIX" 2>"$d/dump.err" | sort > "$d/dump"
	run --catalog --fixtures "$FIX" 2>"$d/catalog.err" | norm > "$d/catalog"
	if cmp -s "$d/dump" "$TMP/host.dump" && cmp -s "$d/catalog" "$TMP/host.catalog"; then
		echo "$arch: $(basename "$pkg") matches the host build ($(wc -l < "$d/dump") dump lines)"
	else
		echo "$arch: $(basename "$pkg") DIFFERS from the host build"
		diff "$TMP/host.dump" "$d/dump" | head -5 || true
		cat "$d/dump.err" "$d/catalog.err" | head -5
		status=1
	fi
done
exit $status
