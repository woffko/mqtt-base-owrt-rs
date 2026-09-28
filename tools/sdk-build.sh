#!/bin/sh
# Build mqtt-metrics and luci-app-mqtt-metrics in an OpenWrt SDK through the
# package overlay (symlinks in <sdk>/package). Other packages, dl/,
# staging_dir/ and build_dir/ of the SDK are left alone.
#
# Usage: tools/sdk-build.sh [<sdk-dir>]
# Output: dist/*.apk and logs/sdk-*.log in the repository.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
SDK=${1:-${SDK:-/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64}}
PACKAGES="mqtt-metrics luci-app-mqtt-metrics"
STAMP=$(date +%Y%m%d-%H%M%S)

# WSL adds Windows directories (with spaces) to PATH; OpenWrt's host tools
# (find -execdir in the rust/host install step) reject such entries.
PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | grep -v ' ' | paste -sd: -)
export PATH

mkdir -p "$REPO/logs" "$REPO/dist"
[ -f "$SDK/rules.mk" ] || { echo "not an OpenWrt SDK: $SDK" >&2; exit 1; }

for pkg in $PACKAGES; do
	link="$SDK/package/$pkg"
	if [ -L "$link" ]; then
		[ "$(readlink "$link")" = "$REPO/package/$pkg" ] || { echo "$link points elsewhere" >&2; exit 1; }
	elif [ -e "$link" ]; then
		echo "$link exists and is not this project's symlink" >&2
		exit 1
	else
		ln -s "$REPO/package/$pkg" "$link"
	fi
done

cd "$SDK"
for pkg in $PACKAGES; do
	grep -q "^CONFIG_PACKAGE_$pkg=" .config || echo "CONFIG_PACKAGE_$pkg=m" >> .config
done
make defconfig > "$REPO/logs/sdk-defconfig-$STAMP.log" 2>&1

for pkg in $PACKAGES; do
	log="$REPO/logs/sdk-$pkg-$STAMP.log"
	echo "building $pkg (log: $log)"
	make "package/$pkg/clean" > /dev/null 2>&1 || true
	if ! make "package/$pkg/compile" V=s -j1 > "$log" 2>&1; then
		echo "build of $pkg failed; last lines of $log:" >&2
		tail -n 40 "$log" >&2
		exit 1
	fi
done

rm -f "$REPO"/dist/*.apk
find bin/packages -name 'mqtt-metrics-[0-9]*.apk' -newer "$REPO/logs/sdk-defconfig-$STAMP.log" -exec cp {} "$REPO/dist/" \;
find bin/packages -name 'luci-app-mqtt-metrics-[0-9]*.apk' -newer "$REPO/logs/sdk-defconfig-$STAMP.log" -exec cp {} "$REPO/dist/" \;
ls -l "$REPO/dist"
