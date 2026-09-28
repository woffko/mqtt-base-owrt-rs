'use strict';
// Static checks of the LuCI package: ACL coverage, menu targets, and that
// backend strings reach the DOM only as text nodes.
// Run: node tests/package.test.js

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const repo = path.join(root, '..', '..');
const acl = JSON.parse(fs.readFileSync(path.join(root, 'root/usr/share/rpcd/acl.d/luci-app-mqtt-metrics.json'), 'utf8'))['luci-app-mqtt-metrics'];
const menu = JSON.parse(fs.readFileSync(path.join(root, 'root/usr/share/luci/menu.d/luci-app-mqtt-metrics.json'), 'utf8'));

// Methods offered by the rpcd plugin (src/rpcd.rs `methods()`).
const rpcdSource = fs.readFileSync(path.join(repo, 'package/mqtt-metrics/src/src/rpcd.rs'), 'utf8');
const methodsBlock = rpcdSource.slice(rpcdSource.indexOf('pub fn methods()'), rpcdSource.indexOf('pub fn call('));
const methods = [ ...methodsBlock.matchAll(/^\s*"([a-z_]+)": \{/gm) ].map((m) => m[1]);
assert.ok(methods.length >= 10, 'parsed rpcd methods: ' + methods);

const read = acl.read.ubus['mqtt-metrics'];
const write = acl.write.ubus['mqtt-metrics'];
assert.deepEqual(read.slice().sort(), [ 'catalog', 'legacy', 'log', 'preview', 'status' ]);
assert.deepEqual([ ...read, ...write ].sort(), methods.slice().sort(), 'every rpcd method is in exactly one ACL list');
assert.equal(read.filter((m) => write.includes(m)).length, 0);
for (const m of [ 'set_password', 'import_legacy', 'purge', 'republish', 'reload', 'test_connection' ])
	assert.ok(write.includes(m), m + ' must require write access');
assert.deepEqual(acl.read.uci, [ 'mqtt-metrics' ]);
assert.deepEqual(acl.write.uci, [ 'mqtt-metrics' ]);
assert.equal(acl.read.file, undefined, 'no file access is granted');

// Every menu view exists, and every view is in the menu.
const viewDir = path.join(root, 'htdocs/luci-static/resources/view/mqtt-metrics');
const views = fs.readdirSync(viewDir).filter((f) => f.endsWith('.js')).map((f) => 'mqtt-metrics/' + f.slice(0, -3));
const targets = Object.values(menu).filter((e) => e.action.type === 'view').map((e) => e.action.path);
assert.deepEqual(targets.slice().sort(), views.slice().sort());

// Views and the shared module: custom buttons never submit forms, backend
// values are wrapped in text nodes, and the password is never read back.
for (const file of fs.readdirSync(viewDir).map((f) => path.join(viewDir, f)).concat(
	path.join(root, 'htdocs/luci-static/resources/mqtt-metrics/common.js'))) {
	const src = fs.readFileSync(file, 'utf8');
	for (const [ i, chunk ] of src.split("E('button', {").slice(1).entries())
		assert.match(chunk.slice(0, 120), /'type': 'button'/, `${path.basename(file)} button ${i + 1} must be type=button`);
	assert.doesNotMatch(src, /innerHTML/, path.basename(file) + ' must not use innerHTML');
	assert.doesNotMatch(src, /uci\.get\([^)]*password/, path.basename(file) + ' must not read a password from UCI');
}

console.log('package.test.js: ok');
