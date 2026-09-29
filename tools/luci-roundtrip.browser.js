'use strict';
// LuCI round-trip test: edit settings on the Broker, Topics and Metrics pages
// through the browser, Save & Apply, then print `uci show mqtt-metrics`
// taken over SSH so the caller can compare. A second pass runs "Reset to
// legacy defaults". Every response body the browser receives is written to
// <out>/responses.txt so the caller can check on the router that the broker
// password never reaches the browser.
//
// Writes are limited to the mqtt-metrics UCI config and the rpcd methods
// reload/republish; set_password, import_legacy and purge are blocked.
//
// It changes the router's configuration: the edit phase sets the keepalive,
// QoS, device name and model, publish interval, a hidden metric and a
// service, and the reset phase resets only the Metrics page, so the Broker and
// Topics edits stay. Run it on a test router, or take `uci export
// mqtt-metrics` first. It refuses to run unless LUCI_ROUNDTRIP_CONFIRM is set
// to the router address.
//
// Password: stdin. Usage:
//   LUCI_ROUNDTRIP_CONFIRM=<router> node tools/luci-roundtrip.browser.js <router> <phase: edit|reset> <out-dir>
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('/home/w0w/.local/share/playwright/node_modules/playwright');

const [ host, phase, out ] = process.argv.slice(2);
if (!host || ![ 'edit', 'reset' ].includes(phase) || !out) {
	console.error('usage: luci-roundtrip.browser.js <router> edit|reset <out-dir>');
	process.exit(2);
}
if (process.env.LUCI_ROUNDTRIP_CONFIRM !== host) {
	console.error('this changes the configuration of ' + host + '; set LUCI_ROUNDTRIP_CONFIRM=' + host + ' to run it');
	process.exit(2);
}
const password = fs.readFileSync(0, 'utf8').replace(/[\r\n]+$/, '');
const origin = 'https://' + host;
const base = origin + '/cgi-bin/luci/admin/services/mqtt-metrics/';
const report = { phase, steps: [], blocked: [], errors: [] };
fs.mkdirSync(out, { recursive: true, mode: 0o700 });
const responses = fs.openSync(path.join(out, 'responses-' + phase + '.txt'), 'w', 0o600);

async function saveApply(page, label) {
	await page.locator('.cbi-page-actions .cbi-button-apply li[data-value="0"]').first().click();
	// Wait for the apply overlay to finish ("Configuration changes applied.").
	await page.waitForFunction(() => /applied|No changes to apply/i.test(document.body.innerText), null, { timeout: 60000 })
		.catch(() => report.errors.push({ step: label, message: 'no apply confirmation seen' }));
	await page.waitForTimeout(3000);
	report.steps.push(label);
}

async function row(page, key) {
	return page.locator(`tr[data-key="${key}"]`);
}

(async () => {
	const browser = await chromium.launch({ headless: true,
		executablePath: '/home/w0w/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome' });
	const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, ignoreHTTPSErrors: true });
	const page = await context.newPage();
	page.setDefaultTimeout(30000);
	page.on('dialog', (d) => d.accept());
	page.on('pageerror', (e) => report.errors.push({ url: page.url().replace(origin, ''), message: e.message.slice(0, 300) }));
	page.on('response', async (r) => {
		try {
			fs.writeSync(responses, r.url() + '\n' + (await r.text()) + '\n');
		} catch (_) { /* bodies of redirects are not available */ }
	});
	await page.route('**/*', async (route) => {
		let body;
		try { body = route.request().postDataJSON(); } catch (_) { body = null; }
		for (const packet of Array.isArray(body) ? body : [ body ]) {
			if (!packet || packet.method !== 'call' || !Array.isArray(packet.params))
				continue;
			const [ , object, method, args ] = packet.params;
			const foreignUci = object === 'uci' && args && args.config && args.config !== 'mqtt-metrics' &&
				[ 'set', 'add', 'delete', 'commit' ].includes(method);
			const blockedRpc = object === 'mqtt-metrics' && [ 'set_password', 'import_legacy', 'purge' ].includes(method);
			if (foreignUci || blockedRpc) {
				report.blocked.push({ object, method });
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
		await page.waitForLoadState('networkidle').catch(() => {});
	}
	report.errors = [];

	if (phase === 'edit') {
		await page.goto(base + 'broker', { waitUntil: 'networkidle' });
		await page.locator('[data-name="keepalive"] input').fill('45');
		await page.locator('[data-name="qos"] select').selectOption('0');
		fs.writeFileSync(path.join(out, 'broker-page.html'), await page.content(), { mode: 0o600 });
		await saveApply(page, 'broker: keepalive 45, qos 0, password untouched');

		await page.goto(base + 'topics', { waitUntil: 'networkidle' });
		await page.locator('[data-name="device_name"] input').fill('Roundtrip Test Device');
		await page.locator('[data-name="model"] input').fill('RT-Model');
		await saveApply(page, 'topics: device name and model');

		await page.goto(base + 'metrics', { waitUntil: 'networkidle' });
		await page.waitForSelector('tr[data-key="system_uptime"]', { state: 'attached' });
		// Expand the System group if it is collapsed.
		const system = page.locator('[data-group="system"] > div');
		if (!(await system.isVisible()))
			await page.locator('[data-group="system"] > h3').click();
		const inputs = page.locator('.cbi-section').first().locator('input');
		await inputs.nth(0).fill('120');                  // global interval
		await inputs.nth(1).check();                      // change-only
		await page.locator('[data-group="system"] input[type=text]').first().fill('30');
		await (await row(page, 'system_load_15')).locator('input[type=checkbox]').first().uncheck();
		const uptime = await row(page, 'system_uptime');
		await uptime.locator('input[type=text]').nth(0).fill('Router Uptime RT');
		await uptime.locator('input[type=text]').nth(1).fill('90');
		await uptime.locator('button').click();           // details
		const details = uptime.locator('xpath=following-sibling::tr[1]');
		await details.locator('input[type=text]').nth(2).fill('mdi:timer-outline');
		await page.locator('label:has-text("dnsmasq") input[type=checkbox]').check();
		await saveApply(page, 'metrics: interval, change-only, group interval, hide, name, icon, service');
	} else {
		await page.goto(base + 'metrics', { waitUntil: 'networkidle' });
		await page.locator('button:has-text("Reset to legacy defaults")').click();
		await saveApply(page, 'metrics: reset to legacy defaults');
	}

	await browser.close();
	fs.closeSync(responses);
	fs.writeFileSync(path.join(out, 'report-' + phase + '.json'), JSON.stringify(report, null, 2), { mode: 0o600 });
	console.log(JSON.stringify(report, null, 2));
	process.exit(report.errors.length ? 1 : 0);
})().catch((e) => {
	// Playwright quotes the value given to fill() in its error text, and the
	// router password must not reach a terminal or a log.
	const text = String((e && e.stack) || e);
	console.error(password ? text.split(password).join('***') : text);
	process.exit(1);
});
