'use strict';
'require view';
'require ui';
'require uci';
'require dom';
'require mqtt-metrics.common as mm';

var CONF = 'mqtt-metrics';

function checkbox(checked, onchange, title) {
	var cb = E('input', { 'type': 'checkbox', 'title': title || '' });
	cb.checked = !!checked;
	cb.addEventListener('change', function() { onchange(cb.checked); });
	return cb;
}

function input(value, placeholder, onchange, width) {
	var el = E('input', {
		'type': 'text',
		'class': 'cbi-input-text',
		'style': 'width:' + (width || '100%') + ';min-width:4em',
		'placeholder': placeholder == null ? '' : String(placeholder)
	});
	el.value = value == null ? '' : String(value);
	el.addEventListener('input', function() { onchange(el.value); });
	return el;
}

function button(label, style, handler) {
	return E('button', { 'type': 'button', 'class': 'cbi-button cbi-button-' + style, 'click': handler }, [ mm.text(label) ]);
}

function overrideOnly(effective, def) {
	return (effective == null || effective === def) ? '' : effective;
}

return view.extend({
	load: function() {
		return Promise.all([ mm.callCatalog(), uci.load(CONF) ]);
	},

	initState: function(catalog) {
		var self = this;
		this.catalog = catalog;
		this.metrics = catalog.metrics || [];
		this.byKey = {};
		this.desired = {};
		this.metrics.forEach(function(m) {
			var def = mm.defaultsOf(m);
			self.byKey[m.key] = m;
			self.desired[m.key] = {
				publish: !!m.publish,
				discovery: !!m.discovery,
				ha_enabled: !!m.ha_enabled,
				diagnostic: !!m.diagnostic,
				name: overrideOnly(m.name, def.name),
				object_id: overrideOnly(m.object_id, def.object_id),
				topic: overrideOnly(m.topic_suffix, def.topic),
				icon: overrideOnly(m.icon || '', def.icon),
				interval: m.interval_override != null ? String(m.interval_override) : ''
			};
		});
		this.groups = {};
		(catalog.groups || []).forEach(function(g) {
			self.groups[g.id] = { enabled: g.enabled !== false, interval: g.interval != null ? String(g.interval) : '' };
		});
		this.publish = {
			interval: uci.get(CONF, 'publish', 'interval') || '300',
			change_only: uci.get(CONF, 'publish', 'change_only') === '1',
			max_age: uci.get(CONF, 'publish', 'max_age') || '3600'
		};
		var list = function(opt) {
			var v = uci.get(CONF, 'select', opt);
			return Array.isArray(v) ? v.slice() : (v ? [ v ] : []);
		};
		this.select = {
			mode: uci.get(CONF, 'select', 'interfaces') || 'auto',
			interfaces: list('interface'),
			devices: list('device'),
			services: list('service')
		};
		if (this.select.mode === 'auto')
			this.select.interfaces = (catalog.interfaces || []).filter(function(i) { return i.selected; }).map(function(i) { return i.name; });
		this.filter = '';
		this.onlyPublished = false;
	},

	renderPublishing: function() {
		var self = this, d = this.catalog.defaults || {};
		return E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, [ mm.text(_('Publishing')) ]),
			E('div', { 'class': 'cbi-section-descr' }, [ mm.text(_('The legacy script published every value every 300 s. Intervals of groups and single metrics override the global interval.')) ]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Global interval (s)')) ]),
				E('div', { 'class': 'cbi-value-field' }, [
					input(this.publish.interval, '300', function(v) { self.publish.interval = v; }, '8em'),
					E('div', { 'class': 'cbi-value-description' }, [ mm.text(_('From %d to %d seconds.').format(d.min_interval || 5, d.max_interval || 86400)) ])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Publish only changes')) ]),
				E('div', { 'class': 'cbi-value-field' }, [
					checkbox(this.publish.change_only, function(v) { self.publish.change_only = v; }),
					E('div', { 'class': 'cbi-value-description' }, [ mm.text(_('Values are still read at their interval but sent only when they change, and at least once per maximum age.')) ])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Maximum age (s)')) ]),
				E('div', { 'class': 'cbi-value-field' }, [ input(this.publish.max_age, '3600', function(v) { self.publish.max_age = v; }, '8em') ])
			])
		]);
	},

	legacyInterfaces: function() {
		return (this.catalog.interfaces || []).filter(function(i) { return i.legacy_wan; }).map(function(i) { return i.name; });
	},

	renderChoiceList: function(items, selected, disabled) {
		return E('div', { 'style': 'display:flex;flex-wrap:wrap;gap:.3em 1.2em' }, items.map(function(item) {
			var cb = checkbox(selected.indexOf(item.name) >= 0, function(v) {
				var i = selected.indexOf(item.name);
				if (v && i < 0)
					selected.push(item.name);
				else if (!v && i >= 0)
					selected.splice(i, 1);
			});
			cb.disabled = !!disabled;
			return E('label', { 'style': 'white-space:nowrap' }, [ cb, ' ', mm.text(item.label) ]);
		}));
	},

	renderSelection: function() {
		var self = this, c = this.catalog;
		var ifaceItems = (c.interfaces || []).map(function(i) {
			return { name: i.name, label: i.name + (i.proto ? ' (' + i.proto + ')' : '') + (i.legacy_wan ? ' *' : '') };
		});
		var ifaceBox = E('div');
		var drawIfaces = function() {
			dom.content(ifaceBox, self.renderChoiceList(ifaceItems, self.select.interfaces, self.select.mode === 'auto'));
		};
		drawIfaces();
		var mode = E('select', { 'class': 'cbi-input-select', 'style': 'width:auto;max-width:100%' }, [
			E('option', { 'value': 'auto' }, [ mm.text(_('Automatic: WAN interfaces (wan*, wwan*), like the legacy script')) ]),
			E('option', { 'value': 'manual' }, [ mm.text(_('Manual selection')) ])
		]);
		mode.value = this.select.mode;
		mode.addEventListener('change', function() {
			self.select.mode = mode.value;
			if (mode.value === 'auto')
				self.select.interfaces = self.legacyInterfaces();
			drawIfaces();
		});

		var deviceItems = (c.devices || []).map(function(d) { return { name: d, label: d }; });
		var serviceItems = (c.services || []).map(function(s) { return { name: s, label: s }; });
		return E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, [ mm.text(_('Selection')) ]),
			E('div', { 'class': 'cbi-section-descr' }, [ mm.text(_('Metrics of newly selected instances appear after Save & Apply. * marks interfaces matched by the legacy WAN rule, which keep the legacy entity ids.')) ]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Interfaces')) ]),
				E('div', { 'class': 'cbi-value-field' }, [ mode, ifaceBox ])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Traffic devices')) ]),
				E('div', { 'class': 'cbi-value-field' }, [
					this.renderChoiceList(deviceItems, this.select.devices, false),
					E('div', { 'class': 'cbi-value-description' }, [ mm.text(_('None selected: the devices of the selected interfaces.')) ])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, [ mm.text(_('Services')) ]),
				E('div', { 'class': 'cbi-value-field' }, [
					serviceItems.length ? this.renderChoiceList(serviceItems, this.select.services, false) : mm.text(_('No procd services found.')),
					E('div', { 'class': 'cbi-value-description' }, [ mm.text(_('Selected services get a "running" entity.')) ])
				])
			])
		]);
	},

	renderDetails: function(m, d) {
		var self = this, def = mm.defaultsOf(m);
		var preview = E('pre', { 'style': 'display:none;white-space:pre-wrap;word-break:break-all;max-height:20em;overflow:auto' });
		return E('div', { 'style': 'padding:.5em 0 .5em 1em' }, [
			E('div', { 'style': 'display:grid;grid-template-columns:minmax(8em,auto) 1fr;gap:.3em .8em;align-items:center' }, [
				mm.text(_('Object ID')), input(d.object_id, def.object_id, function(v) { d.object_id = v; }),
				mm.text(_('Topic suffix')), input(d.topic, def.topic, function(v) { d.topic = v; }),
				mm.text(_('Icon')), input(d.icon, def.icon || 'mdi:...', function(v) { d.icon = v; }),
				mm.text(_('Unique ID')), E('code', {}, [ mm.text(m.unique_id) ]),
				mm.text(_('State topic')), E('code', { 'style': 'word-break:break-all' }, [ mm.text(m.topic) ]),
				mm.text(_('Enabled by default in HA')), checkbox(d.ha_enabled, function(v) { d.ha_enabled = v; }),
				mm.text(_('Diagnostic entity')), checkbox(d.diagnostic, function(v) { d.diagnostic = v; })
			]),
			E('div', { 'style': 'margin-top:.4em' }, [
				button(_('Preview discovery'), 'action', function() {
					return mm.callPreview(m.key).then(function(res) {
						preview.style.display = '';
						dom.content(preview, mm.text(res.error ? res.error : (res.config_topic + '\n' + JSON.stringify(res.payload, null, 2))));
					});
				})
			]),
			preview
		]);
	},

	renderRow: function(m) {
		var self = this, d = this.desired[m.key], def = mm.defaultsOf(m);
		var value;
		if (m.available)
			value = mm.text(mm.formatValue(m.value, m.unit));
		else if (m.last_published != null && m.last_published !== '')
			// Rates and CPU need two samples; this page has only one.
			value = E('span', { 'title': m.reason || '' }, [
				mm.text(mm.formatValue(m.last_published, m.unit) + ' '),
				E('em', { 'style': 'opacity:.7' }, [ mm.text('(' + _('last published') + ')') ])
			]);
		else
			value = E('em', { 'style': 'opacity:.7' }, [ mm.text(m.reason || _('unavailable')) ]);
		var details = E('tr', { 'class': 'tr', 'style': 'display:none' }, [ E('td', { 'class': 'td', 'colspan': 6 }) ]);
		var detailsBuilt = false;
		var row = E('tr', { 'class': 'tr', 'data-key': m.key }, [
			E('td', { 'class': 'td', 'data-title': _('Publish') }, [ checkbox(d.publish, function(v) { d.publish = v; }, _('Publish')) ]),
			E('td', { 'class': 'td', 'data-title': _('HA') }, [ checkbox(d.discovery, function(v) { d.discovery = v; }, _('Show in Home Assistant')) ]),
			E('td', { 'class': 'td', 'data-title': _('Name') }, [
				input(d.name, def.name, function(v) { d.name = v; }),
				m.legacy ? E('small', { 'style': 'opacity:.7' }, [ mm.text(_('legacy')) ]) : (m.recommended ? E('small', { 'style': 'opacity:.7' }, [ mm.text(_('recommended')) ]) : '')
			]),
			E('td', { 'class': 'td', 'data-title': _('Value'), 'style': 'word-break:break-all' }, [ value ]),
			E('td', { 'class': 'td', 'data-title': _('Interval (s)') }, [ input(d.interval, String(m.interval), function(v) { d.interval = v; }, '6em') ]),
			E('td', { 'class': 'td', 'data-title': '' }, [
				button('…', 'neutral', function() {
					if (!detailsBuilt) {
						dom.content(details.firstChild, self.renderDetails(m, d));
						detailsBuilt = true;
					}
					details.style.display = details.style.display === 'none' ? '' : 'none';
				})
			])
		]);
		row._search = (m.name + ' ' + m.key + ' ' + m.instance + ' ' + m.sub_instance).toLowerCase();
		row._details = details;
		return [ row, details ];
	},

	renderGroup: function(g) {
		var self = this, metrics = this.grouped[g.id] || [], gs = this.groups[g.id];
		var rows = [];
		metrics.forEach(function(m) { rows = rows.concat(self.renderRow(m)); });
		var table = E('table', { 'class': 'table' }, [
			E('tr', { 'class': 'tr table-titles' }, [
				E('th', { 'class': 'th' }, [ mm.text(_('Publish')) ]),
				E('th', { 'class': 'th' }, [ mm.text(_('HA')) ]),
				E('th', { 'class': 'th' }, [ mm.text(_('Name')) ]),
				E('th', { 'class': 'th' }, [ mm.text(_('Value')) ]),
				E('th', { 'class': 'th' }, [ mm.text(_('Interval (s)')) ]),
				E('th', { 'class': 'th' }, [])
			])
		].concat(rows));
		var setAll = function(v) {
			metrics.forEach(function(m) { self.desired[m.key].publish = v; });
			table.querySelectorAll('tr[data-key]').forEach(function(tr) {
				tr.querySelector('input[type=checkbox]').checked = v;
			});
		};
		var missing = (g.missing || []).map(function(x) {
			return E('li', {}, [ mm.text(x.name + ': ' + x.reason) ]);
		});
		var published = metrics.filter(function(m) { return self.desired[m.key].publish; }).length;
		var body = E('div', { 'style': published ? '' : 'display:none' }, [
			E('div', { 'style': 'display:flex;flex-wrap:wrap;gap:.5em 1.5em;align-items:center;margin:.3em 0' }, [
				E('label', {}, [ checkbox(gs.enabled, function(v) { gs.enabled = v; }), ' ', mm.text(_('Collect this group')) ]),
				E('label', {}, [ mm.text(_('Group interval (s)') + ' '), input(gs.interval, this.publish.interval, function(v) { gs.interval = v; }, '6em') ]),
				button(_('Publish all'), 'action', function() { setAll(true); }),
				button(_('Publish none'), 'neutral', function() { setAll(false); })
			]),
			missing.length ? E('ul', { 'class': 'cbi-section-descr' }, missing) : '',
			metrics.length ? table : ''
		]);
		var summary = E('span', { 'style': 'opacity:.7' }, [ mm.text(' ' + _('%d of %d published').format(published, metrics.length)) ]);
		var header = E('h3', { 'style': 'cursor:pointer' }, [ mm.text(g.title), summary ]);
		header.addEventListener('click', function() {
			body.style.display = body.style.display === 'none' ? '' : 'none';
		});
		return E('div', { 'class': 'cbi-section', 'data-group': g.id }, [ header, body ]);
	},

	applyFilter: function() {
		var f = this.filter.toLowerCase(), self = this;
		this.root.querySelectorAll('tr[data-key]').forEach(function(tr) {
			var show = (!f || tr._search.indexOf(f) >= 0) && (!self.onlyPublished || self.desired[tr.getAttribute('data-key')].publish);
			tr.style.display = show ? '' : 'none';
			if (!show)
				tr._details.style.display = 'none';
		});
		if (f || this.onlyPublished)
			this.root.querySelectorAll('[data-group] > div').forEach(function(b) { b.style.display = ''; });
	},

	// The Publishing and Selection sections keep references into this.publish
	// and this.select; draw them again whenever those objects are replaced.
	drawSections: function() {
		dom.content(this.publishBox, this.renderPublishing());
		dom.content(this.selectBox, this.renderSelection());
	},

	redrawGroups: function() {
		var self = this;
		dom.content(this.groupBox, (this.catalog.groups || []).map(function(g) { return self.renderGroup(g); }));
		this.applyFilter();
	},

	render: function(data) {
		var self = this;
		this.initState(data[0] || {});
		this.grouped = mm.groupMetrics(this.metrics);
		this.groupBox = E('div');
		this.publishBox = E('div');
		this.selectBox = E('div');

		var search = input('', _('Filter by name, key or instance'), function(v) { self.filter = v; self.applyFilter(); }, '20em');
		var tools = E('div', { 'class': 'cbi-section' }, [
			E('div', { 'style': 'display:flex;flex-wrap:wrap;gap:.5em 1em;align-items:center' }, [
				search,
				E('label', {}, [ checkbox(false, function(v) { self.onlyPublished = v; self.applyFilter(); }), ' ', mm.text(_('Only published')) ]),
				button(_('Recommended set'), 'action', function() {
					mm.applyRecommended(self.metrics, self.desired);
					self.redrawGroups();
					ui.addNotification(null, E('p', {}, [ mm.text(_('Recommended metrics are selected. Save & Apply to publish them.')) ]), 'info');
				}),
				button(_('Reset to legacy defaults'), 'negative', function() {
					if (!window.confirm(_('Remove every per-metric override and group setting, restore the 300 s interval and the automatic interface selection?')))
						return;
					self.desired = mm.legacyDefaults(self.metrics);
					Object.keys(self.groups).forEach(function(g) { self.groups[g] = { enabled: true, interval: '' }; });
					self.publish = { interval: '300', change_only: false, max_age: '3600' };
					self.select = { mode: 'auto', interfaces: self.legacyInterfaces(), devices: [], services: [] };
					self.resetRequested = true;
					ui.addNotification(null, E('p', {}, [ mm.text(_('Legacy defaults restored in the form. Save & Apply to use them.')) ]), 'info');
					self.drawSections();
					self.redrawGroups();
				})
			])
		]);

		this.root = E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, [ mm.text(_('MQTT Metrics: Metrics')) ]),
			E('div', { 'class': 'cbi-map-descr' }, [ mm.text(_('Choose what is published and what Home Assistant shows. Hiding a metric removes its entity; values that a router cannot provide are listed with the reason.')) ]),
			(this.catalog.config_errors || []).length ? E('div', { 'class': 'alert-message warning' },
				this.catalog.config_errors.map(function(e) { return E('div', {}, [ mm.text(e) ]); })) : '',
			this.publishBox,
			this.selectBox,
			tools,
			this.groupBox
		]);
		this.drawSections();
		this.redrawGroups();
		return this.root;
	},

	validate: function() {
		var d = this.catalog.defaults || {}, min = d.min_interval || 5, max = d.max_interval || 86400;
		var errors = [], self = this;
		if (!mm.validInterval(this.publish.interval, min, max) || this.publish.interval === '')
			errors.push(_('Global interval must be from %d to %d seconds').format(min, max));
		if (!mm.validInterval(this.publish.max_age, min, 7 * max))
			errors.push(_('Maximum age is out of range'));
		Object.keys(this.groups).forEach(function(g) {
			if (!mm.validInterval(self.groups[g].interval, min, max))
				errors.push(_('Interval of group %s is out of range').format(g));
		});
		Object.keys(this.desired).forEach(function(key) {
			var x = self.desired[key];
			if (!mm.validInterval(x.interval, min, max))
				errors.push(_('%s: interval is out of range').format(key));
			if (x.name && !mm.validName(x.name))
				errors.push(_('%s: the name must be at most 128 bytes long').format(key));
			if (x.object_id && !mm.validObjectId(x.object_id))
				errors.push(_('%s: object ID may contain only letters, digits, "_" and "-"').format(key));
			if (x.topic && !mm.validTopic(x.topic))
				errors.push(_('%s: invalid topic suffix').format(key));
			if (x.icon && !mm.validIcon(x.icon))
				errors.push(_('%s: icon must look like mdi:name').format(key));
		});
		return errors;
	},

	handleSave: function(ev) {
		var self = this, errors = this.validate();
		if (errors.length) {
			ui.addNotification(_('Not saved'), E('ul', {}, errors.map(function(e) { return E('li', {}, [ mm.text(e) ]); })), 'danger');
			return Promise.reject(new Error('invalid input'));
		}

		var ensure = function(sid, type) {
			if (!uci.get(CONF, sid))
				uci.add(CONF, type, sid);
		};
		var setOrUnset = function(sid, opt, value) {
			if (value == null || value === '' || (Array.isArray(value) && !value.length))
				uci.unset(CONF, sid, opt);
			else
				uci.set(CONF, sid, opt, value);
		};

		ensure('publish', 'publish');
		uci.set(CONF, 'publish', 'interval', String(this.publish.interval));
		uci.set(CONF, 'publish', 'change_only', this.publish.change_only ? '1' : '0');
		uci.set(CONF, 'publish', 'max_age', String(this.publish.max_age));

		ensure('select', 'select');
		uci.set(CONF, 'select', 'interfaces', this.select.mode);
		setOrUnset('select', 'interface', this.select.mode === 'manual' ? this.select.interfaces.slice() : []);
		setOrUnset('select', 'device', this.select.devices.slice());
		setOrUnset('select', 'service', this.select.services.slice());

		Object.keys(this.groups).forEach(function(g) {
			var gs = self.groups[g];
			if (gs.enabled && gs.interval === '') {
				if (uci.get(CONF, g))
					uci.remove(CONF, g);
				return;
			}
			ensure(g, 'group');
			uci.set(CONF, g, 'enabled', gs.enabled ? '1' : '0');
			setOrUnset(g, 'interval', gs.interval);
		});

		Object.keys(this.desired).forEach(function(key) {
			var ops = mm.overrideOptions(self.byKey[key], self.desired[key]);
			if (ops.empty) {
				if (uci.get(CONF, key))
					uci.remove(CONF, key);
				return;
			}
			ensure(key, 'metric');
			Object.keys(ops.set).forEach(function(opt) { uci.set(CONF, key, opt, ops.set[opt]); });
			ops.unset.forEach(function(opt) { uci.unset(CONF, key, opt); });
		});

		if (this.resetRequested) {
			// Also drop overrides of instances that are not in the catalog now.
			(uci.sections(CONF, 'metric') || []).forEach(function(s) {
				if (!self.byKey[s['.name']])
					uci.remove(CONF, s['.name']);
			});
			this.resetRequested = false;
		}

		return uci.save();
	},

	handleSaveApply: function(ev, mode) {
		return this.handleSave(ev).then(function() {
			ui.changes.apply(mode == '0');
		});
	},

	handleReset: function() {
		window.location.reload();
	}
});
