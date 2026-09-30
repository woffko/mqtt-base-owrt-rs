'use strict';
// Runs the LuCI views on top of the REAL LuCI client code (luci.js, ui.js,
// form.js, rpc.js, validation.js ... taken from an OpenWrt SDK) in headless
// Chromium. Every request is answered by Playwright: files from disk and a
// small ubus/uci mock. Nothing leaves this machine.
//
// tools/luci-view-tests.browser.js runs the Metrics view against a stand-in
// for the LuCI runtime; this one exercises what depends on the real form,
// validation and rpc code: the form validators, the map re-rendering after a
// save, rejected RPC calls.
//
// Usage: node tools/luci-real.browser.js [<sdk-dir>]
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('/home/w0w/.local/share/playwright/node_modules/playwright');

const REPO = path.join(__dirname, '..');
const SDK = process.argv[2] || process.env.SDK ||
	'/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64';
const LUCI_RES = SDK + '/feeds/luci/modules/luci-base/htdocs/luci-static/resources';
const APP_RES = REPO + '/package/luci-app-mqtt-metrics/htdocs/luci-static/resources';
const ORIGIN = 'http://luci.test';

const catalog = {
	config_errors: [],
	identity: { hostname: 'r1', base_topic: 'openwrt/r1', device_id: 'openwrt_r1', client_id: 'mqtt-metrics-r1' },
	password_set: true,
	tls_supported: true,
	defaults: { interval: 60, min_interval: 5, max_interval: 86400 },
	groups: [],
	metrics: [],
	interfaces: [],
	devices: [],
	services: []
};

function parseUciFile(text) {
	const values = {};
	let cur = null, idx = 0;
	for (let line of text.split('\n')) {
		line = line.trim();
		let m;
		if ((m = line.match(/^config\s+(\S+)\s+'([^']*)'$/))) {
			cur = m[2];
			values[cur] = { '.type': m[1], '.name': cur, '.anonymous': false, '.index': idx++ };
		} else if ((m = line.match(/^option\s+(\S+)\s+'([^']*)'$/)) && cur) {
			values[cur][m[1]] = m[2];
		} else if ((m = line.match(/^list\s+(\S+)\s+'([^']*)'$/)) && cur) {
			(values[cur][m[1]] = values[cur][m[1]] || []).push(m[2]);
		}
	}
	return values;
}

function defaultUci() {
	return parseUciFile(fs.readFileSync(REPO + '/package/mqtt-metrics/files/etc/config/mqtt-metrics', 'utf8'));
}

async function start(opts) {
	const view = opts.view;
	const state = {
		uci: { 'mqtt-metrics': defaultUci(), luci: {}, system: {} },
		uciWrites: [],
		errors: [],
		handlers: opts.handlers || {}
	};
	let indexCounter = 100;

	function uciReply(method, a) {
		const cfg = state.uci[a.config];
		switch (method) {
		case 'get':
			return cfg ? [ 0, { values: JSON.parse(JSON.stringify(cfg)) } ] : [ 4 ];
		case 'changes':
			return [ 0, { changes: {} } ];
		case 'set': {
			state.uciWrites.push({ method, args: a });
			const s = cfg && cfg[a.section];
			if (!s)
				return [ 4 ];
			for (const [ k, v ] of Object.entries(a.values || {})) {
				if (v === '' || v == null)
					delete s[k];
				else
					s[k] = v;
			}
			return [ 0 ];
		}
		case 'add': {
			state.uciWrites.push({ method, args: a });
			const name = a.name || 'cfg' + (indexCounter++).toString(16);
			if (cfg[name])
				return [ 2 ];
			cfg[name] = Object.assign({ '.type': a.type, '.name': name, '.anonymous': !a.name, '.index': indexCounter++ }, a.values || {});
			return [ 0, { section: name } ];
		}
		case 'delete': {
			state.uciWrites.push({ method, args: a });
			const s = cfg && cfg[a.section];
			if (!s)
				return [ 4 ];
			if (a.options == null)
				delete cfg[a.section];
			else
				for (const o of a.options)
					delete s[o];
			return [ 0 ];
		}
		case 'order':
			return [ 0 ];
		case 'apply':
		case 'confirm':
			state.uciWrites.push({ method, args: a });
			return [ 0 ];
		}
		return [ 3 ];
	}

	function callUbus(sid, object, method, args) {
		args = args || {};
		if (state.handlers[object + '.' + method])
			return state.handlers[object + '.' + method](args, state);
		if (object === 'session' && method === 'access')
			return [ 0, { access: true } ];
		if (object === 'uci')
			return uciReply(method, args);
		if (object === 'luci' && method === 'getFeatures')
			return [ 0, {} ];
		if (object === 'file' && method === 'list')
			return [ 0, { entries: [] } ];
		return [ 4 ];
	}

	function ubusRequest(req) {
		if (req.method === 'list')
			return { jsonrpc: '2.0', id: req.id, result: {} };
		if (req.method === 'call') {
			const [ sid, object, method, args ] = req.params;
			return { jsonrpc: '2.0', id: req.id, result: callUbus(sid, object, method, args) };
		}
		return { jsonrpc: '2.0', id: req.id, error: { code: -32601, message: 'Method not found' } };
	}

	const name = view.split('/').pop();
	const env = {
		media: '/luci-static/bootstrap',
		resource: '/luci-static/resources',
		scriptname: '/cgi-bin/luci',
		pathinfo: '/admin/services/mqtt-metrics/' + name,
		documentroot: '/www',
		requestpath: [ 'admin', 'services', 'mqtt-metrics', name ],
		dispatchpath: [ 'admin', 'services', 'mqtt-metrics', name ],
		pollinterval: 5,
		ubuspath: '/ubus/',
		sessionid: '0123456789abcdef0123456789abcdef',
		token: 'testtoken',
		nodespec: { satisfied: true, readonly: false, title: 'x' },
		apply_rollback: 90, apply_holdoff: 4, apply_timeout: 5, apply_display: 1.5,
		rollback_token: null
	};
	const indexHtml = `<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><title>t</title>
<script src="/luci-static/resources/cbi.js"></script>
<script src="/luci-static/resources/luci.js?v=1"></script>
<script>L = new LuCI(${JSON.stringify(env)});</script>
</head><body>
<header><ul class="nav" id="topmenu" style="display:none"></ul><div id="indicators"></div></header>
<div id="maincontent" class="container"><div id="tabmenu" style="display:none"></div>
<div id="view"><div class="spinning">Loading view…</div>
<script>L.require('ui').then(function(ui) { ui.instantiateView('${view}'); });</script></div></div>
</body></html>`;

	const browser = await chromium.launch({ headless: true,
		executablePath: '/home/w0w/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome', args: [ '--no-sandbox' ] });
	const page = await (await browser.newContext({ viewport: { width: 1280, height: 900 } })).newPage();
	page.setDefaultTimeout(15000);
	page.on('pageerror', (e) => state.errors.push(String(e.message || e)));
	page.on('dialog', (d) => d.accept());
	await page.route('**/*', async (route) => {
		const url = new URL(route.request().url());
		if (url.origin !== ORIGIN)
			return route.abort('blockedbyclient');
		const p = url.pathname;
		if (p === '/' || p === '/index.html')
			return route.fulfill({ status: 200, contentType: 'text/html', body: indexHtml });
		if (p.startsWith('/ubus') || p.startsWith('/cgi-bin/luci/admin/ubus')) {
			let body = null;
			try { body = route.request().postDataJSON(); } catch (_) { body = null; }
			const reply = Array.isArray(body) ? body.map(ubusRequest) : (body ? ubusRequest(body) : { jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } });
			return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(reply) });
		}
		if (p.startsWith('/luci-static/resources/')) {
			const rel = p.slice('/luci-static/resources/'.length);
			for (const base of [ APP_RES, LUCI_RES ]) {
				const f = path.join(base, rel);
				if (f.startsWith(base) && fs.existsSync(f) && fs.statSync(f).isFile())
					return route.fulfill({ status: 200, contentType: 'application/javascript', body: fs.readFileSync(f) });
			}
		}
		return route.fulfill({ status: 404, body: 'not found ' + p });
	});
	await page.goto(ORIGIN + '/', { waitUntil: 'domcontentloaded' });
	await page.waitForSelector('.cbi-map, .alert-message', { timeout: 15000 });
	await page.waitForTimeout(300);
	return { browser, page, state };
}

const field = (option, section) => '[data-field="cbid.mqtt-metrics.' + (section || 'broker') + '.' + option + '"]';
const invalid = (page, option) => page.$eval(field(option) + ' input', (e) => e.classList.contains('cbi-input-invalid'));
const typeInto = async (page, selector, value) => {
	await page.fill(selector, value);
	await page.press(selector, 'Tab');
	await page.waitForTimeout(150);
};
const alerts = (page) => page.$$eval('.alert-message, .modal', (els) => els.map((e) => e.textContent.replace(/\s+/g, ' ').trim()));

const scenarios = {
	// The identity banner is put into the map by hand; the map renders again
	// after a save and used to drop it for good.
	async topics() {
		const h = await start({ view: 'mqtt-metrics/topics', handlers: { 'mqtt-metrics.catalog': () => [ 0, catalog ] } });
		try {
			const shown = () => h.page.$eval('.alert-message.warning', (e) => getComputedStyle(e).display).catch(() => 'missing');
			assert.equal(await shown(), 'none', 'hidden at first');
			await typeInto(h.page, field('node', 'identity') + ' input', 'newnode');
			assert.equal(await shown(), 'block', 'shown after an edit');
			await h.page.click('button.cbi-button-save');
			await h.page.waitForTimeout(800);
			assert.equal(await shown(), 'none', 'still there after Save, and the baseline is the saved value');
			await typeInto(h.page, field('device_id', 'identity') + ' input', 'abc');
			assert.equal(await shown(), 'block', 'shown after an edit that follows a save');
			assert.deepEqual(h.state.errors, []);
		} finally {
			await h.browser.close();
		}
	},

	// Validators of the real form code, a failing call, the password field.
	async broker() {
		const h = await start({ view: 'mqtt-metrics/broker', handlers: {
			'mqtt-metrics.catalog': () => [ 0, catalog ],
			'mqtt-metrics.test_connection': () => [ 6 ]
		} });
		try {
			const { page } = h;
			// A browser must not fill the router login into the broker credentials.
			assert.equal(await page.getAttribute(field('_password') + ' input', 'autocomplete'), 'new-password');
			assert.equal(await page.getAttribute(field('username') + ' input', 'autocomplete'), 'off');

			// The daemon refuses port 0 and a keepalive with a fraction.
			for (const [ option, value, bad ] of [
				[ 'port', '0', true ], [ 'port', '1883', false ], [ 'port', '65536', true ], [ 'port', '1.5', true ],
				[ 'keepalive', '60.5', true ], [ 'keepalive', '60', false ], [ 'keepalive', '4', true ]
			]) {
				await typeInto(page, field(option) + ' input', value);
				assert.equal(await invalid(page, option), bad, option + ' = ' + value);
			}

			// A call that fails is shown, not swallowed.
			await page.click(field('_test') + ' button');
			await page.waitForTimeout(500);
			const text = (await alerts(page)).join(' | ');
			assert.match(text, /Connection failed: RPC call to mqtt-metrics\/test_connection failed with ubus code 6/, text);
			assert.doesNotMatch(text, /handleCallReply/, 'no stack in the message');
			assert.deepEqual(h.state.errors, []);
		} finally {
			await h.browser.close();
		}
	},

	// The catalog is the only source of TLS support and of the password state.
	async broker_without_the_service() {
		const h = await start({ view: 'mqtt-metrics/broker', handlers: {
			'mqtt-metrics.catalog': () => [ 7 ]
		} });
		try {
			const { page } = h;
			const descr = await page.$eval('.cbi-map-descr', (e) => e.textContent);
			assert.match(descr, /did not answer/);
			const password = await page.$eval(field('_password'), (e) => e.textContent);
			assert.match(password, /unknown/);
			const tls = await page.$eval(field('tls'), (e) => e.textContent);
			assert.doesNotMatch(tls, /no TLS support/);
			await page.click(field('tls') + ' input[type="checkbox"]');
			await page.click('button.cbi-button-save');
			await page.waitForTimeout(800);
			assert.ok(h.state.uciWrites.some((w) => w.args.values && w.args.values.tls === '1'), 'TLS can be switched on');
			assert.deepEqual(h.state.errors, []);
		} finally {
			await h.browser.close();
		}
	},

	// Failed calls on the Diagnostics page.
	async diagnostics() {
		const h = await start({ view: 'mqtt-metrics/diagnostics', handlers: {
			'mqtt-metrics.catalog': () => [ 0, catalog ],
			'mqtt-metrics.log': () => [ 0, { lines: [] } ],
			'mqtt-metrics.status': () => [ 0, { owned_topics: 3 } ],
			'mqtt-metrics.reload': () => [ 6 ]
		} });
		try {
			const { page } = h;
			const body = await page.$eval('.cbi-map', (e) => e.textContent.replace(/\s+/g, ' '));
			assert.match(body, /Actions/);
			assert.doesNotMatch(body, /publish_metrics|[Ll]egacy/, 'no trace of the old script');
			await page.click('button:has-text("Reload configuration")');
			await page.waitForTimeout(500);
			const text = (await alerts(page)).join(' | ');
			assert.match(text, /Reload: RPC call to mqtt-metrics\/reload failed with ubus code 6/, text);
			assert.deepEqual(h.state.errors, []);
		} finally {
			await h.browser.close();
		}
	},

	// A healthy service renders the Status page.
	async status() {
		const h = await start({ view: 'mqtt-metrics/status', handlers: {
			'mqtt-metrics.status': () => [ 0, {
				state: 'connected', running: true, since: 1, broker: '192.0.2.10:1883', messages: 5, metrics: [],
				device: { id: 'x', name: 'n', base_topic: 'b', availability_topic: 'b/availability', client_id: 'c', discovery: true },
				owned_topics: 3, config_errors: []
			} ],
			'mqtt-metrics.test_connection': () => [ 6 ]
		} });
		try {
			const { page } = h;
			const overview = await page.$eval('table.table', (e) => e.textContent.replace(/\s+/g, ' '));
			assert.match(overview, /Service\s*Connected/i);
			assert.match(overview, /192\.0\.2\.10:1883/);
			await page.click('button:has-text("Test connection")');
			await page.waitForTimeout(500);
			assert.match((await alerts(page)).join(' | '), /Test connection: RPC call to mqtt-metrics\/test_connection failed/);
			assert.deepEqual(h.state.errors, []);
		} finally {
			await h.browser.close();
		}
	}
};

module.exports = { start, catalog };

if (require.main === module) (async () => {
	if (!fs.existsSync(LUCI_RES + '/form.js')) {
		console.error('no LuCI client code in ' + LUCI_RES + '; pass the SDK directory as the argument');
		process.exit(2);
	}
	const only = process.env.SCENARIO;
	let failed = 0;
	for (const [ name, run ] of Object.entries(scenarios)) {
		if (only && only !== name)
			continue;
		try {
			await run();
			console.log('luci-real: ' + name + ' ok');
		} catch (e) {
			failed++;
			console.error('luci-real: ' + name + ' FAILED: ' + (e && e.message || e));
		}
	}
	process.exit(failed ? 1 : 0);
})();
