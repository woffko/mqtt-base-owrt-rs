'use strict';
'require view';
'require form';
'require ui';
'require uci';
'require mqtt-metrics.common as mm';

function formValue(map, section, option) {
	var o = map.lookupOption(option, section);
	return o ? o[0].formvalue(section) : null;
}

return view.extend({
	pendingPassword: null,

	load: function() {
		return Promise.all([
			L.resolveDefault(mm.callCatalog(), {}),
			L.resolveDefault(mm.callLegacy(), {}),
			uci.load('mqtt-metrics')
		]);
	},

	render: function(data) {
		var self = this, catalog = data[0] || {}, legacy = data[1] || {};
		var identity = catalog.identity || {};
		var m, s, o;

		m = new form.Map('mqtt-metrics', _('MQTT Metrics: Broker'),
			_('Connection to the MQTT broker. The password is stored outside UCI in a root-only file and is never sent back to the browser.'));

		s = m.section(form.NamedSection, 'main', 'main', _('Service'));
		o = s.option(form.Flag, 'enabled', _('Enabled'));
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.ListValue, 'log_level', _('Log level'));
		o.value('error', _('Errors'));
		o.value('warn', _('Warnings'));
		o.value('info', _('Information'));
		o.value('debug', _('Debug'));
		o.default = 'info';

		s = m.section(form.NamedSection, 'broker', 'broker', _('Broker'));
		o = s.option(form.Value, 'host', _('Host'), _('Name or address of the broker. The service stays idle while this is empty.'));
		o.datatype = 'host';
		o = s.option(form.Value, 'port', _('Port'));
		o.datatype = 'port';
		o.placeholder = '1883';
		o = s.option(form.Value, 'username', _('User name'));
		o = s.option(form.Value, '_password', _('Password'),
			catalog.password_set ? _('A password is stored. Leave empty to keep it.') : _('No password is stored.'));
		o.password = true;
		o.placeholder = catalog.password_set ? _('(unchanged)') : '';
		o.cfgvalue = function() { return null; };
		o.write = function(section_id, value) { self.pendingPassword = value; };
		o.remove = function() {};
		if (catalog.password_set) {
			o = s.option(form.Flag, '_clear_password', _('Remove stored password'));
			o.cfgvalue = function() { return '0'; };
			o.write = function(section_id, value) {
				if (value === '1')
					self.pendingPassword = '';
			};
			o.remove = function() {};
		}
		o = s.option(form.Value, 'client_id', _('Client ID'), _('Must be unique on the broker. Default: mqtt-metrics-&lt;hostname&gt;.'));
		o.placeholder = identity.client_id || '';
		o.validate = function(section_id, value) {
			return value === '' || mm.validObjectId(value) ? true : _('Only letters, digits, "_" and "-" are allowed');
		};
		o = s.option(form.Value, 'keepalive', _('Keepalive (s)'));
		o.datatype = 'range(5,3600)';
		o.placeholder = '60';
		o = s.option(form.ListValue, 'qos', _('QoS for states'), _('Discovery messages always use QoS 1.'));
		o.value('0', '0');
		o.value('1', '1');
		o.default = '1';
		o = s.option(form.Flag, 'retain', _('Retain states'), _('The legacy script retained every state.'));
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.Flag, 'tls', _('Use TLS'),
			catalog.tls_supported ? null : _('This build has no TLS support; enabling it makes the configuration invalid.'));
		o.default = '0';
		o.validate = function(section_id, value) {
			return (value === '1' && !catalog.tls_supported) ? _('TLS is not supported by this build') : true;
		};
		o = s.option(form.Value, 'tls_ca', _('CA certificate file'),
			_('PEM file with the certificate authority of the broker. Empty: the system bundle /etc/ssl/certs/ca-certificates.crt.'));
		o.depends('tls', '1');
		o.placeholder = '/etc/ssl/certs/ca-certificates.crt';
		o = s.option(form.Flag, 'tls_verify', _('Verify the broker certificate'),
			_('Turn off only for a self-signed broker certificate without a CA file: the connection stays encrypted but the broker is not authenticated.'));
		o.depends('tls', '1');
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.Value, 'tls_cert', _('Client certificate file'), _('Optional PEM file for client certificate authentication.'));
		o.depends('tls', '1');
		o = s.option(form.Value, 'tls_key', _('Client key file'), _('PEM private key for the client certificate. Keep it readable by root only.'));
		o.depends('tls', '1');
		o.validate = function(section_id, value) {
			var cert = formValue(m, 'broker', 'tls_cert') || '';
			return (!!cert === !!value) ? true : _('Client certificate and key must be set together');
		};

		o = s.option(form.Button, '_test', _('Connection test'),
			_('Connects with the values shown above, including unsaved changes, then disconnects. Nothing is published.'));
		o.inputtitle = _('Test connection');
		o.inputstyle = 'action';
		o.onclick = function() {
			var typed = formValue(m, 'broker', '_password') || '';
			var args = [
				formValue(m, 'broker', 'host') || '',
				+(formValue(m, 'broker', 'port') || 1883),
				formValue(m, 'broker', 'username') || '',
				typed,
				typed === '',
				formValue(m, 'broker', 'client_id') || '',
				formValue(m, 'broker', 'tls') === '1'
			];
			return mm.callTest.apply(mm, args).then(function(res) {
				ui.addNotification(null, E('p', {}, [ mm.text(res.ok ? res.message : (_('Connection failed') + ': ' + res.error)) ]),
					res.ok ? 'info' : 'danger');
			});
		};

		if (legacy.importable) {
			s = m.section(form.NamedSection, 'broker', 'broker', _('Legacy script'),
				_('The legacy publish_metrics.sh script is installed on this router. Its broker address, user and password can be copied here; the password is copied on the router and not shown.'));
			o = s.option(form.DummyValue, '_legacy_host', _('Legacy broker'));
			o.cfgvalue = function() { return (legacy.broker_host || '-') + (legacy.broker_username ? ' (' + legacy.broker_username + ')' : ''); };
			o = s.option(form.Button, '_import', _('Import'));
			o.inputtitle = _('Import broker settings');
			o.inputstyle = 'apply';
			o.onclick = function() {
				return mm.callImportLegacy().then(function(res) {
					if (!res.ok) {
						ui.addNotification(null, E('p', {}, [ mm.text(_('Import failed') + ': ' + res.error) ]), 'danger');
						return;
					}
					ui.addNotification(null, E('p', {}, [ mm.text(_('Broker settings imported. Reloading the page.')) ]), 'info');
					window.setTimeout(function() { window.location.reload(); }, 1500);
				});
			};
		}

		this.map = m;
		return m.render();
	},

	handleSave: function(ev) {
		var self = this;
		self.pendingPassword = null;
		return this.map.save().then(function() {
			if (self.pendingPassword == null)
				return;
			var password = self.pendingPassword;
			self.pendingPassword = null;
			return mm.callSetPassword(password).then(function(res) {
				if (!res.ok)
					ui.addNotification(null, E('p', {}, [ mm.text(_('Password not saved') + ': ' + res.error) ]), 'danger');
				else
					ui.addNotification(null, E('p', {}, [ mm.text(res.password_set ? _('Password saved.') : _('Password removed.')) ]), 'info');
			});
		});
	}
});
