'use strict';
// Desktop and mobile screenshots of every MQTT Metrics LuCI page.
// Read-only: UCI writes and write methods of the mqtt-metrics rpcd object are
// blocked in the browser. JavaScript errors on the pages are reported.
//
// Password: stdin. Usage:
//   node tools/luci-screenshots.browser.js <router-address> [<output-dir>]
// Output is written under logs/ (git-ignored): screenshots show live router
// data such as addresses.
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('/home/w0w/.local/share/playwright/node_modules/playwright');

const host = process.argv[2];
if (!host) {
	console.error('usage: luci-screenshots.browser.js <router-address> [<output-dir>]');
	process.exit(2);
}
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '');
const out = process.argv[3] || path.join(__dirname, '..', 'logs', 'screenshots-' + stamp);
const password = fs.readFileSync(0, 'utf8').replace(/[\r\n]+$/, '');
const origin = 'https://' + host;
const base = origin + '/cgi-bin/luci/admin/services/mqtt-metrics/';
const pages = [ 'status', 'broker', 'topics', 'metrics', 'diagnostics' ];
const writeMethods = [ 'test_connection', 'set_password', 'import_legacy', 'republish', 'purge', 'reload' ];
const viewports = {
	desktop: { width: 1440, height: 900 },
	mobile: { width: 390, height: 844 }
};
const report = { shots: [], blocked: [], errors: [], console: [] };

fs.mkdirSync(out, { recursive: true, mode: 0o700 });

(async () => {
	const browser = await chromium.launch({ headless: true,
		executablePath: '/home/w0w/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome' });
	for (const [ kind, viewport ] of Object.entries(viewports)) {
		const context = await browser.newContext({ viewport, ignoreHTTPSErrors: true,
			isMobile: kind === 'mobile', hasTouch: kind === 'mobile', colorScheme: 'light' });
		const page = await context.newPage();
		page.setDefaultTimeout(30000);
		page.on('pageerror', (e) => report.errors.push({ kind, url: page.url().replace(origin, ''), message: e.message.slice(0, 300) }));
		page.on('console', (m) => { if (m.type() === 'error') report.console.push({ kind, text: m.text().slice(0, 300) }); });
		await page.route('**/*', async (route) => {
			const url = new URL(route.request().url());
			if (url.origin !== origin)
				return route.abort('blockedbyclient');
			let body;
			try { body = route.request().postDataJSON(); } catch (_) { body = null; }
			for (const packet of Array.isArray(body) ? body : [ body ]) {
				if (!packet || packet.method !== 'call' || !Array.isArray(packet.params))
					continue;
				const [ , object, method ] = packet.params;
				if ((object === 'uci' && [ 'set', 'add', 'delete', 'commit', 'apply', 'revert', 'confirm' ].includes(method)) ||
				    (object === 'mqtt-metrics' && writeMethods.includes(method))) {
					report.blocked.push({ kind, object, method });
					return route.abort('blockedbyclient');
				}
			}
			return route.continue();
		});

		await page.goto(base + 'status', { waitUntil: 'domcontentloaded' });
		if (await page.locator('input[name="luci_password"]').count()) {
			await page.locator('input[name="luci_username"]').fill('root');
			await page.locator('input[name="luci_password"]').fill(password);
			await page.locator('button.cbi-button-positive, button:has-text("Log in"), button:has-text("Login")').first().click();
			await page.waitForLoadState('domcontentloaded');
		}
		for (const name of pages) {
			await page.goto(base + name, { waitUntil: 'domcontentloaded' });
			await page.waitForSelector('#view > :not(.spinning)', { timeout: 30000 }).catch(() => {});
			await page.waitForTimeout(1500);
			if (name === 'metrics') {
				// Expand every group for the full-page shot.
				const headers = page.locator('[data-group] > h3');
				for (let i = 0; i < await headers.count(); i++) {
					const body = headers.nth(i).locator('xpath=following-sibling::div[1]');
					if (!(await body.isVisible()))
						await headers.nth(i).click();
				}
			}
			const file = path.join(out, `${kind}-${name}.png`);
			await page.screenshot({ path: file, fullPage: true });
			report.shots.push(path.basename(file));
		}
		await context.close();
	}
	await browser.close();
	fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify(report, null, 2), { mode: 0o600 });
	console.log(JSON.stringify({ out, shots: report.shots.length, errors: report.errors, console: report.console, blocked: report.blocked.length }, null, 2));
	process.exit(report.errors.length ? 1 : 0);
})().catch((e) => {
	console.error(e);
	process.exit(1);
});
