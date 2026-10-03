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
	load: function() {
		return Promise.all([
			L.resolveDefault(mm.callCatalog(), null),
			uci.load('mqtt-metrics')
		]);
	},

	render: function(data) {
		var catalog = data[0] || {};
		// The catalog says whether this build supports TLS; when the call
		// failed the page must not pretend to know.
		var unknown = !data[0];
		var identity = catalog.identity || {};
		var m, s, o;

		m = new form.Map('mqtt-metrics', _('MQTT Metrics: Broker'),
			_('Connection to the MQTT broker.') +
			(unknown ? ' ' + _('The service did not answer, so what this page shows about TLS support may be wrong.') : ''));

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
		o.datatype = 'and(uinteger,range(1,65535))';
		o.placeholder = '1883';
		o = s.option(form.Value, 'username', _('User name'));
		o = s.option(form.Value, 'password', _('Password'));
		o.password = true;
		o = s.option(form.Value, 'client_id', _('Client ID'), _('Must be unique on the broker. Default: mqtt-metrics-&lt;hostname&gt;.'));
		o.placeholder = identity.client_id || '';
		o.validate = function(section_id, value) {
			return value === '' || mm.validObjectId(value) ? true : _('Only letters, digits, "_" and "-" are allowed');
		};
		o = s.option(form.Value, 'keepalive', _('Keepalive (s)'));
		o.datatype = 'and(uinteger,range(5,3600))';
		o.placeholder = '60';
		o = s.option(form.ListValue, 'qos', _('QoS for states'), _('Discovery messages always use QoS 1.'));
		o.value('0', '0');
		o.value('1', '1');
		o.default = '1';
		o = s.option(form.Flag, 'retain', _('Retain states'), _('The broker keeps the last state, so Home Assistant shows it after a restart.'));
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.Flag, 'tls', _('Use TLS'),
			catalog.tls_supported === false ? _('This build has no TLS support; enabling it makes the configuration invalid.') : null);
		o.default = '0';
		o.validate = function(section_id, value) {
			return (value === '1' && catalog.tls_supported === false) ? _('TLS is not supported by this build') : true;
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
			_('Connects with the address, user, password and TLS switches shown above, including unsaved changes, then disconnects. Nothing is published. Certificate files are taken from the saved configuration.'));
		o.inputtitle = _('Test connection');
		o.inputstyle = 'action';
		o.onclick = function() {
			var args = [
				formValue(m, 'broker', 'host') || '',
				+(formValue(m, 'broker', 'port') || 1883),
				formValue(m, 'broker', 'username') || '',
				formValue(m, 'broker', 'password') || '',
				formValue(m, 'broker', 'tls') === '1',
				formValue(m, 'broker', 'tls_verify') !== '0'
			];
			return mm.callTest.apply(mm, args).then(function(res) {
				ui.addNotification(null, E('p', {}, [ mm.text(res.ok ? res.message : (_('Connection failed') + ': ' + res.error)) ]),
					res.ok ? 'info' : 'danger');
			}).catch(function(err) {
				ui.addNotification(null, E('p', {}, [ mm.text(_('Connection failed') + ': ' + mm.errorText(err)) ]), 'danger');
			});
		};

		// A browser that saved the router's own login could fill it into the
		// broker's user name and password, and Save would store the router
		// password as the broker password. The map renders again after a save,
		// so the attributes are set on every render.
		var renderContents = m.renderContents;
		m.renderContents = function() {
			return renderContents.apply(this, arguments).then(function(node) {
				// LuCI makes the field type="text" first and switches it to
				// "password" in the next frame, so it is found by its class.
				node.querySelectorAll('input.cbi-input-password').forEach(function(i) { i.setAttribute('autocomplete', 'new-password'); });
				var user = node.querySelector('[data-field="cbid.mqtt-metrics.broker.username"] input');
				if (user)
					user.setAttribute('autocomplete', 'off');
				return node;
			});
		};

		return m.render();
	}
});
