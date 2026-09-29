'use strict';
'require view';
'require form';
'require uci';
'require mqtt-metrics.common as mm';

function topicValidator(allowEmpty) {
	return function(section_id, value) {
		if (value === '' && allowEmpty)
			return true;
		return mm.validTopic(value) ? true : _('A topic must not be empty, contain "+" or "#", or have empty levels');
	};
}

return view.extend({
	load: function() {
		return Promise.all([
			L.resolveDefault(mm.callCatalog(), {}),
			uci.load('mqtt-metrics')
		]);
	},

	render: function(data) {
		var catalog = data[0] || {}, id = catalog.identity || {};
		var hostname = id.hostname || 'hostname';
		var before = {};
		mm.IDENTITY_KEYS.forEach(function(k) { before[k] = uci.get('mqtt-metrics', 'identity', k) || ''; });

		var m, s, o;
		m = new form.Map('mqtt-metrics', _('MQTT Metrics: Topics and device'),
			_('Defaults reproduce the legacy publish_metrics.sh script, so existing Home Assistant entities keep their history.'));

		var warning = E('div', { 'class': 'alert-message warning', 'style': 'display:none' }, [
			mm.text(_('The base topic, node, discovery prefix or device ID changed. A new device ID creates new Home Assistant entities; their history does not carry over. A new base topic or node moves the state topics: anything else that reads the old topics must be updated. Home Assistant has to listen on the discovery prefix. The old retained topics are removed automatically.'))
		]);
		var updateWarning = function() {
			var after = {};
			mm.IDENTITY_KEYS.forEach(function(k) {
				var opt = m.lookupOption(k, 'identity');
				after[k] = opt ? (opt[0].formvalue('identity') || '') : before[k];
			});
			warning.style.display = mm.identityChanged(before, after).length ? '' : 'none';
		};

		s = m.section(form.NamedSection, 'identity', 'identity', _('Topics'));
		o = s.option(form.Value, 'base_topic', _('Base topic'), _('State topics are &lt;base topic&gt;/&lt;node&gt;/...'));
		o.placeholder = 'openwrt';
		o.validate = topicValidator(true);
		o.onchange = updateWarning;
		o = s.option(form.Value, 'node', _('Node'), _('Topic level for this router. Default: the hostname.'));
		o.placeholder = hostname;
		o.validate = function(section_id, value) {
			return (value === '' || !/[\/+#]/.test(value)) ? true : _('The node is a single topic level');
		};
		o.onchange = updateWarning;
		o = s.option(form.DummyValue, '_state_example', _('Example state topic'));
		o.cfgvalue = function() { return (id.base_topic || ('openwrt/' + hostname)) + '/system/uptime'; };

		s = m.section(form.NamedSection, 'identity', 'identity', _('Home Assistant'));
		o = s.option(form.Flag, 'discovery', _('MQTT discovery'),
			_('When off, states are still published but no entity is announced (shadow mode). Turning it off removes the entities this router announced.'));
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.Value, 'discovery_prefix', _('Discovery prefix'));
		o.placeholder = 'homeassistant';
		o.validate = topicValidator(true);
		o.onchange = updateWarning;
		o = s.option(form.Flag, 'birth', _('Resend discovery when Home Assistant restarts'),
			_('Subscribes to the birth topic below, the only subscription the service makes. A message there only makes the service resend its discovery messages; it never triggers anything else. Turn it off if the broker does not allow the subscription.'));
		o.default = '1';
		o.rmempty = false;
		o = s.option(form.Value, 'birth_topic', _('Birth topic'),
			_('Home Assistant announces "online" here after a restart.'));
		o.depends('birth', '1');
		o.placeholder = 'homeassistant/status';
		o.validate = topicValidator(true);
		o = s.option(form.Value, 'device_id', _('Device ID'), _('Prefix of every unique_id. Default: openwrt_&lt;hostname&gt;.'));
		o.placeholder = id.device_id || ('openwrt_' + hostname);
		o.validate = function(section_id, value) {
			return value === '' || mm.validObjectId(value) ? true : _('Only letters, digits, "_" and "-" are allowed');
		};
		o.onchange = updateWarning;
		o = s.option(form.Value, 'device_name', _('Device name'));
		o.placeholder = 'OpenWrt ' + hostname;
		o = s.option(form.Value, 'manufacturer', _('Manufacturer'));
		o.placeholder = 'OpenWrt';
		o = s.option(form.Value, 'model', _('Model'));
		o.placeholder = 'Router';
		o = s.option(form.Value, 'configuration_url', _('Configuration URL'), _('Link shown on the Home Assistant device page, for example this LuCI page.'));
		o.datatype = 'string';
		o.placeholder = window.location.origin + L.url('admin/services/mqtt-metrics');
		o = s.option(form.Flag, 'cleanup_legacy', _('Remove legacy leftovers'),
			_('On the first start with discovery on, adopt the legacy script\'s topics and remove the entities that can never have a value (for example IPv4 on a DHCPv6 interface).'));
		o.default = '1';
		o.rmempty = false;

		// The map renders its contents again after every save, which removes
		// anything added to it by hand. The banner (and the baseline it compares
		// with) is put back each time.
		var renderContents = m.renderContents;
		m.renderContents = function() {
			return renderContents.apply(this, arguments).then(function(node) {
				mm.IDENTITY_KEYS.forEach(function(k) { before[k] = uci.get('mqtt-metrics', 'identity', k) || ''; });
				node.insertBefore(warning, node.querySelector('.cbi-section'));
				updateWarning();
				return node;
			});
		};
		return m.render();
	}
});
