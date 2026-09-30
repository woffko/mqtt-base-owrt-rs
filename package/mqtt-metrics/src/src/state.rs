// SPDX-License-Identifier: GPL-2.0-or-later
//! Persisted set of retained topics this router owns on the broker.
//!
//! Every retained topic the daemon publishes (discovery configs and states)
//! is recorded here. A full pass clears recorded topics that are no longer
//! wanted, which removes hidden metrics, deselected interfaces and old ids
//! after a rename from Home Assistant. The file changes only when the set
//! changes, so normal operation does not write to flash.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use serde_json::{json, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopicState {
    pub topics: BTreeSet<String>,
}

impl TopicState {
    pub fn to_json(&self) -> String {
        json!({
            "version": 1,
            "topics": self.topics.iter().collect::<Vec<_>>(),
        })
        .to_string()
    }

    pub fn from_json(text: &str) -> Option<TopicState> {
        let v: Value = serde_json::from_str(text).ok()?;
        if v.get("version").and_then(Value::as_u64) != Some(1) {
            return None;
        }
        // Fields other than `topics` (an older version wrote `legacy_seeded`)
        // are ignored.
        Some(TopicState {
            topics: v
                .get("topics")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Load the state; a missing or unreadable file yields an empty state.
    pub fn load(path: &Path) -> TopicState {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| TopicState::from_json(&t))
            .unwrap_or_default()
    }

    /// Write atomically (temporary file and rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, self.to_json().as_bytes(), 0o600)
    }
}

pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = TopicState::default();
        s.topics.insert("a/b".into());
        s.topics.insert("c".into());
        assert_eq!(TopicState::from_json(&s.to_json()), Some(s));
        assert_eq!(TopicState::from_json("{\"version\":2}"), None);
        assert_eq!(TopicState::from_json("garbage"), None);
        // A file written by an older version.
        let old =
            TopicState::from_json(r#"{"version":1,"legacy_seeded":true,"topics":["x"]}"#).unwrap();
        assert_eq!(old.topics.len(), 1);
    }

    #[test]
    fn atomic_write() {
        let dir = std::env::temp_dir().join(format!("mqtt-metrics-state-{}", std::process::id()));
        let path = dir.join("state.json");
        let mut s = TopicState::default();
        s.topics.insert("x".into());
        s.save(&path).unwrap();
        assert_eq!(TopicState::load(&path), s);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
