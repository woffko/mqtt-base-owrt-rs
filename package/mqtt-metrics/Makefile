# SPDX-License-Identifier: GPL-2.0-or-later

include $(TOPDIR)/rules.mk

PKG_NAME:=mqtt-metrics
PKG_VERSION:=0.1.0
PKG_RELEASE:=6

PKG_MAINTAINER:=woffko
PKG_LICENSE:=GPL-2.0-or-later

PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1

include $(INCLUDE_DIR)/package.mk
include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk

define Package/mqtt-metrics
  SECTION:=utils
  CATEGORY:=Utilities
  TITLE:=Read-only router metrics publisher for MQTT and Home Assistant
  URL:=https://github.com/woffko/mqtt-base-owrt-rs
  DEPENDS:=$(RUST_ARCH_DEPENDS) +uci +ubus
endef

define Package/mqtt-metrics/description
  Publishes router state (system, interfaces, traffic, mwan3, HA cluster,
  DHCP, WireGuard, Wi-Fi, services) to an MQTT broker with Home Assistant
  discovery over one persistent connection. The defaults reproduce the
  legacy publish_metrics.sh topics and entity ids. The integration is
  read-only: it never acts on messages received from MQTT.
endef

define Package/mqtt-metrics/conffiles
/etc/config/mqtt-metrics
endef

# default_postinst enables and starts the service on a fresh install. On an
# upgrade the running process still uses the old binary, so restart it.
define Package/mqtt-metrics/postinst
[ -n "$${IPKG_INSTROOT}" ] || {
	if [ "$${PKG_UPGRADE:-0}" = 1 ]; then
		/etc/init.d/mqtt-metrics restart >/dev/null 2>&1 || true
	fi
	exit 0
}
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) ./src/Cargo.toml ./src/Cargo.lock $(PKG_BUILD_DIR)/
	$(CP) ./src/src $(PKG_BUILD_DIR)/
endef

define Build/Compile
	$(call Build/Compile/Cargo,,--features tls)
endef

define Package/mqtt-metrics/install
	$(INSTALL_DIR) $(1)/usr/sbin
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/mqtt-metricsd $(1)/usr/sbin/
	$(INSTALL_DIR) $(1)/etc/config
	$(INSTALL_CONF) ./files/etc/config/mqtt-metrics $(1)/etc/config/mqtt-metrics
	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/etc/init.d/mqtt-metrics $(1)/etc/init.d/mqtt-metrics
	$(INSTALL_DIR) $(1)/lib/upgrade/keep.d
	$(INSTALL_DATA) ./files/lib/upgrade/keep.d/mqtt-metrics $(1)/lib/upgrade/keep.d/mqtt-metrics
endef

$(eval $(call BuildPackage,mqtt-metrics))
