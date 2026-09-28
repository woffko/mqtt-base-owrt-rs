// SPDX-License-Identifier: GPL-2.0-or-later
//! Detection of the legacy `publish_metrics.sh` setup and import of its
//! broker settings. Nothing here edits `/etc/rc.local` or crontabs; the
//! migration of those files is a documented manual step.

use std::path::Path;
use std::time::Duration;

use crate::config::{CONFIG_NAME, SECRET_PATH};
use crate::exec;
use crate::state::write_atomic;

pub const SCRIPT: &str = "/usr/bin/publish_metrics.sh";
pub const MARKER: &str = "/etc/.publish_metrics_installed";
pub const CRONTAB: &str = "/etc/crontabs/root";
pub const RC_LOCAL: &str = "/etc/rc.local";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Detection {
    pub script: bool,
    pub marker: bool,
    pub cron: bool,
    pub rc_local: bool,
}

pub fn detect() -> Detection {
    let contains = |path: &str| {
        std::fs::read_to_string(path)
            .map(|t| {
                t.lines()
                    .any(|l| !l.trim_start().starts_with('#') && l.contains("publish_metrics.sh"))
            })
            .unwrap_or(false)
    };
    Detection {
        script: Path::new(SCRIPT).exists(),
        marker: Path::new(MARKER).exists(),
        cron: contains(CRONTAB),
        rc_local: contains(RC_LOCAL),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Broker {
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<set>"))
            .finish()
    }
}

/// Read `NAME="value"` assignments from the script without evaluating it.
fn assignment(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(name)?.strip_prefix('=')?;
        let value = if let Some(inner) = rest.strip_prefix('"') {
            inner.strip_suffix('"')?.to_string()
        } else if let Some(inner) = rest.strip_prefix('\'') {
            inner.strip_suffix('\'')?.to_string()
        } else {
            rest.split_whitespace().next().unwrap_or("").to_string()
        };
        (!value.is_empty() && !value.contains(['$', '`'])).then_some(value)
    })
}

pub fn parse_script(text: &str) -> Option<Broker> {
    Some(Broker {
        host: assignment(text, "MQTT_BROKER")?,
        port: assignment(text, "MQTT_PORT").and_then(|p| p.parse().ok()),
        username: assignment(text, "MQTT_USER"),
        password: assignment(text, "MQTT_PASSWORD"),
    })
}

fn uci(args: &[&str]) -> Result<(), String> {
    let out = exec::run("uci", args, None, Duration::from_secs(10), 64 * 1024)
        .map_err(|e| format!("uci: {e}"))?;
    if out.success() {
        Ok(())
    } else {
        Err(format!("uci {} failed", args.first().unwrap_or(&"")))
    }
}

/// Copy the legacy broker settings into UCI and the password file. The
/// password is written to the file directly and never appears in argv,
/// output or logs. Returns the imported (non-secret) settings.
pub fn import(script: &Path) -> Result<Broker, String> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let broker =
        parse_script(&text).ok_or_else(|| "the script has no MQTT_BROKER setting".to_string())?;
    uci(&["set", &format!("{CONFIG_NAME}.broker=broker")])?;
    uci(&["set", &format!("{CONFIG_NAME}.broker.host={}", broker.host)])?;
    if let Some(port) = broker.port {
        uci(&["set", &format!("{CONFIG_NAME}.broker.port={port}")])?;
    }
    match &broker.username {
        Some(user) => uci(&["set", &format!("{CONFIG_NAME}.broker.username={user}")])?,
        None => {
            uci(&["-q", "delete", &format!("{CONFIG_NAME}.broker.username")])
                .or(Ok::<(), String>(()))?
        }
    }
    write_atomic(
        Path::new(SECRET_PATH),
        broker.password.as_deref().unwrap_or("").as_bytes(),
        0o600,
    )
    .map_err(|e| format!("{SECRET_PATH}: {e}"))?;
    uci(&["commit", CONFIG_NAME])?;
    Ok(broker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_assignments() {
        let script = "#!/bin/sh\nMQTT_BROKER=\"192.0.2.10\"\nMQTT_PORT=\"1883\"\nMQTT_USER=\"openwrt\"\nMQTT_PASSWORD=\"s3cr'et\"\nTOPIC_PREFIX=\"openwrt\"\n";
        let b = parse_script(script).unwrap();
        assert_eq!(b.host, "192.0.2.10");
        assert_eq!(b.port, Some(1883));
        assert_eq!(b.username.as_deref(), Some("openwrt"));
        assert_eq!(b.password.as_deref(), Some("s3cr'et"));
        assert!(!format!("{b:?}").contains("s3cr"));
    }

    #[test]
    fn refuses_computed_values() {
        assert_eq!(parse_script("MQTT_BROKER=\"$(cat /x)\"\n"), None);
        assert_eq!(parse_script("echo nothing\n"), None);
    }
}
