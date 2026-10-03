# SPDX-License-Identifier: Apache-2.0

include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-mqtt-metrics
PKG_VERSION:=0.1.0
PKG_RELEASE:=8

PKG_MAINTAINER:=woffko
PKG_LICENSE:=Apache-2.0

include $(INCLUDE_DIR)/package.mk

define Package/luci-app-mqtt-metrics
  SECTION:=luci
  CATEGORY:=LuCI
  SUBMENU:=3. Applications
  TITLE:=LuCI support for MQTT Metrics
  URL:=https://github.com/woffko/mqtt-base-owrt-rs
  DEPENDS:=+mqtt-metrics +luci-base
  PKGARCH:=all
endef

define Package/luci-app-mqtt-metrics/description
  Web interface for mqtt-metrics: broker settings, topics and device
  identity, per-metric publishing and Home Assistant visibility, status and
  diagnostics.
endef

define Package/luci-app-mqtt-metrics/postinst
[ -n "$${IPKG_INSTROOT}" ] || {
	rm -f /tmp/luci-indexcache /tmp/luci-indexcache.*
	rm -rf /tmp/luci-modulecache /tmp/luci-modulecache.*
	/etc/init.d/rpcd reload >/dev/null 2>&1 || true
	exit 0
}
endef

define Build/Compile
endef

define Package/luci-app-mqtt-metrics/install
	$(INSTALL_DIR) $(1)/www
	$(CP) ./htdocs/* $(1)/www/
	$(INSTALL_DIR) $(1)/usr/share/luci/menu.d $(1)/usr/share/rpcd/acl.d
	$(INSTALL_DATA) ./root/usr/share/luci/menu.d/luci-app-mqtt-metrics.json $(1)/usr/share/luci/menu.d/
	$(INSTALL_DATA) ./root/usr/share/rpcd/acl.d/luci-app-mqtt-metrics.json $(1)/usr/share/rpcd/acl.d/
	$(INSTALL_DIR) $(1)/usr/libexec/rpcd
	$(LN) /usr/sbin/mqtt-metricsd $(1)/usr/libexec/rpcd/mqtt-metrics
endef

$(eval $(call BuildPackage,luci-app-mqtt-metrics))
