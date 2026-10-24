#!/bin/sh
# Build the release packages for every supported package architecture with
# the official OpenWrt SDKs.
#
# Usage: tools/release-build.sh <work-dir> [<arch>...]
#
# <work-dir> holds the SDK downloads, the unpacked SDKs, a shared dl/
# directory, the source snapshot and the results (out/<arch>/). It needs about
# 6 GB per architecture; put it on a disk with room, it does not have to be
# the repository's.
#
# The packages are built from `git archive HEAD`, so only committed code is
# released. Each SDK builds its own Rust toolchain (rust/host) the first time,
# which takes the longest; later runs reuse it. Up to $JOBS architectures
# (default 3) build at the same time.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
SELF=$REPO/tools/release-build.sh
WORK=${1:?usage: release-build.sh <work-dir> [<arch>...]}
shift
OPENWRT=${OPENWRT:-25.12.6}
JOBS=${JOBS:-3}
MAKE_JOBS=${MAKE_JOBS:-16}
BASE=https://downloads.openwrt.org/releases/$OPENWRT/targets

# Package architecture and the SDK target that provides it. One package
# covers every device of that architecture. 32-bit x86 is missing: OpenWrt
# builds Rust for i586, without SSE2, which ring (TLS) requires.
TABLE='
x86_64 x86/64
aarch64_generic rockchip/armv8
aarch64_cortex-a53 mediatek/filogic
aarch64_cortex-a72 bcm27xx/bcm2711
aarch64_cortex-a76 bcm27xx/bcm2712
arm_cortex-a7 mediatek/mt7629
arm_cortex-a7_neon-vfpv4 ipq40xx/generic
arm_cortex-a9 bcm53xx/generic
arm_cortex-a9_vfpv3-d16 mvebu/cortexa9
arm_cortex-a15_neon-vfpv4 armsr/armv7
mips_24kc ath79/generic
mipsel_24kc ramips/mt7621
'

# WSL adds Windows directories (with spaces) to PATH; OpenWrt's host tools
# reject such entries.
PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | grep -v ' ' | paste -sd: -)
export PATH

mkdir -p "$WORK/downloads" "$WORK/sdk" "$WORK/dl" "$WORK/out" "$WORK/logs"
WORK=$(cd "$WORK" && pwd)

# Source snapshot of the committed tree.
# The commit is fixed for the whole run, even if HEAD moves meanwhile.
rev=${RELEASE_REV:-$(git -C "$REPO" rev-parse --short=12 HEAD)}
export RELEASE_REV=$rev
src="$WORK/src-$rev"
if [ ! -d "$src" ]; then
	mkdir -p "$src.tmp"
	git -C "$REPO" archive "$rev" package | tar -x -C "$src.tmp"
	mv "$src.tmp" "$src"
fi
echo "source: $REPO at $rev"

# One architecture, run as its own process (--build) so that `set -e` stops
# it at the first failed step and its `cd` stays inside.
build_one() {
	arch=$1
	target=$(printf '%s\n' "$TABLE" | awk -v a="$arch" '$1 == a { print $2 }')
	[ -n "$target" ] || { echo "$arch: unknown architecture"; return 1; }
	sdk="$WORK/sdk/$arch"
	echo "== $arch from $target, OpenWrt $OPENWRT, source $rev"
	if [ ! -f "$sdk/rules.mk" ]; then
		sums=$(curl -fsS --retry 3 "$BASE/$target/sha256sums")
		file=$(printf '%s\n' "$sums" | grep -o "openwrt-sdk-$OPENWRT-[^ ]*\.tar\.zst" | head -1)
		[ -n "$file" ] || { echo "no SDK for $target"; return 1; }
		want=$(printf '%s\n' "$sums" | awk -v f="*$file" '$2 == f { print $1 }')
		if [ ! -f "$WORK/downloads/$file" ] ||
			[ "$(sha256sum "$WORK/downloads/$file" | cut -d' ' -f1)" != "$want" ]; then
			curl -fsS --retry 3 -o "$WORK/downloads/$file.part" "$BASE/$target/$file"
			mv "$WORK/downloads/$file.part" "$WORK/downloads/$file"
		fi
		[ "$(sha256sum "$WORK/downloads/$file" | cut -d' ' -f1)" = "$want" ] ||
			{ echo "checksum mismatch: $file"; return 1; }
		rm -rf "$sdk.tmp"
		mkdir -p "$sdk.tmp"
		tar --zstd -xf "$WORK/downloads/$file" -C "$sdk.tmp" --strip-components=1
		mv "$sdk.tmp" "$sdk"
	fi
	cd "$sdk"
	# One download cache for all SDKs (Rust sources, crates).
	if [ ! -L dl ]; then
		rm -rf dl
		ln -s "$WORK/dl" dl
	fi
	if [ ! -f feeds/packages.index ]; then
		./scripts/feeds update base packages
		./scripts/feeds install -p base ubus uci libubox rpcd
		./scripts/feeds install -p packages rust
	fi
	for pkg in mqtt-metrics luci-app-mqtt-metrics; do
		ln -sfn "$src/package/$pkg" "package/$pkg"
		grep -q "^CONFIG_PACKAGE_$pkg=" .config 2>/dev/null ||
			echo "CONFIG_PACKAGE_$pkg=m" >> .config
	done
	make defconfig
	stamp="$WORK/logs/$arch.start"
	touch "$stamp"
	make "package/mqtt-metrics/clean" >/dev/null 2>&1 || true
	make -j"$MAKE_JOBS" package/mqtt-metrics/compile V=s
	if [ "$arch" = x86_64 ]; then
		make "package/luci-app-mqtt-metrics/clean" >/dev/null 2>&1 || true
		make package/luci-app-mqtt-metrics/compile V=s
	fi
	rm -rf "$WORK/out/$arch"
	mkdir -p "$WORK/out/$arch"
	find bin/packages -name 'mqtt-metrics-[0-9]*.apk' -newer "$stamp" -exec cp {} "$WORK/out/$arch/" \;
	find bin/packages -name 'luci-app-mqtt-metrics-[0-9]*.apk' -newer "$stamp" -exec cp {} "$WORK/out/$arch/" \;
	ls "$WORK/out/$arch"/mqtt-metrics-[0-9]*.apk >/dev/null
	ls -l "$WORK/out/$arch"
	echo "== $arch done"
}

run_one() {
	log="$WORK/logs/$1.log"
	if "$SELF" "$WORK" --build "$1" > "$log" 2>&1; then
		echo "$1: ok"
	else
		echo "$1: FAILED (log: $log)"
		return 1
	fi
}

# Internal: --one is used by the parallel driver below, --build by run_one.
case ${1:-} in
--one) run_one "$2"; exit $? ;;
--build) build_one "$2"; exit 0 ;;
esac

if [ $# -eq 0 ]; then
	set -- $(printf '%s\n' "$TABLE" | awk 'NF { print $1 }')
fi

# The first build fills the shared dl/ with the Rust sources; starting all at
# once would download them in parallel into the same place.
first=$1
shift
run_one "$first" || exit 1

status=0
if [ $# -gt 0 ]; then
	printf '%s\n' "$@" | xargs -P "$JOBS" -n 1 "$SELF" "$WORK" --one || status=1
fi
echo "results in $WORK/out"
exit $status
