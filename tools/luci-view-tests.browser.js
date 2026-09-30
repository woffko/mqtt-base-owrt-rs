'use strict';
// Runs the LuCI views in headless Chromium against a small stand-in for the
// LuCI runtime (E(), _(), view, ui, uci, dom, the require loader), without a
// router. It exercises what the pure helper tests cannot: the DOM built by the
// Metrics view, the state kept behind it and the UCI writes it produces, and
// how the Diagnostics actions report a failing call.
//
// Usage: node tools/luci-view-tests.browser.js [<resources-dir>]
// The default is the package's htdocs/luci-static/resources. Passing another
// tree (for example a checkout of an older commit) shows whether a test fails
// there.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('/home/w0w/.local/share/playwright/node_modules/playwright');

const resources = process.argv[2] || path.join(__dirname, '..', 'package', 'luci-app-mqtt-metrics',
	'htdocs', 'luci-static', 'resources');
const read = (file) => fs.readFileSync(path.join(resources, file), 'utf8');

const metric = (key, group, name, extra) => Object.assign({
	key, group, name, default_name: name, instance: '', sub_instance: '',
	object_id: key, default_object_id: key,
	topic: 'openwrt/r1/' + key, topic_suffix: key, default_topic: key,
	unique_id: 'openwrt_r1_' + key, icon: null, default_icon: null, unit: null,
	value: '1', last_published: null, available: true, reason: null, permanent: false,
	publish: false, default_publish: false, discovery: true, ha_enabled: true,
	diagnostic: false, default_diagnostic: false, interval: 300, interval_override: null,
	recommended: false
}, extra || {});

const catalog = {
	config_errors: [],
	defaults: { interval: 60, min_interval: 5, max_interval: 86400 },
	groups: [
		{ id: 'system', title: 'System', enabled: true, interval: null, missing: [] },
		{ id: 'interface', title: 'Network interfaces', enabled: true, interval: null, missing: [] }
	],
	metrics: [
		metric('system_uptime', 'system', 'Uptime', { publish: true, default_publish: true, unit: 's', value: '100' }),
		metric('system_load_1', 'system', 'Load (1 min)', { recommended: true, value: '0.5' }),
		metric('system_cpu_usage', 'system', 'CPU Usage', {
			available: false, value: null, reason: 'waiting for a second sample',
			last_published: '12.5', unit: '%', publish: true, recommended: true
		}),
		metric('wan_wan_status', 'interface', 'WAN wan Status', { publish: true, default_publish: true })
	],
	interfaces: [
		{ name: 'wan', proto: 'pppoe', auto_wan: true, selected: true },
		{ name: 'wanb', proto: 'static', auto_wan: true, selected: true },
		{ name: 'lan', proto: 'static', auto_wan: false, selected: false }
	],
	devices: [ 'eth0', 'pppoe-wan' ],
	services: [ 'dnsmasq', 'ha-cluster' ],
	selected_devices: [],
	selected_services: [ 'dnsmasq' ]
};

// Runs inside the page.
async function scenario(input) {
	// --- stand-in for the LuCI runtime -------------------------------------
	String.prototype.format = function() {
		const args = arguments;
		let i = 0;
		return this.replace(/%%|%[sd]/g, (t) => t === '%%' ? '%' : String(args[i++]));
	};
	window._ = (s) => s;
	window.E = function(tag, attrs, children) {
		if (arguments.length === 2 && (typeof attrs !== 'object' || attrs instanceof Node || Array.isArray(attrs))) {
			children = attrs;
			attrs = {};
		}
		const el = document.createElement(tag);
		Object.keys(attrs || {}).forEach((k) => {
			const v = attrs[k];
			if (v == null || v === false)
				return;
			if (typeof v === 'function')
				el.addEventListener(k, v);
			else if (k === 'class')
				el.className = v;
			else
				el.setAttribute(k, v);
		});
		const add = (c) => {
			if (c == null || c === false || c === '')
				return;
			if (Array.isArray(c))
				c.forEach(add);
			else
				el.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
		};
		add(children);
		return el;
	};

	const notifications = [];
	const ui = {
		addNotification: (title, node, cls) => notifications.push({ title, text: node && node.textContent, cls }),
		createHandlerFn: (ctx, fn, ...args) => (ev) => Promise.resolve(fn.apply(ctx, args.concat([ ev ])))
	};
	const dom = {
		content: (node, children) => {
			while (node.firstChild)
				node.removeChild(node.firstChild);
			[].concat(children).forEach((c) => {
				if (c != null && c !== '')
					node.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
			});
			return node;
		}
	};

	// In-memory UCI with the operations the view uses.
	const data = JSON.parse(JSON.stringify(input.uci));   // {section: {type, options}}
	const ops = [];
	const uci = {
		load: () => Promise.resolve(),
		get: (conf, sid, opt) => {
			const s = data[sid];
			if (!s)
				return null;
			return opt == null ? Object.assign({ '.name': sid, '.type': s.type }, s.options) : (s.options[opt] == null ? null : s.options[opt]);
		},
		set: (conf, sid, opt, val) => { ops.push([ 'set', sid, opt, val ]); if (data[sid]) data[sid].options[opt] = val; },
		unset: (conf, sid, opt) => { ops.push([ 'unset', sid, opt ]); if (data[sid]) delete data[sid].options[opt]; },
		add: (conf, type, sid) => { ops.push([ 'add', sid, type ]); data[sid] = { type, options: {} }; return sid; },
		remove: (conf, sid) => { ops.push([ 'remove', sid ]); delete data[sid]; },
		sections: (conf, type) => Object.keys(data).filter((k) => data[k].type === type).map((k) => ({ '.name': k })),
		save: () => Promise.resolve()
	};

	// The real shared module.
	const mm = new Function('baseclass', 'rpc', 'document',
		input.common)({ extend: (o) => o }, { declare: (d) => () => Promise.resolve(d) }, document);
	const view = { extend: (o) => o };
	const mods = { view, ui, uci, dom, mm };

	// LuCI's loader: 'require x as y' directives become function parameters.
	const loadView = (source) => {
		const names = [];
		const re = /^'require ([\w.-]+)(?: as (\w+))?';/gm;
		let m;
		while ((m = re.exec(source)))
			names.push(m[2] || m[1].split('.').pop());
		return new Function(...names, source)(...names.map((n) => mods[n]));
	};
	const klass = loadView(input.metrics);

	window.confirm = () => true;

	// --- helpers -----------------------------------------------------------
	const results = {};
	const root = document.body;
	const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
	const section = (title) => [ ...root.querySelectorAll('.cbi-section') ].find((s) => {
		const h = s.querySelector('h3');
		return h && text(h).indexOf(title) === 0;
	});
	const field = (sectionTitle, label) => {
		const row = [ ...section(sectionTitle).querySelectorAll('.cbi-value') ].find((r) => text(r.querySelector('.cbi-value-title')) === label);
		return row.querySelector('.cbi-value-field');
	};
	const click = (label) => [ ...root.querySelectorAll('button') ].find((b) => text(b) === label).click();
	const change = (el) => el.dispatchEvent(new Event('change'));
	const typeInto = (el, value) => { el.value = value; el.dispatchEvent(new Event('input')); };
	const serviceBox = (name) => [ ...field('Selection', 'Services').querySelectorAll('label') ].find((l) => text(l) === name).querySelector('input');
	const interfaceBox = (name) => [ ...field('Selection', 'Interfaces').querySelectorAll('label') ].find((l) => text(l).indexOf(name + ' ') === 0).querySelector('input');
	const snapshot = () => ({
		interval: field('Publishing', 'Global interval (s)').querySelector('input').value,
		changeOnly: field('Publishing', 'Publish only changes').querySelector('input').checked,
		mode: field('Selection', 'Interfaces').querySelector('select').value,
		services: [ ...field('Selection', 'Services').querySelectorAll('input') ].filter((i) => i.checked).map((i) => text(i.parentNode)),
		interfaces: [ ...field('Selection', 'Interfaces').querySelectorAll('label') ].filter((l) => l.querySelector('input').checked).map((l) => text(l).split(' ')[0])
	});

	// --- scenario ----------------------------------------------------------
	const view1 = Object.create(klass);
	const dataIn = await view1.load();
	root.appendChild(view1.render([ input.catalog, dataIn[1] ]));

	results.initial = snapshot();

	// A metric whose value needs a second sample still shows what was published.
	const cpuRow = root.querySelector('tr[data-key="system_cpu_usage"]');
	results.cpuValue = text(cpuRow.children[3]);
	results.uptimeValue = text(root.querySelector('tr[data-key="system_uptime"]').children[3]);

	// Reset to defaults, then look at the form and change it again.
	click('Reset to defaults');
	results.afterReset = snapshot();
	serviceBox('ha-cluster').checked = true;
	change(serviceBox('ha-cluster'));
	typeInto(field('Publishing', 'Global interval (s)').querySelector('input'), '120');
	results.afterEdit = snapshot();

	ops.length = 0;
	await view1.handleSave();
	results.ops = ops.map((o) => o.join(' '));

	// Switching to manual selection and back keeps a consistent list.
	const modeSelect = field('Selection', 'Interfaces').querySelector('select');
	modeSelect.value = 'manual';
	change(modeSelect);
	results.manualInterfaces = snapshot().interfaces;
	interfaceBox('lan').checked = true;
	change(interfaceBox('lan'));
	ops.length = 0;
	await view1.handleSave();
	results.manualOps = ops.map((o) => o.join(' '));

	// Diagnostics: a call that fails must be reported, not swallowed.
	mm.callPurge = () => Promise.reject(new Error('RPC call timed out'));
	const diagnostics = Object.create(loadView(input.diagnostics));
	root.appendChild(diagnostics.render([ {}, {}, {}, { owned_topics: 3 } ]));
	notifications.length = 0;
	click('Reset retained topics');
	await new Promise((resolve) => setTimeout(resolve, 50));
	results.purgeNotifications = notifications.map((n) => n.text + ' [' + n.cls + ']');
	return results;
}

(async () => {
	const browser = await chromium.launch({ headless: true,
		executablePath: '/home/w0w/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome' });
	const page = await browser.newPage();
	const errors = [];
	page.on('pageerror', (e) => errors.push(e.message));
	await page.setContent('<!doctype html><html><body></body></html>');
	const r = await page.evaluate(scenario, {
		common: read('mqtt-metrics/common.js'),
		metrics: read('view/mqtt-metrics/metrics.js'),
		diagnostics: read('view/mqtt-metrics/diagnostics.js'),
		catalog,
		uci: {
			publish: { type: 'publish', options: { interval: '60', change_only: '1', max_age: '3600' } },
			select: { type: 'select', options: { interfaces: 'manual', interface: [ 'lan' ], service: [ 'dnsmasq' ] } }
		}
	}).finally(() => browser.close());

	assert.deepEqual(errors, [], 'page errors');

	// Loaded from the stored configuration.
	assert.equal(r.initial.interval, '60');
	assert.equal(r.initial.changeOnly, true);
	assert.equal(r.initial.mode, 'manual');
	assert.deepEqual(r.initial.services, [ 'dnsmasq' ]);
	assert.deepEqual(r.initial.interfaces, [ 'lan' ]);

	// Values: a fresh one, and the last published one when only one sample exists.
	assert.equal(r.uptimeValue, '100 s');
	assert.match(r.cpuValue, /^12\.5 %\s*\(last published\)$/);

	// "Reset to defaults" must show what Save will write.
	assert.equal(r.afterReset.interval, '300', 'reset: global interval');
	assert.equal(r.afterReset.changeOnly, false, 'reset: publish only changes');
	assert.equal(r.afterReset.mode, 'auto', 'reset: interface selection');
	assert.deepEqual(r.afterReset.services, [], 'reset: services');
	assert.deepEqual(r.afterReset.interfaces, [ 'wan', 'wanb' ], 'reset: the automatically selected interfaces are shown');

	// Edits made after the reset are the ones that get saved.
	assert.deepEqual(r.afterEdit.services, [ 'ha-cluster' ]);
	assert.ok(r.ops.includes('set publish interval 120'), r.ops.join('\n'));
	assert.ok(r.ops.includes('set select interfaces auto'), r.ops.join('\n'));
	assert.ok(r.ops.includes('set select service ha-cluster'), 'service ticked after the reset is saved: ' + r.ops.join('\n'));
	assert.ok(!r.ops.some((o) => o.startsWith('set select interface ')), 'auto mode stores no interface list');

	// Manual mode stores the list.
	assert.deepEqual(r.manualInterfaces, [ 'wan', 'wanb' ]);
	assert.ok(r.manualOps.includes('set select interface wan,wanb,lan'), r.manualOps.join('\n'));

	// A failing purge call is shown to the user.
	assert.equal(r.purgeNotifications.length, 1, 'purge failure: ' + r.purgeNotifications.join('\n'));
	assert.match(r.purgeNotifications[0], /Reset retained topics: .*timed out \[danger\]$/);

	console.log('luci-view-tests: ok');
})().catch((e) => {
	console.error(e.message || e);
	process.exit(1);
});
