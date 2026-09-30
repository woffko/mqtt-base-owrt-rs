'use strict';
'require view';
'require ui';
'require dom';
'require mqtt-metrics.common as mm';

return view.extend({
	load: function() {
		return Promise.all([
			L.resolveDefault(mm.callCatalog(), {}),
			L.resolveDefault(mm.callLog(), {}),
			L.resolveDefault(mm.callStatus(), {})
		]);
	},

	renderPreview: function(catalog) {
		var metrics = (catalog.metrics || []).slice().sort(function(a, b) { return a.key < b.key ? -1 : a.key > b.key ? 1 : 0; });
		var select = E('select', { 'class': 'cbi-input-select', 'style': 'width:auto;max-width:100%' }, metrics.map(function(m) {
			return E('option', { 'value': m.key }, [ mm.text(m.key + (m.publish ? '' : ' (' + _('not published') + ')')) ]);
		}));
		var out = E('pre', { 'style': 'white-space:pre-wrap;word-break:break-all;max-height:30em;overflow:auto' });
		return E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, [ mm.text(_('Discovery preview')) ]),
			E('div', { 'class': 'cbi-section-descr' }, [ mm.text(_('The exact Home Assistant discovery message for a metric with the saved configuration.')) ]),
			E('div', { 'style': 'display:flex;gap:.5em;flex-wrap:wrap' }, [
				select,
				E('button', {
					'type': 'button',
					'class': 'cbi-button cbi-button-action',
					'click': ui.createHandlerFn(this, function() {
						return mm.callPreview(select.value).then(function(res) {
							dom.content(out, mm.text(res.error ? res.error :
								_('Topic') + ': ' + res.config_topic + '\n' + _('State topic') + ': ' + res.state_topic + '\n' +
								_('Announced') + ': ' + (res.announced ? _('yes') : _('no')) + '\n\n' + JSON.stringify(res.payload, null, 2)));
						}).catch(function(err) {
							dom.content(out, mm.text(mm.errorText(err)));
						});
					})
				}, [ mm.text(_('Show')) ])
			]),
			out
		]);
	},

	renderActions: function(status) {
		var run = function(call, title, confirmText) {
			return function(ev) {
				if (confirmText && !window.confirm(confirmText))
					return;
				return call().then(function(res) {
					ui.addNotification(null, E('p', {}, [ mm.text(title + ': ' + (res && (res.message || res.error) || '')) ]), res && res.ok ? 'info' : 'danger');
				}).catch(function(err) {
					ui.addNotification(null, E('p', {}, [ mm.text(title + ': ' + mm.errorText(err)) ]), 'danger');
				});
			};
		};
		return E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, [ mm.text(_('Actions')) ]),
			E('p', {}, [ mm.text(_('Retained topics owned by this router: %d').format(status.owned_topics || 0)) ]),
			E('div', { 'style': 'display:flex;gap:.5em;flex-wrap:wrap' }, [
				E('button', { 'type': 'button', 'class': 'cbi-button cbi-button-action',
					'click': ui.createHandlerFn(this, run(mm.callReload, _('Reload'))) }, [ mm.text(_('Reload configuration')) ]),
				E('button', { 'type': 'button', 'class': 'cbi-button cbi-button-apply',
					'click': ui.createHandlerFn(this, run(mm.callRepublish, _('Republish'))) }, [ mm.text(_('Republish discovery and states')) ]),
				E('button', { 'type': 'button', 'class': 'cbi-button cbi-button-negative',
					'click': ui.createHandlerFn(this, run(mm.callPurge, _('Reset retained topics'),
						_('Clear every retained topic this router owns on the broker and publish everything again? Home Assistant recreates the entities.'))) },
					[ mm.text(_('Reset retained topics')) ])
			])
		]);
	},

	renderLog: function(log) {
		var newestFirst = function(l) { return (l.lines || []).slice().reverse().join('\n'); };
		var box = E('pre', { 'style': 'white-space:pre-wrap;word-break:break-all;max-height:30em;overflow:auto' },
			[ mm.text(log.error ? log.error : (newestFirst(log) || _('No log lines.'))) ]);
		return E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, [ mm.text(_('Service log')) ]),
			E('div', { 'class': 'cbi-section-descr' }, [ mm.text(_('Newest lines first.')) ]),
			E('button', {
				'type': 'button',
				'class': 'cbi-button cbi-button-neutral',
				'click': ui.createHandlerFn(this, function() {
					return mm.callLog().then(function(l) {
						dom.content(box, mm.text(l.error ? l.error : (newestFirst(l) || _('No log lines.'))));
					}).catch(function(err) {
						dom.content(box, mm.text(mm.errorText(err)));
					});
				})
			}, [ mm.text(_('Refresh')) ]),
			box
		]);
	},

	render: function(data) {
		return E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, [ mm.text(_('MQTT Metrics: Diagnostics')) ]),
			this.renderActions(data[2] || {}),
			this.renderPreview(data[0] || {}),
			this.renderLog(data[1] || {})
		]);
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
