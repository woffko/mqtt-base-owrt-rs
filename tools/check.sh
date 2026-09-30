#!/bin/sh
# Run every source check required before a commit (AGENTS.md).
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)

cd "$REPO/package/mqtt-metrics/src"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --quiet
# The package is built with TLS (ADR-0005): check that feature set as well.
cargo clippy --all-targets --features tls -- -D warnings
cargo test --quiet --features tls

cd "$REPO/package/luci-app-mqtt-metrics"
for f in htdocs/luci-static/resources/mqtt-metrics/*.js htdocs/luci-static/resources/view/mqtt-metrics/*.js; do
	node --check "$f"
done
node tests/common.test.js
node tests/package.test.js

cd "$REPO"
# The LuCI views run in headless Chromium against a stand-in for the LuCI
# runtime; skipped where the local Playwright installation is not available.
if [ -d /home/w0w/.local/share/playwright/node_modules/playwright ]; then
	node tools/luci-view-tests.browser.js
else
	echo "skipping tools/luci-view-tests.browser.js (no Playwright)"
fi
# The same views on the real LuCI client code (form, validation, rpc) from an
# SDK; skipped where the SDK or Playwright is not available.
SDK=${SDK:-/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64}
if [ -d /home/w0w/.local/share/playwright/node_modules/playwright ] && [ -f "$SDK/feeds/luci/modules/luci-base/htdocs/luci-static/resources/form.js" ]; then
	node tools/luci-real.browser.js "$SDK"
else
	echo "skipping tools/luci-real.browser.js (no Playwright or no SDK with LuCI)"
fi
sh -n package/mqtt-metrics/files/etc/init.d/mqtt-metrics
sh -n tools/e2e-smoke.sh
python3 -m py_compile tools/sanitize-fixtures.py tools/gen-topics.py tools/e2e/e2e.py
# The daemon against a fake broker and fake uci/ubus takes about a minute and
# a half: run it with E2E=1 before a deployment.
if [ -n "${E2E:-}" ]; then
	sh tools/e2e-smoke.sh
else
	echo "skipping tools/e2e-smoke.sh (set E2E=1 to run it)"
fi
echo "all checks passed"
