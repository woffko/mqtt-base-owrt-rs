'use strict';
'require view';
'require poll';
'require dom';
'require ui';
'require mqtt-metrics.common as mm';

var STATE_LABELS = {
	connected: [ _('Connected'), 'success' ],
	connecting: [ _('Connecting'), 'notice' ],
	disconnected: [ _('Disconnected'), 'danger' ],
	unconfigured: [ _('No broker configured'), 'notice' ],
	disabled: [ _('Disabled'), 'notice' ],
	config_error: [ _('Configuration error'), 'danger' ],
	starting: [ _('Starting'), 'notice' ],
	stopped: [ _('Stopped'), 'danger' ]
};

var GROUP_TITLES = {
	system: _('System'),
	interface: _('Network interfaces'),
	traffic: _('Traffic'),
	mwan3: _('Multi-WAN (mwan3)'),
	ha: _('HA cluster'),
	dhcp: _('DHCP'),
	wireguard: _('WireGuard'),
	wifi: _('Wi-Fi'),
	service: _('Services'),
	packages: _('Packages')
};

function badge(state) {
	var l = STATE_LABELS[state] || [ state || _('Unknown'), 'notice' ];
	return E('span', { 'class': 'label ' + l[1], 'style': 'font-size:100%' }, [ mm.text(l[0]) ]);
}

function row(label, value) {
	return E('tr', { 'class': 'tr' }, [
		E('td', { 'class': 'td left', 'width': '33%' }, [ mm.text(label) ]),
		E('td', { 'class': 'td left' }, [ value instanceof Node ? value : mm.text(value) ])
	]);
}

return view.extend({
	load: function() {
		return mm.callStatus();
	},

	renderOverview: function(st) {
		var dev = st.device || {};
		var rows = [
			row(_('Service'), badge(st.running === false ? 'stopped' : st.state)),
			row(_('Broker'), st.broker || '-'),
			row(_('State since'), mm.formatTime(st.since)),
			row(_('Last connect'), mm.formatTime(st.last_connect)),
			row(_('Last publish'), mm.formatTime(st.last_publish)),
			row(_('Messages sent'), st.messages != null ? String(st.messages) : '-'),
			row(_('Publish errors'), st.publish_errors != null ? String(st.publish_errors) : '-'),
			row(_('Reconnect attempts'), st.reconnects != null ? String(st.reconnects) : '-')
		];
		if (st.last_error)
			rows.push(row(_('Last error'), E('span', { 'style': 'color:#c44' }, [ mm.text(st.last_error) ])));
		if (st.birth_denied)
			rows.push(row(_('Birth topic'), E('span', { 'style': 'color:#c44' }, [
				mm.text(_('The broker denied the subscription. Discovery is not resent when Home Assistant restarts; allow the topic in the broker ACL or turn the subscription off on the Topics page.'))
			])));
		if (st.next_attempt && st.state !== 'connected')
			rows.push(row(_('Next attempt'), mm.formatTime(st.next_attempt)));
		if (dev.id) {
			rows.push(row(_('Device ID'), dev.id));
			rows.push(row(_('Device name'), dev.name));
			rows.push(row(_('Base topic'), dev.base_topic));
			rows.push(row(_('Availability topic'), dev.availability_topic));
			rows.push(row(_('Client ID'), dev.client_id));
			rows.push(row(_('Home Assistant discovery'), dev.discovery ? _('On') : _('Off (shadow mode)')));
			rows.push(row(_('Retained topics owned'), String(st.owned_topics || 0)));
		}
		var errors = (st.config_errors || []).map(function(e) {
			return E('li', {}, [ mm.text(e) ]);
		});
		return E('div', {}, [
			errors.length ? E('div', { 'class': 'alert-message warning' }, [
				E('strong', {}, [ mm.text(_('The configuration is invalid; the service does not publish until it is fixed:')) ]),
				E('ul', {}, errors)
			]) : '',
			E('table', { 'class': 'table' }, rows)
		]);
	},

	renderMetrics: function(st) {
		var metrics = st.metrics || [];
		if (!metrics.length)
			return E('p', { 'class': 'cbi-section-descr' }, [ mm.text(_('Nothing is published yet.')) ]);
		var groups = mm.groupMetrics(metrics);
		var out = [];
		Object.keys(GROUP_TITLES).forEach(function(g) {
			if (!groups[g])
				return;
			out.push(E('h4', {}, [ mm.text(GROUP_TITLES[g]) ]));
			out.push(E('table', { 'class': 'table' }, [
				E('tr', { 'class': 'tr table-titles' }, [
					E('th', { 'class': 'th' }, [ mm.text(_('Metric')) ]),
					E('th', { 'class': 'th' }, [ mm.text(_('Value')) ]),
					E('th', { 'class': 'th' }, [ mm.text(_('Topic')) ]),
					E('th', { 'class': 'th' }, [ mm.text(_('Published')) ]),
					E('th', { 'class': 'th' }, [ mm.text(_('HA')) ])
				])
			].concat(groups[g].map(function(m) {
				return E('tr', { 'class': 'tr' }, [
					E('td', { 'class': 'td', 'data-title': _('Metric') }, [ mm.text(m.name) ]),
					E('td', { 'class': 'td', 'data-title': _('Value') }, [ mm.text(mm.formatValue(m.value, m.unit)) ]),
					E('td', { 'class': 'td', 'data-title': _('Topic'), 'style': 'word-break:break-all' }, [ mm.text(m.topic) ]),
					E('td', { 'class': 'td', 'data-title': _('Published') }, [ mm.text(mm.formatTime(m.published_at)) ]),
					E('td', { 'class': 'td', 'data-title': _('HA') }, [ mm.text(m.discovery ? _('yes') : _('no')) ])
				]);
			}))));
		});
		return E('div', {}, out);
	},

	action: function(call, title) {
		return function(ev) {
			var btn = ev.currentTarget;
			btn.disabled = true;
			return call().then(function(res) {
				var ok = res && res.ok;
				ui.addNotification(null, E('p', {}, [ mm.text(title + ': ' + (res && (res.message || res.error) || (ok ? _('done') : _('failed')))) ]),
					ok ? 'info' : 'danger');
			}).catch(function(err) {
				ui.addNotification(null, E('p', {}, [ mm.text(title + ': ' + err) ]), 'danger');
			}).finally(function() {
				btn.disabled = false;
			});
		};
	},

	render: function(st) {
		var self = this;
		var overview = E('div', {}, [ this.renderOverview(st) ]);
		var metrics = E('div', {}, [ this.renderMetrics(st) ]);

		poll.add(function() {
			return mm.callStatus().then(function(s) {
				dom.content(overview, self.renderOverview(s));
				dom.content(metrics, self.renderMetrics(s));
			});
		}, 5);

		var buttons = E('div', { 'class': 'cbi-page-actions', 'style': 'text-align:left' }, [
			E('button', {
				'type': 'button',
				'class': 'cbi-button cbi-button-action',
				'click': ui.createHandlerFn(this, this.action(function() { return mm.callTest(); }, _('Test connection')))
			}, [ mm.text(_('Test connection')) ]), ' ',
			E('button', {
				'type': 'button',
				'class': 'cbi-button cbi-button-apply',
				'click': ui.createHandlerFn(this, this.action(mm.callRepublish, _('Republish discovery')))
			}, [ mm.text(_('Republish discovery')) ])
		]);

		return E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, [ mm.text(_('MQTT Metrics')) ]),
			E('div', { 'class': 'cbi-map-descr' }, [ mm.text(_('Read-only router metrics for MQTT and Home Assistant. The service publishes; it never accepts commands from MQTT.')) ]),
			E('div', { 'class': 'cbi-section' }, [ E('h3', {}, [ mm.text(_('Connection')) ]), overview, buttons ]),
			E('div', { 'class': 'cbi-section' }, [ E('h3', {}, [ mm.text(_('Published metrics')) ]), metrics ])
		]);
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
