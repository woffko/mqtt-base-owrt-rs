---
name: openwrt-package
description: Rules for OpenWrt 25.12 packages in this repository - package feed layout, Rust packages built with rust-package.mk, procd init scripts, UCI config, rpcd plugins, LuCI JavaScript apps, apk versioning, SDK builds and on-router acceptance. Use for any change under package/, any SDK build, and any install on a router.
---

# OpenWrt package rules

These rules follow the OpenWrt package guidelines (packages feed
`CONTRIBUTING.md`, LuCI `doc/` conventions) and the conventions already used
by `/home/w0w/cake-autorate-rs/package/`. When they conflict, the upstream
OpenWrt rule wins.

## Repository layout

The repository is a package feed and SDK overlay:

```text
package/<pkg>/Makefile
package/<pkg>/files/        installed config, init scripts, rpcd/ucitrack files
package/<pkg>/src/          bundled source (Cargo workspace for Rust)
package/luci-app-<pkg>/Makefile
package/luci-app-<pkg>/htdocs/luci-static/resources/view/<pkg>/*.js
package/luci-app-<pkg>/root/usr/share/luci/menu.d/luci-app-<pkg>.json
package/luci-app-<pkg>/root/usr/share/rpcd/acl.d/luci-app-<pkg>.json
feeds.conf.example          src-git or src-link line for this feed
```

- One package directory per `PKG_NAME`. The daemon and its LuCI app are
  separate packages; the LuCI app depends on the daemon, never the reverse.
- The LuCI app sets `PKGARCH:=all` and has an empty `Build/Compile`.

## Makefile

- Start with an SPDX line, then `include $(TOPDIR)/rules.mk`.
- Set `PKG_NAME`, `PKG_VERSION`, `PKG_RELEASE`, `PKG_MAINTAINER`,
  `PKG_LICENSE`, and `PKG_LICENSE_FILES` when a licence file ships.
- Bump `PKG_RELEASE` on every packaging or code change that reaches a router.
  Reset it to 1 when `PKG_VERSION` changes. apk compares
  `<version>-r<release>`; never reuse a version for different content.
- Rust packages:
  - `PKG_BUILD_DEPENDS:=rust/host`, `PKG_BUILD_PARALLEL:=1`;
  - `include $(INCLUDE_DIR)/package.mk` then
    `include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk`;
  - `DEPENDS:=$(RUST_ARCH_DEPENDS) ...`;
  - `Build/Prepare` copies only `Cargo.toml`, `Cargo.lock` and `src/` into
    `$(PKG_BUILD_DIR)`; `Build/Compile` calls `Build/Compile/Cargo`;
  - install the binary from `$(PKG_INSTALL_DIR)/bin/`.
- Declare every runtime dependency in `DEPENDS` (`+libubox`, `+ubus`, ...).
  Do not rely on packages that happen to be installed on the test router.
- List every UCI file under `Package/<pkg>/conffiles` so upgrades and
  sysupgrade keep user settings.
- `postinst`/`prerm` scripts must start with `[ -n "$${IPKG_INSTROOT}" ] ||`
  so image builds do not execute them, and must be idempotent. A LuCI app
  `postinst` clears `/tmp/luci-indexcache*` and `/tmp/luci-modulecache*` and
  reloads rpcd.
- Keep Cargo release profile small: `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `strip = true`, `panic = "abort"` unless unwinding is
  required.

## Runtime integration

- Services use procd (`USE_PROCD=1`, `START=`/`STOP=` chosen relative to
  network and ubus), `procd_set_param respawn`, stdout/stderr to syslog, and
  `procd_add_reload_trigger "<uci-config>"` so UCI commits reload the daemon.
- Configuration lives in `/etc/config/<name>` (UCI). Secrets stored in UCI
  must be in a file with mode 0600 and must never appear in argv, logs,
  `ubus` replies, LuCI page source, or crash output.
- LuCI talks to the daemon through an rpcd plugin: either an executable in
  `/usr/libexec/rpcd/<object>` implementing `list` and `call <method>` (JSON on
  stdin/stdout), or a ucode plugin in `/usr/share/rpcd/ucode/`. Every method
  must be listed in the ACL file with the smallest read/write split possible.
- LuCI views are client-side JavaScript (`'use strict'; 'require view';`,
  `form`, `rpc`, `uci`, `ui`, `poll`). No Lua, no server-side templates. All
  user-visible strings go through `_()` so they can be translated.
- Never write files outside the package's own paths at install time. Do not
  edit `/etc/rc.local`, crontabs, or other packages' config from package
  scripts; migration from a legacy setup is a documented manual step or an
  explicit opt-in action in the UI.

## Build

- Use an OpenWrt 25.12 SDK whose target matches the router
  (x86_64 test router: `openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl`).
- Overlay workflow: symlink or copy `package/<pkg>` into `<sdk>/package/`,
  `make defconfig`, then
  `make package/<pkg>/{clean,compile} V=s -j1`. Save logs under `logs/`
  (git-ignored). On failure, re-run the failing package with `V=s` first.
- Never run `make dirclean`, `make distclean`, `git clean -xfd`, or delete
  `dl/`, `staging_dir/`, `build_dir/`, `bin/`, or `.config` in a shared SDK
  without an explicit request.
- Fast iteration outside the SDK: `cargo build --release --target
  x86_64-unknown-linux-musl`. Final artifacts that reach a router must come
  from the SDK so `DEPENDS`, conffiles and scripts are exercised.

## On-router acceptance

- Before any change on a router: record `apk info -v`, the relevant UCI files,
  `/etc/crontabs/root`, `ubus list | wc -l`, and take a backup of every file
  that will change.
- Install with `apk add --allow-untrusted /tmp/<file>.apk`; check
  `apk info -v <pkg>`, the service state, `logread`, and that LuCI `/ubus`
  still answers.
- Rollback must be defined and tested before a cutover.
- Test routers may carry real traffic: no firewall restart, no forced WAN
  failover, never signal `ubusd`, no reboot without an explicit request.
