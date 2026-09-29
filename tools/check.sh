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
sh -n package/mqtt-metrics/files/etc/init.d/mqtt-metrics
python3 -m py_compile tools/sanitize-fixtures.py tools/parity-check.py tools/gen-topics.py
echo "all checks passed"
