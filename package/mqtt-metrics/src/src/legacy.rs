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

/// Whether any non-comment line of a crontab or `rc.local` runs or installs
/// the legacy script.
pub fn mentions_script(text: &str) -> bool {
    text.lines()
        .any(|l| !l.trim_start().starts_with('#') && l.contains("publish_metrics.sh"))
}

/// Whether the crontab still schedules the legacy script.
pub fn cron_active(crontab: &str) -> bool {
    mentions_script(crontab)
}

pub fn detect() -> Detection {
    let contains = |path: &str| {
        std::fs::read_to_string(path)
            .map(|t| mentions_script(&t))
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

/// What the script says about one setting.
#[derive(Debug, PartialEq, Eq)]
enum Assignment {
    Absent,
    Value(String),
    /// Only the shell can tell the effective value (variables, command
    /// substitution, concatenation), so it is not imported.
    Computed,
}

/// Text after a closing quote: nothing, or a comment.
fn ends_line(rest: &str) -> bool {
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with('#')
}

/// Parse the value of an assignment the way `sh` reads it, for the literal
/// forms this file can contain.
fn parse_value(rest: &str) -> Assignment {
    if let Some(inner) = rest.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = inner.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    return if ends_line(&inner[i + 1..]) {
                        Assignment::Value(value)
                    } else {
                        Assignment::Computed
                    };
                }
                '$' | '`' => return Assignment::Computed,
                '\\' => match chars.next() {
                    // Inside double quotes a backslash only escapes these.
                    Some((_, e @ ('$' | '`' | '"' | '\\'))) => value.push(e),
                    Some((_, other)) => {
                        value.push('\\');
                        value.push(other);
                    }
                    None => return Assignment::Computed,
                },
                other => value.push(other),
            }
        }
        return Assignment::Computed;
    }
    if let Some(inner) = rest.strip_prefix('\'') {
        return match inner.split_once('\'') {
            Some((value, after)) if ends_line(after) => Assignment::Value(value.to_string()),
            _ => Assignment::Computed,
        };
    }
    let word = rest
        .split(|c: char| c.is_whitespace() || c == ';' || c == '#')
        .next()
        .unwrap_or("");
    if word.contains(['$', '`', '"', '\'', '\\', '(', ')', '&', '|', '<', '>']) {
        Assignment::Computed
    } else {
        Assignment::Value(word.to_string())
    }
}

fn assignment(text: &str, name: &str) -> Assignment {
    let found = text.lines().find_map(|line| {
        let line = line.trim();
        // `export NAME=value` and `readonly NAME=value` assign as well.
        let line = ["export ", "readonly "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .map_or(line, str::trim_start);
        let rest = line.strip_prefix(name)?.strip_prefix('=')?;
        Some(parse_value(rest))
    });
    match found {
        Some(assignment) => assignment,
        // Assigned somewhere the parser does not read (inside a function or
        // a condition): not a script without that setting, and importing "no
        // password" would delete the stored one.
        None if text.contains(&format!("{name}=")) => Assignment::Computed,
        None => Assignment::Absent,
    }
}

/// Read the broker settings from the script without running it. A setting
/// that only the shell could evaluate is an error: importing a guess would
/// store a wrong password.
pub fn parse_script(text: &str) -> Result<Broker, String> {
    let computed = |name: &str| {
        format!("{name} in the script uses shell expansion and cannot be imported; set it in LuCI")
    };
    let host = match assignment(text, "MQTT_BROKER") {
        Assignment::Value(v) if !v.is_empty() => v,
        Assignment::Computed => return Err(computed("MQTT_BROKER")),
        _ => return Err("the script has no MQTT_BROKER setting".to_string()),
    };
    let port = match assignment(text, "MQTT_PORT") {
        Assignment::Absent => None,
        Assignment::Value(v) if v.is_empty() => None,
        Assignment::Value(v) => Some(
            v.parse::<u16>()
                .map_err(|_| format!("MQTT_PORT '{v}' is not a port number"))?,
        ),
        Assignment::Computed => return Err(computed("MQTT_PORT")),
    };
    let optional = |name: &str| match assignment(text, name) {
        Assignment::Absent => Ok(None),
        Assignment::Value(v) => Ok(Some(v).filter(|v| !v.is_empty())),
        Assignment::Computed => Err(computed(name)),
    };
    Ok(Broker {
        host,
        port,
        username: optional("MQTT_USER")?,
        password: optional("MQTT_PASSWORD")?,
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
    let broker = parse_script(&text)?;
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
    match broker.password.as_deref() {
        Some(password) => write_atomic(Path::new(SECRET_PATH), password.as_bytes(), 0o600),
        // The script connects without a password: so must we.
        None => match std::fs::remove_file(SECRET_PATH) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
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
    fn export_and_readonly_assignments_are_read() {
        let script = "export MQTT_BROKER=\"192.0.2.10\"\nreadonly MQTT_USER='u'\nexport   MQTT_PASSWORD=\"pw\" # c\n";
        let b = parse_script(script).unwrap();
        assert_eq!(b.host, "192.0.2.10");
        assert_eq!(b.username.as_deref(), Some("u"));
        assert_eq!(b.password.as_deref(), Some("pw"));
    }

    #[test]
    fn an_assignment_the_parser_does_not_read_is_not_an_absent_password() {
        // Importing "no password" would delete the stored secret.
        let script = "MQTT_BROKER=\"192.0.2.10\"\n[ -f /etc/x ] && MQTT_PASSWORD=\"pw\"\n";
        assert!(parse_script(script).is_err());
        // A script that never assigns one connects without a password.
        let anonymous = parse_script("MQTT_BROKER=\"192.0.2.10\"\n").unwrap();
        assert_eq!(anonymous.password, None);
    }

    #[test]
    fn refuses_computed_values() {
        assert!(parse_script("MQTT_BROKER=\"$(cat /x)\"\n").is_err());
        assert!(parse_script("echo nothing\n").is_err());
        assert!(parse_script("MQTT_BROKER=\"$HOST\"\n").is_err());
        assert!(parse_script("MQTT_BROKER=h\nMQTT_PORT=$P\n").is_err());
    }

    #[test]
    fn a_password_line_that_cannot_be_read_is_an_error_not_an_empty_password() {
        // `$$` is a shell expansion (the process id): the effective password
        // is not what the file says, so importing it would store a wrong one.
        for password in ["pa$$word", "a`id`b", "\"a\"$b", "abc\"def"] {
            let script = format!("MQTT_BROKER=\"192.0.2.10\"\nMQTT_PASSWORD=\"{password}\"\n");
            let err = parse_script(&script).unwrap_err();
            assert!(err.contains("MQTT_PASSWORD"), "{password}: {err}");
        }
    }

    #[test]
    fn a_script_without_password_or_user_is_an_anonymous_broker() {
        let b = parse_script("MQTT_BROKER=broker.example\n").unwrap();
        assert_eq!((b.port, b.username, b.password), (None, None, None));
        let b = parse_script("MQTT_BROKER=broker.example\nMQTT_PASSWORD=\"\"\n").unwrap();
        assert_eq!(b.password, None);
    }

    #[test]
    fn trailing_comments_and_shell_escapes_are_understood() {
        let script = "MQTT_BROKER=\"192.0.2.10\" # the broker\nMQTT_USER='openwrt'  # user\nMQTT_PASSWORD=\"a\\\"b\\$c\\\\d\\e\" # pw\n";
        let b = parse_script(script).unwrap();
        assert_eq!(b.host, "192.0.2.10");
        assert_eq!(b.username.as_deref(), Some("openwrt"));
        // `\"` `\$` `\\` are escapes; `\e` keeps its backslash.
        assert_eq!(b.password.as_deref(), Some("a\"b$c\\d\\e"));
    }

    #[test]
    fn concatenation_is_not_a_literal() {
        assert!(parse_script("MQTT_BROKER=\"a\"$B\n").is_err());
        assert!(parse_script("MQTT_BROKER='a''b'\n").is_err());
    }

    #[test]
    fn crontab_and_rc_local_detection_skips_comments() {
        assert!(cron_active("*/5 * * * * /usr/bin/publish_metrics.sh\n"));
        assert!(!cron_active(
            "# */5 * * * * /usr/bin/publish_metrics.sh\n0 3 * * * true\n"
        ));
        assert!(mentions_script(
            "cat > /usr/bin/publish_metrics.sh <<'EOF'\n"
        ));
    }
}
