// SPDX-License-Identifier: GPL-2.0-or-later
//! Active DHCP lease counts from `luci-rpc getDHCPLeases`.

use serde_json::Value as Json;

use super::{Context, GroupResult, Reading, Value};
use crate::catalog::*;
use crate::source::SourceError;

pub fn collect(ctx: &Context, out: &mut GroupResult) {
    let leases = match ctx.source.ubus("luci-rpc", "getDHCPLeases", None) {
        Ok(l) => l,
        Err(SourceError::Missing(_)) => {
            out.miss_group(Group::Dhcp, "luci-rpc (rpcd-mod-luci) is not installed");
            return;
        }
        Err(e) => {
            out.miss_group_err(Group::Dhcp, &e);
            return;
        }
    };
    for (def, key) in [
        (&DHCP_LEASES4, "dhcp_leases"),
        (&DHCP_LEASES6, "dhcp6_leases"),
    ] {
        // `null` means the server has no lease table for that family.
        let count = leases.get(key).and_then(Json::as_array).map_or(0, Vec::len);
        out.push(
            def,
            Instance::none(),
            Reading::Value(Value::Int(i64::try_from(count).unwrap_or(i64::MAX))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn counts_leases() {
        let r = run(
            Group::Dhcp,
            &fixture("primary"),
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert!(
            matches!(value_of(&r, "dhcp.leases_ipv4", "", ""), Some(Reading::Value(Value::Int(n))) if n > 0)
        );
        assert_eq!(
            value_of(&r, "dhcp.leases_ipv6", "", ""),
            Some(Reading::Value(Value::Int(0)))
        );
    }
}
