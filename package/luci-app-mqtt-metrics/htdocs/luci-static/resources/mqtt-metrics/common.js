'use strict';
'require baseclass';
'require rpc';

// Shared RPC declarations and pure helpers for the MQTT Metrics views.
// Functions without DOM or RPC access are unit-tested in
// tests/common.test.js.

// `reject: true`: a failed call (rpcd without the plugin, a timeout) rejects.
// Without it rpc.js resolves the bare ubus status code, and the views would
// show that number as if it were the reply.
var callStatus = rpc.declare({ object: 'mqtt-metrics', method: 'status', reject: true });
var callCatalog = rpc.declare({ object: 'mqtt-metrics', method: 'catalog', reject: true });
var callPreview = rpc.declare({ object: 'mqtt-metrics', method: 'preview', params: [ 'key' ], reject: true });
var callLegacy = rpc.declare({ object: 'mqtt-metrics', method: 'legacy', reject: true });
var callLog = rpc.declare({ object: 'mqtt-metrics', method: 'log', reject: true });
var callTest = rpc.declare({
	object: 'mqtt-metrics',
	method: 'test_connection',
	params: [ 'host', 'port', 'username', 'password', 'keep_password', 'tls', 'tls_verify' ],
	reject: true
});
var callSetPassword = rpc.declare({ object: 'mqtt-metrics', method: 'set_password', params: [ 'password' ], reject: true });
var callImportLegacy = rpc.declare({ object: 'mqtt-metrics', method: 'import_legacy', reject: true });
var callRepublish = rpc.declare({ object: 'mqtt-metrics', method: 'republish', reject: true });
var callPurge = rpc.declare({ object: 'mqtt-metrics', method: 'purge', reject: true });
var callReload = rpc.declare({ object: 'mqtt-metrics', method: 'reload', reject: true });

var BOOL_FIELDS = [ 'publish', 'discovery', 'ha_enabled', 'diagnostic' ];
var TEXT_FIELDS = [ 'name', 'object_id', 'topic', 'icon' ];

// The message of a failed call. LuCI's RPC errors carry the stack after the
// first line of the message, which is of no use to the person reading it.
function errorText(err) {
	return String((err && err.message) ? err.message : err).split('\n')[0];
}

// Text node for backend data: LuCI's E() parses string children as HTML.
function text(value) {
	return document.createTextNode(value == null ? '' : String(value));
}

// Defaults of a catalog entry, i.e. the values without any UCI override.
function defaultsOf(m) {
	return {
		publish: !!m.default_publish,
		discovery: true,
		ha_enabled: true,
		diagnostic: !!m.default_diagnostic,
		name: m.default_name || '',
		object_id: m.default_object_id || '',
		topic: m.default_topic || '',
		icon: m.default_icon || '',
		interval: ''
	};
}

// Current effective settings of a catalog entry as edited in the form.
function currentOf(m) {
	var topicSuffix = m.default_topic || '';
	return {
		publish: !!m.publish,
		discovery: !!m.discovery,
		ha_enabled: !!m.ha_enabled,
		diagnostic: !!m.diagnostic,
		name: m.name || '',
		object_id: m.object_id || '',
		topic: m.topic_suffix != null ? m.topic_suffix : topicSuffix,
		icon: m.icon || '',
		interval: m.interval_override != null ? String(m.interval_override) : ''
	};
}

// UCI changes for one metric section: options to set and to remove so that
// only values that differ from the defaults are stored.
function overrideOptions(m, desired) {
	var def = defaultsOf(m), set = {}, unset = [];
	BOOL_FIELDS.forEach(function(f) {
		if (desired[f] == null || !!desired[f] === def[f])
			unset.push(f);
		else
			set[f] = desired[f] ? '1' : '0';
	});
	TEXT_FIELDS.forEach(function(f) {
		var v = desired[f] == null ? '' : String(desired[f]).trim();
		if (v === '' || v === def[f])
			unset.push(f);
		else
			set[f] = v;
	});
	var iv = desired.interval == null ? '' : String(desired.interval).trim();
	if (iv === '')
		unset.push('interval');
	else
		set.interval = iv;
	return { set: set, unset: unset, empty: Object.keys(set).length === 0 };
}

// Topic prefixes and suffixes: no wildcards, no empty levels.
function validTopic(value) {
	if (typeof value !== 'string' || value === '' || value.length > 1024)
		return false;
	if (/[+#\u0000]/.test(value))
		return false;
	return value.split('/').every(function(level) { return level !== ''; });
}

// The daemon limits a display name to 128 bytes of UTF-8 and refuses the whole
// configuration otherwise, so the form has to count bytes, not characters.
function utf8Length(value) {
	var n = 0;
	for (var i = 0; i < value.length; i++) {
		var c = value.charCodeAt(i);
		if (c < 0x80)
			n += 1;
		else if (c < 0x800)
			n += 2;
		else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < value.length) {
			n += 4;
			i++;
		}
		else
			n += 3;
	}
	return n;
}

function validName(value) {
	return typeof value === 'string' && utf8Length(value) <= 128;
}

function validObjectId(value) {
	return typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}

function validIcon(value) {
	return value === '' || /^mdi:[a-z0-9-]+$/.test(value);
}

function validInterval(value, min, max) {
	if (value === '' || value == null)
		return true;
	if (!/^[0-9]+$/.test(String(value)))
		return false;
	var n = +value;
	return n >= min && n <= max;
}

// Catalog metrics grouped by group id, keeping catalog order.
function groupMetrics(metrics) {
	var out = {};
	(metrics || []).forEach(function(m) {
		(out[m.group] = out[m.group] || []).push(m);
	});
	return out;
}

// Desired settings after the "Recommended set" action: legacy and
// recommended metrics published, nothing else changed.
function applyRecommended(metrics, desired) {
	(metrics || []).forEach(function(m) {
		if (m.recommended || m.legacy)
			desired[m.key].publish = true;
	});
	return desired;
}

// Desired settings after "Reset to legacy defaults": every override removed.
function legacyDefaults(metrics) {
	var desired = {};
	(metrics || []).forEach(function(m) {
		desired[m.key] = defaultsOf(m);
	});
	return desired;
}

// Identity settings whose change replaces Home Assistant entities.
var IDENTITY_KEYS = [ 'base_topic', 'node', 'discovery_prefix', 'device_id' ];

function identityChanged(before, after) {
	return IDENTITY_KEYS.filter(function(k) {
		return (before[k] || '') !== (after[k] || '');
	});
}

function formatDuration(seconds) {
	if (seconds == null || isNaN(seconds))
		return '-';
	seconds = Math.max(0, Math.floor(seconds));
	var d = Math.floor(seconds / 86400), h = Math.floor(seconds % 86400 / 3600),
	    m = Math.floor(seconds % 3600 / 60), s = seconds % 60;
	if (d > 0)
		return d + 'd ' + h + 'h';
	if (h > 0)
		return h + 'h ' + m + 'm';
	if (m > 0)
		return m + 'm ' + s + 's';
	return s + 's';
}

function formatTime(unix) {
	if (!unix)
		return '-';
	var d = new Date(unix * 1000);
	function p(n) { return (n < 10 ? '0' : '') + n; }
	return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate()) + ' ' +
		p(d.getHours()) + ':' + p(d.getMinutes()) + ':' + p(d.getSeconds());
}

function formatBytes(n) {
	var units = [ 'B', 'KiB', 'MiB', 'GiB', 'TiB' ], i = 0;
	while (n >= 1024 && i < units.length - 1) {
		n /= 1024;
		i++;
	}
	return (i ? n.toFixed(1) : String(n)) + ' ' + units[i];
}

function formatValue(value, unit) {
	if (value == null || value === '')
		return '-';
	if (unit === 'B' && /^[0-9]+$/.test(String(value)) && +value >= 1024)
		return value + ' B (' + formatBytes(+value) + ')';
	return unit ? value + ' ' + unit : String(value);
}

return baseclass.extend({
	callStatus: callStatus,
	callCatalog: callCatalog,
	callPreview: callPreview,
	callLegacy: callLegacy,
	callLog: callLog,
	callTest: callTest,
	callSetPassword: callSetPassword,
	callImportLegacy: callImportLegacy,
	callRepublish: callRepublish,
	callPurge: callPurge,
	callReload: callReload,
	text: text,
	errorText: errorText,
	defaultsOf: defaultsOf,
	currentOf: currentOf,
	overrideOptions: overrideOptions,
	validTopic: validTopic,
	utf8Length: utf8Length,
	validName: validName,
	validObjectId: validObjectId,
	validIcon: validIcon,
	validInterval: validInterval,
	groupMetrics: groupMetrics,
	applyRecommended: applyRecommended,
	legacyDefaults: legacyDefaults,
	identityChanged: identityChanged,
	IDENTITY_KEYS: IDENTITY_KEYS,
	formatDuration: formatDuration,
	formatTime: formatTime,
	formatValue: formatValue,
	formatBytes: formatBytes
});
