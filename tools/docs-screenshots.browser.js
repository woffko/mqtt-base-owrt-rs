'use strict';
// Screenshots for the documentation. The LuCI pages run on the real LuCI
// client code and the Bootstrap theme of an OpenWrt SDK in headless Chromium.
// Every request is answered here: files from disk, a ubus/uci mock and the
// menu. The data comes from the sanitized fixtures in tests/fixtures/primary
// and is made generic before use (host name, model, interface and service
// names); a check refuses to take a picture that shows anything from the list
// of forbidden strings. Nothing is sent to any host.
//
// Usage: node tools/docs-screenshots.browser.js [<sdk-dir>] [<out-dir>]
// The daemon binary is taken from $MQTT_METRICSD or the debug build.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { chromium } = require('/home/w0w/.local/share/playwright/node_modules/playwright');

const REPO = path.join(__dirname, '..');
const SDK = process.argv[2] || process.env.SDK ||
	'/home/w0w/cake-autorate-rs-owrt/openwrt-sdk-25.12.5-x86-64_gcc-14.3.0_musl.Linux-x86_64';
const OUT = process.argv[3] || path.join(REPO, 'docs', 'screenshots');
const LUCI = SDK + '/feeds/luci';
const RESOURCES = [
	REPO + '/package/luci-app-mqtt-metrics/htdocs/luci-static/resources',
	LUCI + '/themes/luci-theme-bootstrap/htdocs/luci-static/resources',
	LUCI + '/modules/luci-base/htdocs/luci-static/resources'
];
const MEDIA = LUCI + '/themes/luci-theme-bootstrap/htdocs/luci-static/bootstrap';
const BINARY = process.env.MQTT_METRICSD || REPO + '/package/mqtt-metrics/src/target/debug/mqtt-metricsd';
const FIXTURES = REPO + '/tests/fixtures/primary';
// The origin shows in the Configuration URL placeholder.
const ORIGIN = 'http://openwrt.lan';

const HOST = 'router';
// Strings that must not appear in a picture: names of the test fixtures, the
// address ranges of the test equipment and home networks. Personal words
// (host names, user names) go into the file named by $DOCS_FORBIDDEN, one
// regular expression per line, which stays out of the repository.
const FORBIDDEN = [ /router-primary/, /router-secondary/, /QEMU/i, /wifi87/, /bond0/, /\bsync\b/,
	/10\.0\.77\./, /192\.168\./, /owsync/, /lease-sync/ ].concat(
	process.env.DOCS_FORBIDDEN ? fs.readFileSync(process.env.DOCS_FORBIDDEN, 'utf8').split('\n')
		.map((l) => l.trim()).filter(Boolean).map((l) => new RegExp(l, 'i')) : []);

// --- anonymous data -----------------------------------------------------

function generic(text) {
	return text
		.replace(/router-primary/g, HOST)
		.replace(/QEMU Standard PC \(Q35 \+ ICH9, 2009\)/g, 'Generic x86/64')
		.replace(/qemu-standard-pc-q35-ich9-2009/g, 'generic')
		.replace(/25\.12-SNAPSHOT/g, '25.12.6')
		.replace(/r0\+\d+-[0-9a-f]+/g, 'r30000-0123456789');
}

const catalog = JSON.parse(generic(execFileSync(BINARY, [ '--catalog', '--fixtures', FIXTURES ], { encoding: 'utf8' })));
const dump = generic(execFileSync(BINARY, [ '--dump', '--fixtures', FIXTURES ], { encoding: 'utf8' }))
	.split('\n').filter(Boolean).map((l) => JSON.parse(l));

catalog.interfaces = catalog.interfaces.filter((i) => [ 'wan', 'wan_6', 'wanb', 'wanb_6', 'lan', 'wireguard' ].includes(i.name));
catalog.devices = [ 'br-lan', 'eth0', 'eth1', 'eth2', 'eth3', 'pppoe-wan', 'wireguard' ];
catalog.services = [ 'cron', 'dnsmasq', 'dropbear', 'firewall', 'log', 'mqtt-metrics', 'network', 'odhcpd', 'rpcd', 'sysntpd', 'uhttpd' ];
const dropped = new Set([ 'lease-sync', 'owsync', 'wifi', 'wifi_v6', 'lan_v6' ]);
catalog.metrics = catalog.metrics.filter((m) => !(m.group === 'ha' && dropped.has(m.instance)));
// A typical selection: the default set plus the recommended system values
// and the state of the two multi-WAN uplinks. Rates and CPU need a second sample; give them a value.
const fake = { system_cpu_usage: '6.4', system_cpu0_usage: '8.1', system_cpu1_usage: '5.2', system_cpu2_usage: '7.7', system_cpu3_usage: '4.6' };
// The release packages are built with TLS.
catalog.tls_supported = true;
catalog.metrics.forEach((m) => {
	if (m.recommended && m.group === 'system')
		m.publish = true;
	if (m.group === 'mwan3' && [ 'wan', 'wanb' ].includes(m.instance) && !m.sub_instance &&
		[ 'mwan3.status', 'mwan3.online', 'mwan3.score' ].includes(m.template))
		m.publish = true;
	if (fake[m.key] != null) {
		m.value = fake[m.key];
		m.available = true;
		m.reason = null;
	}
});

const now = 1792834200; // 2026-10-24 09:30 UTC
const published = catalog.metrics.filter((m) => m.publish && m.value != null);
const status = {
	state: 'connected', running: true, pid: 4242, since: now - 86400 * 3, updated: now,
	broker: '192.0.2.10:1883', last_connect: now - 86400 * 3, last_publish: now - 42,
	messages: 18234, publish_errors: 0, reconnects: 0, last_error: null, birth_denied: false,
	config_errors: [], owned_topics: published.length * 2 + 1,
	device: {
		id: catalog.identity.device_id, name: catalog.identity.device_name, hostname: HOST,
		base_topic: catalog.identity.base_topic, availability_topic: catalog.identity.availability_topic,
		client_id: catalog.identity.client_id, discovery: true
	},
	metrics: published.map((m) => ({
		key: m.key, group: m.group, name: m.name, topic: m.topic, value: m.value, unit: m.unit,
		published_at: now - 42, discovery: m.discovery
	}))
};
const log = { lines: [
	`Sat Oct 24 08:00:01 2026 daemon.info mqtt-metricsd[4242]: info: mqtt-metrics 0.1.0 starting`,
	`Sat Oct 24 08:00:01 2026 daemon.info mqtt-metricsd[4242]: info: configuration loaded: broker 192.0.2.10:1883, base topic openwrt/${HOST}, discovery on`,
	`Sat Oct 24 08:00:01 2026 daemon.info mqtt-metricsd[4242]: info: connected to 192.0.2.10:1883 as mqtt-metrics-${HOST}`,
	`Sat Oct 24 08:12:37 2026 daemon.info mqtt-metricsd[4242]: info: Home Assistant came online; resending discovery`
] };

function preview(key) {
	const m = catalog.metrics.find((x) => x.key === key);
	if (!m)
		return { error: 'metric ' + key + ' is not available on this router' };
	const config = dump.find((p) => p.topic.endsWith('/' + m.unique_id + '/config'));
	return {
		key, config_topic: config ? config.topic : '', state_topic: m.topic, announced: !!config,
		payload: config ? JSON.parse(config.payload) : null
	};
}

function uciFile() {
	const values = {};
	let cur = null, idx = 0;
	const text = fs.readFileSync(REPO + '/package/mqtt-metrics/files/etc/config/mqtt-metrics', 'utf8');
	for (let line of text.split('\n')) {
		line = line.trim();
		let m;
		if ((m = line.match(/^config\s+(\S+)\s+'([^']*)'$/))) {
			cur = m[2];
			values[cur] = { '.type': m[1], '.name': cur, '.anonymous': false, '.index': idx++ };
		} else if ((m = line.match(/^option\s+(\S+)\s+'([^']*)'$/)) && cur) {
			if (m[2] !== '')
				values[cur][m[1]] = m[2];
		}
	}
	Object.assign(values.broker, { host: '192.0.2.10', username: 'homeassistant', password: 'example-password' });
	return values;
}
const uci = { 'mqtt-metrics': uciFile() };

// --- menu, as menu.d and the base modules would build it ----------------

const node = (title, order, action, children) => Object.assign({ title, order, satisfied: true, action }, children ? { children } : {});
const appMenu = JSON.parse(fs.readFileSync(REPO + '/package/luci-app-mqtt-metrics/root/usr/share/luci/menu.d/luci-app-mqtt-metrics.json', 'utf8'));
const tabs = {};
for (const [ p, n ] of Object.entries(appMenu)) {
	const name = p.split('/')[3];
	if (name)
		tabs[name] = node(n.title, n.order, n.action);
}
const menu = { satisfied: true, children: { admin: node('Administration', 10, { type: 'firstchild' }, {
	status: node('Status', 10, { type: 'firstchild' }, { overview: node('Overview', 1, { type: 'view', path: 'status/index' }) }),
	system: node('System', 20, { type: 'firstchild' }, { system: node('System', 1, { type: 'view', path: 'system/system' }) }),
	services: node('Services', 40, { type: 'firstchild' }, {
		'mqtt-metrics': node('MQTT Metrics', 60, { type: 'firstchild' }, tabs)
	}),
	network: node('Network', 50, { type: 'firstchild' }, { network: node('Interfaces', 10, { type: 'view', path: 'network/network' }) }),
	logout: node('Log out', 999, { type: 'function' })
}) } };

// --- mock server --------------------------------------------------------

function ubus(object, method, args) {
	if (object === 'session' && method === 'access')
		return [ 0, { access: true } ];
	if (object === 'uci') {
		const cfg = uci[args.config];
		// Other configurations (luci, system) are read by LuCI itself.
		if (method === 'get')
			return [ 0, { values: JSON.parse(JSON.stringify(cfg || {})) } ];
		if (method === 'changes')
			return [ 0, { changes: {} } ];
		return [ 0 ];
	}
	if (object === 'luci' && method === 'getFeatures')
		return [ 0, {} ];
	if (object === 'mqtt-metrics') {
		switch (method) {
		case 'catalog': return [ 0, catalog ];
		case 'status': return [ 0, status ];
		case 'log': return [ 0, log ];
		case 'preview': return [ 0, preview(args.key) ];
		case 'test_connection': return [ 0, { ok: true, message: 'connected to 192.0.2.10:1883', duration_ms: 18 } ];
		}
	}
	return [ 4 ];
}

function page(view) {
	const name = view.split('/').pop();
	const env = {
		media: '/luci-static/bootstrap', resource: '/luci-static/resources', scriptname: '/cgi-bin/luci',
		pathinfo: '/admin/services/mqtt-metrics/' + name, documentroot: '/www',
		requestpath: [ 'admin', 'services', 'mqtt-metrics', name ],
		dispatchpath: [ 'admin', 'services', 'mqtt-metrics', name ],
		pollinterval: 5, ubuspath: '/ubus/', sessionid: '0123456789abcdef0123456789abcdef', token: 'token',
		nodespec: { satisfied: true, readonly: false, title: name }, apply_rollback: 90, apply_holdoff: 4,
		apply_timeout: 5, apply_display: 1.5, rollback_token: null
	};
	return `<!DOCTYPE html>
<html lang="en" data-darkmode="false">
<head>
<meta charset="utf-8">
<title>${HOST} | MQTT Metrics</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/luci-static/bootstrap/cascade.css">
<link rel="stylesheet" media="only screen and (max-device-width: 854px)" href="/luci-static/bootstrap/mobile.css">
<script src="/luci-static/resources/cbi.js"></script>
<script src="/luci-static/resources/luci.js"></script>
<script>L = new LuCI(${JSON.stringify(env)});</script>
</head>
<body class="lang_en" data-page="admin-services-mqtt-metrics-${name}">
<header>
	<a class="brand" href="#">${HOST}</a>
	<ul class="nav" id="topmenu" style="display:none"></ul>
	<div id="indicators" class="pull-right"></div>
</header>
<div id="maincontent" class="container">
	<div id="tabmenu" style="display:none"></div>
	<div id="view"><div class="spinning">Loading view…</div>
	<script>L.require('ui').then(function(ui) { ui.instantiateView('${view}'); });</script></div>
</div>
<footer>
	<span>Powered by <a href="#">LuCI openwrt-25.12 branch</a> / <a href="#">OpenWrt 25.12.6</a></span>
	<ul class="breadcrumb pull-right" id="modemenu" style="display:none"></ul>
</footer>
<script>L.require('menu-bootstrap')</script>
</body>
</html>`;
}

function contentType(file) {
	return { '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png' }[path.extname(file)] || 'application/octet-stream';
}

async function open(browser, view, viewport, mobile) {
	const context = await browser.newContext({ viewport, deviceScaleFactor: mobile ? 2 : 1, isMobile: mobile, hasTouch: mobile, colorScheme: 'light' });
	const pg = await context.newPage();
	const errors = [];
	pg.on('pageerror', (e) => errors.push(String(e.message || e)));
	await pg.route('**/*', async (route) => {
		const url = new URL(route.request().url());
		if (url.origin !== ORIGIN)
			return route.abort('blockedbyclient');
		const p = url.pathname;
		if (p === '/')
			return route.fulfill({ status: 200, contentType: 'text/html', body: page(view) });
		if (p === '/cgi-bin/luci/admin/menu')
			return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(menu) });
		if (p.startsWith('/ubus')) {
			let body = null;
			try { body = route.request().postDataJSON(); } catch (_) { body = null; }
			const one = (req) => req.method === 'call'
				? { jsonrpc: '2.0', id: req.id, result: ubus(req.params[1], req.params[2], req.params[3] || {}) }
				: { jsonrpc: '2.0', id: req.id, result: {} };
			const reply = Array.isArray(body) ? body.map(one) : one(body || {});
			return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(reply) });
		}
		const roots = p.startsWith('/luci-static/bootstrap/') ? [ [ '/luci-static/bootstrap/', MEDIA ] ]
			: RESOURCES.map((r) => [ '/luci-static/resources/', r ]);
		for (const [ prefix, root ] of roots) {
			if (!p.startsWith(prefix))
				continue;
			const file = path.join(root, p.slice(prefix.length));
			if (file.startsWith(root) && fs.existsSync(file) && fs.statSync(file).isFile())
				return route.fulfill({ status: 200, contentType: contentType(file), body: fs.readFileSync(file) });
		}
		return route.fulfill({ status: 404, body: 'not found' });
	});
	await pg.goto(ORIGIN + '/', { waitUntil: 'domcontentloaded' });
	await pg.waitForSelector('#view .cbi-map, #view .cbi-section', { timeout: 20000 });
	await pg.waitForSelector('#tabmenu .tabs', { timeout: 20000 });
	await pg.waitForTimeout(800);
	return { context, pg, errors };
}

async function shoot(browser, view, file, opts) {
	opts = opts || {};
	const viewport = opts.mobile ? { width: 390, height: 844 } : { width: 1280, height: opts.height || 900 };
	const { context, pg, errors } = await open(browser, view, viewport, !!opts.mobile);
	try {
		if (opts.before)
			await opts.before(pg);
		const text = await pg.evaluate(() => document.documentElement.innerText + ' ' +
			Array.from(document.querySelectorAll('input')).map((i) => i.value).join(' '));
		for (const re of FORBIDDEN)
			if (re.test(text))
				throw new Error(file + ': the page shows ' + re);
		if (errors.length)
			throw new Error(file + ': ' + errors.join('; '));
		if (opts.clip) {
			// The header is fixed and would cover a part below the top.
			await pg.addStyleTag({ content: 'header { position: static !important; } body { padding-top: 0 !important; }' });
			await pg.waitForTimeout(200);
			const clip = await pg.evaluate(opts.clip);
			await pg.screenshot({ path: path.join(OUT, file), fullPage: true, clip });
		} else
			await pg.screenshot({ path: path.join(OUT, file), fullPage: !opts.top });
		console.log('docs-screenshots: ' + file);
	} finally {
		await context.close();
	}
}

(async () => {
	fs.mkdirSync(OUT, { recursive: true });
	for (const re of FORBIDDEN) {
		if (re.test(JSON.stringify(catalog)) || re.test(JSON.stringify(status)))
			throw new Error('the data still contains ' + re);
	}
	const browser = await chromium.launch({ headless: true,
		executablePath: '/home/w0w/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome', args: [ '--no-sandbox' ] });
	try {
		await shoot(browser, 'mqtt-metrics/status', 'status.png');
		await shoot(browser, 'mqtt-metrics/broker', 'broker.png');
		await shoot(browser, 'mqtt-metrics/topics', 'topics.png');
		await shoot(browser, 'mqtt-metrics/metrics', 'metrics.png', { top: true, height: 1500 });
		await shoot(browser, 'mqtt-metrics/metrics', 'metrics-details.png', { clip: () => {
			// From the group heading to the end of the expanded detail row.
			const top = document.querySelector('[data-group="system"]').getBoundingClientRect().top + window.scrollY;
			const row = document.querySelector('tr[data-key="system_uptime"]').nextElementSibling;
			const bottom = row.getBoundingClientRect().bottom + window.scrollY;
			return { x: 0, y: Math.max(0, top - 2), width: document.documentElement.clientWidth, height: bottom - top + 22 };
		}, before: async (pg) => {
			// The detail row of one metric, with its discovery preview.
			const row = pg.locator('tr[data-key="system_uptime"]');
			await row.locator('button').last().click();
			await pg.waitForTimeout(200);
			await pg.locator('button:has-text("Preview discovery")').first().click();
			await pg.waitForTimeout(400);
		} });
		await shoot(browser, 'mqtt-metrics/diagnostics', 'diagnostics.png', { before: async (pg) => {
			await pg.selectOption('#view select', 'system_uptime');
			await pg.click('#view button:has-text("Show")');
			await pg.waitForTimeout(400);
		} });
		await shoot(browser, 'mqtt-metrics/status', 'mobile-status.png', { mobile: true, top: true });
		await shoot(browser, 'mqtt-metrics/metrics', 'mobile-metrics.png', { mobile: true, top: true });
	} finally {
		await browser.close();
	}
})().catch((e) => {
	console.error(String(e && e.stack || e));
	process.exit(1);
});
