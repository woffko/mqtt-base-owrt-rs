// SPDX-License-Identifier: GPL-2.0-or-later
//! Small text helpers shared by the catalog, discovery and status code.

/// Object-id sanitizer, the same as `tr -c 'A-Za-z0-9_-' '_'`: the default
/// entity ids depend on it.
pub fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// UCI section names allow only `[A-Za-z0-9_]`.
pub fn sanitize_key(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Replace characters that are not allowed inside one MQTT topic level.
pub fn sanitize_topic_level(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '+' | '#' | '/' | '\0' => '_',
            other => other,
        })
        .collect()
}

/// A topic prefix or topic suffix is valid when it is non-empty, contains no
/// wildcards or NUL, and has no empty levels.
pub fn valid_topic(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['+', '#', '\0'])
        && value.split('/').all(|level| !level.is_empty())
        && value.len() <= 1024
}

/// Object ids are used inside discovery topics and unique ids.
pub fn valid_object_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Format a Unix timestamp as an RFC 3339 UTC string, e.g.
/// `2026-09-28T17:31:55+00:00`.
pub fn rfc3339_utc(unix_seconds: u64) -> String {
    let days = unix_seconds / 86_400;
    let rem = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (H. Hinnant's
/// algorithm, restricted to non-negative day counts).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizers_match_tr() {
        assert_eq!(sanitize_id("wan_6"), "wan_6");
        assert_eq!(sanitize_id("router-primary"), "router-primary");
        assert_eq!(sanitize_id("bond0.87"), "bond0_87");
        assert_eq!(sanitize_id("my phone"), "my_phone");
        assert_eq!(sanitize_key("router-primary"), "router_primary");
        assert_eq!(sanitize_topic_level("a/b+c#"), "a_b_c_");
    }

    #[test]
    fn topic_validation() {
        assert!(valid_topic("openwrt"));
        assert!(valid_topic("home/openwrt"));
        assert!(!valid_topic(""));
        assert!(!valid_topic("/openwrt"));
        assert!(!valid_topic("openwrt/"));
        assert!(!valid_topic("a//b"));
        assert!(!valid_topic("a/+/b"));
        assert!(!valid_topic("a/#"));
        assert!(valid_object_id("openwrt_router-1_wan"));
        assert!(!valid_object_id("bad id"));
    }

    #[test]
    fn rfc3339() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00+00:00");
        assert_eq!(rfc3339_utc(1_790_627_051), "2026-09-28T20:24:11+00:00");
    }
}
