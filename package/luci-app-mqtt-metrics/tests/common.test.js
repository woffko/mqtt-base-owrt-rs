'use strict';
// Unit tests for the pure helpers in mqtt-metrics/common.js.
// Run: node tests/common.test.js

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const source = fs.readFileSync(path.join(__dirname, '..', 'htdocs', 'luci-static', 'resources',
	'mqtt-metrics', 'common.js'), 'utf8');
const baseclass = { extend: (o) => o };
const rpc = { declare: (d) => { const f = () => Promise.resolve(d); f.decl = d; return f; } };
const documentStub = { createTextNode: (v) => ({ nodeValue: v }) };
const mm = new Function('baseclass', 'rpc', 'document', source)(baseclass, rpc, documentStub);

const uptime = {
	key: 'system_uptime', group: 'system', default_publish: true, default_diagnostic: false,
	default_name: 'Uptime', default_object_id: 'system_uptime', default_topic: 'system/uptime',
	default_icon: 'mdi:clock-outline', legacy: true, recommended: false
};
const load = {
	key: 'system_load_1', group: 'system', default_publish: false, default_diagnostic: false,
	default_name: 'Load (1 min)', default_object_id: 'system_load_1', default_topic: 'system/load_1',
	default_icon: 'mdi:gauge', legacy: false, recommended: true
};
const cpu0 = Object.assign({}, load, { key: 'system_cpu0_usage', recommended: false });

// Defaults store nothing.
let ops = mm.overrideOptions(uptime, mm.defaultsOf(uptime));
assert.equal(ops.empty, true);
assert.deepEqual(ops.set, {});

// Hiding a legacy metric stores publish=0 only.
ops = mm.overrideOptions(uptime, Object.assign(mm.defaultsOf(uptime), { publish: false }));
assert.deepEqual(ops.set, { publish: '0' });
assert.ok(ops.unset.includes('name') && ops.unset.includes('interval'));

// Text overrides equal to the default, or empty, are not stored.
ops = mm.overrideOptions(uptime, Object.assign(mm.defaultsOf(uptime), { name: 'Uptime', topic: '', icon: 'mdi:timer', interval: ' 60 ' }));
assert.deepEqual(ops.set, { icon: 'mdi:timer', interval: '60' });

// Opt-in metric switched on.
ops = mm.overrideOptions(load, Object.assign(mm.defaultsOf(load), { publish: true, ha_enabled: false }));
assert.deepEqual(ops.set, { publish: '1', ha_enabled: '0' });

// Validators mirror the daemon.
assert.equal(mm.validTopic('openwrt'), true);
assert.equal(mm.validTopic('a/b'), true);
for (const bad of [ '', '/a', 'a/', 'a//b', 'a/+/b', 'a/#' ])
	assert.equal(mm.validTopic(bad), false, bad);
assert.equal(mm.validObjectId('openwrt_r-1'), true);
assert.equal(mm.validObjectId('bad id'), false);
assert.equal(mm.utf8Length('abc'), 3);
assert.equal(mm.utf8Length('\u00e9'), 2);
assert.equal(mm.utf8Length('\u0444'), 2);
assert.equal(mm.utf8Length('\u20ac'), 3);
assert.equal(mm.utf8Length('\ud83d\ude00'), 4);
assert.equal(mm.validName('a'.repeat(128)), true);
assert.equal(mm.validName('a'.repeat(129)), false);
// 65 Cyrillic letters are 65 characters but 130 bytes.
assert.equal(mm.validName('\u0444'.repeat(64)), true);
assert.equal(mm.validName('\u0444'.repeat(65)), false);
assert.equal(mm.validIcon(''), true);
assert.equal(mm.validIcon('mdi:router'), true);
assert.equal(mm.validIcon('router'), false);
assert.equal(mm.validInterval('', 5, 86400), true);
assert.equal(mm.validInterval('4', 5, 86400), false);
assert.equal(mm.validInterval('300', 5, 86400), true);
assert.equal(mm.validInterval('1e3', 5, 86400), false);

// Recommended set turns on recommended and legacy metrics only.
const desired = mm.legacyDefaults([ uptime, load, cpu0 ]);
assert.equal(desired.system_uptime.publish, true);
assert.equal(desired.system_load_1.publish, false);
mm.applyRecommended([ uptime, load, cpu0 ], desired);
assert.equal(desired.system_load_1.publish, true);
assert.equal(desired.system_cpu0_usage.publish, false);

// Identity changes that replace HA entities.
assert.deepEqual(mm.identityChanged({ base_topic: '', device_id: '' }, { base_topic: '', device_id: 'gw1' }), [ 'device_id' ]);
assert.deepEqual(mm.identityChanged({ node: 'x' }, { node: 'x' }), []);

// Grouping keeps order.
assert.deepEqual(Object.keys(mm.groupMetrics([ uptime, { key: 'k', group: 'dhcp' }, load ])), [ 'system', 'dhcp' ]);
assert.equal(mm.groupMetrics([ uptime, load ]).system.length, 2);

// Formatting.
assert.equal(mm.formatDuration(59), '59s');
assert.equal(mm.formatDuration(3700), '1h 1m');
assert.equal(mm.formatDuration(90000), '1d 1h');
assert.equal(mm.formatValue('12.5', '%'), '12.5 %');
assert.equal(mm.formatValue(null, '%'), '-');
assert.equal(mm.formatValue('647132216', 'B'), '647132216 B (617.2 MiB)');
assert.equal(mm.formatValue('512', 'B'), '512 B');
assert.equal(mm.formatBytes(1536), '1.5 KiB');

// Backend text is never passed to E() as a string.
assert.equal(mm.text('<b>x</b>').nodeValue, '<b>x</b>');

console.log('common.test.js: ok');
