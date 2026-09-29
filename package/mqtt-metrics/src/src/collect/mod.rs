// SPDX-License-Identifier: GPL-2.0-or-later
//! Collectors turn data sources into typed samples.
//!
//! A collector never formats payloads and never invents values: a missing
//! source becomes an unavailable reading (or a missing template) with a
//! reason that LuCI shows next to the metric.

mod dhcp;
mod hacluster;
mod iface;
mod mwan3;
pub mod net;
mod packages;
mod services;
mod system;
mod traffic;
mod wifi;
mod wireguard;

pub use services::available as services_available;

use std::collections::BTreeMap;

use crate::catalog::{Group, Instance, Kind, MetricDef, ALL};
use crate::config::Config;
use crate::source::{Source, SourceError, SourceResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    /// Value and number of decimals used in the payload.
    Float(f64, u8),
    Bool(bool),
}

impl Value {
    /// MQTT payload for this value.
    pub fn payload(&self, kind: Kind) -> String {
        match (self, kind) {
            (Value::Bool(b), Kind::Binary { on, off }) => (if *b { on } else { off }).to_string(),
            (Value::Bool(b), Kind::Sensor) => (if *b { "true" } else { "false" }).to_string(),
            (Value::Text(t), _) => t.clone(),
            (Value::Int(i), _) => i.to_string(),
            (Value::Float(v, d), _) => format!("{:.*}", usize::from(*d), v),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Value(Value),
    /// No value right now.
    ///
    /// - `permanent`: the entity can never have a value on this router (for
    ///   example IPv4 on a DHCPv6-only interface), so it is not announced to
    ///   Home Assistant.
    /// - `failed`: the source could not be read (timeout, garbled answer). The
    ///   last published value is kept instead of being cleared, because the
    ///   value did not go away.
    ///
    /// Neither: the value is absent (interface down, target not answering); a
    /// value published earlier is cleared once.
    Unavailable {
        reason: String,
        permanent: bool,
        failed: bool,
    },
}

impl Reading {
    pub fn text(value: impl Into<String>) -> Reading {
        Reading::Value(Value::Text(value.into()))
    }

    /// The value is absent right now.
    pub fn missing(reason: impl Into<String>) -> Reading {
        Reading::Unavailable {
            reason: reason.into(),
            permanent: false,
            failed: false,
        }
    }

    /// The value cannot exist on this router.
    pub fn impossible(reason: impl Into<String>) -> Reading {
        Reading::Unavailable {
            reason: reason.into(),
            permanent: true,
            failed: false,
        }
    }

    /// The source could not be read; the previous value is kept.
    pub fn failed(reason: impl Into<String>) -> Reading {
        Reading::Unavailable {
            reason: reason.into(),
            permanent: false,
            failed: true,
        }
    }

    /// A missing source is an absent value; any other error is a failed read.
    pub fn from_error(err: &SourceError) -> Reading {
        match err {
            SourceError::Missing(_) => Reading::missing(err.to_string()),
            SourceError::Failed(_) => Reading::failed(err.to_string()),
        }
    }

    /// Text reading that is unavailable when empty.
    pub fn non_empty(value: String, reason: &str) -> Reading {
        if value.is_empty() {
            Reading::missing(reason)
        } else {
            Reading::Value(Value::Text(value))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub def: &'static MetricDef,
    pub inst: Instance,
    pub reading: Reading,
}

impl Sample {
    pub fn new(def: &'static MetricDef, inst: Instance, reading: Reading) -> Sample {
        Sample { def, inst, reading }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupResult {
    pub samples: Vec<Sample>,
    /// Templates without any instance on this router, with the reason.
    pub missing: Vec<(&'static MetricDef, String)>,
    /// A source failed (as opposed to being absent): instances that are
    /// missing from this result have not necessarily gone away.
    pub failed: bool,
}

impl GroupResult {
    fn push(&mut self, def: &'static MetricDef, inst: Instance, reading: Reading) {
        self.samples.push(Sample::new(def, inst, reading));
    }

    fn miss(&mut self, def: &'static MetricDef, reason: impl Into<String>) {
        self.missing.push((def, reason.into()));
    }

    /// Mark every template of `group` as missing.
    fn miss_group(&mut self, group: Group, reason: &str) {
        for def in ALL.iter().copied().filter(|d| d.group == group) {
            self.miss(def, reason);
        }
    }

    /// Mark every template of `group` as missing because reading `err`
    /// failed or the source is absent.
    fn miss_group_err(&mut self, group: Group, err: &SourceError) {
        self.failed |= matches!(err, SourceError::Failed(_));
        self.miss_group(group, &err.to_string());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Counters {
    pub rx: u64,
    pub tx: u64,
    pub at: f64,
}

/// State kept between collection rounds (rates and cached constants).
#[derive(Debug, Clone, Default)]
pub struct CollectorState {
    pub cpu: BTreeMap<String, CpuTimes>,
    pub traffic: BTreeMap<String, Counters>,
    pub arch: Option<String>,
}

pub struct Context<'a> {
    pub source: &'a dyn Source,
    pub cfg: &'a Config,
    /// Unix time in seconds.
    pub now: u64,
    /// Monotonic time in seconds, for rates.
    pub mono: f64,
}

/// Data shared by several groups in one round.
#[derive(Default)]
pub struct RoundCache {
    net: Option<SourceResult<net::NetView>>,
}

impl RoundCache {
    pub fn net(&mut self, ctx: &Context) -> &SourceResult<net::NetView> {
        self.net
            .get_or_insert_with(|| net::view(ctx.source, ctx.cfg))
    }
}

/// Collect one group.
pub fn collect(
    group: Group,
    ctx: &Context,
    cache: &mut RoundCache,
    state: &mut CollectorState,
) -> GroupResult {
    let mut out = GroupResult::default();
    match group {
        Group::System => system::collect(ctx, state, &mut out),
        Group::Interface => iface::collect(ctx, cache, &mut out),
        Group::Traffic => traffic::collect(ctx, cache, state, &mut out),
        Group::Mwan3 => mwan3::collect(ctx, &mut out),
        Group::HaCluster => hacluster::collect(ctx, &mut out),
        Group::Dhcp => dhcp::collect(ctx, &mut out),
        Group::WireGuard => wireguard::collect(ctx, &mut out),
        Group::Wifi => wifi::collect(ctx, &mut out),
        Group::Service => services::collect(ctx, &mut out),
        Group::Packages => packages::collect(ctx, &mut out),
    }
    out
}

/// 2020-01-01T00:00:00Z. A router without a battery-backed clock starts near
/// 1970 until NTP sets the time; values derived from the wall clock are not
/// meaningful before that.
const CLOCK_SET_AFTER: u64 = 1_577_836_800;

pub fn clock_is_set(now: u64) -> bool {
    now >= CLOCK_SET_AFTER
}

/// Runtime hostname, as the legacy script resolves it.
pub fn hostname(source: &dyn Source) -> String {
    if let Ok(name) = source.command("uci", &["-q", "get", "system.@system[0].hostname"]) {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if let Ok(name) = source.read("/proc/sys/kernel/hostname") {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    "openwrt".to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source::FixtureSource;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub fn fixture(node: &str) -> FixtureSource {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures")
            .join(node);
        FixtureSource::new(&dir)
    }

    /// A private copy of a fixture directory that a test may modify between
    /// collection rounds. The copy is removed on drop.
    pub struct Scratch {
        pub dir: PathBuf,
        pub source: FixtureSource,
    }

    impl Scratch {
        pub fn new(node: &str, tag: &str) -> Scratch {
            let from = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tests/fixtures")
                .join(node);
            // Tests run in parallel: every copy gets its own directory.
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "mqtt-metrics-scratch-{}-{}-{tag}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            for entry in std::fs::read_dir(&from).unwrap() {
                let entry = entry.unwrap();
                if entry.path().is_file() {
                    std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
                }
            }
            let source = FixtureSource::new(&dir);
            Scratch { dir, source }
        }

        pub fn json(&self, name: &str) -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(self.dir.join(name)).unwrap()).unwrap()
        }

        pub fn write_json(&self, name: &str, doc: &serde_json::Value) {
            std::fs::write(self.dir.join(name), doc.to_string()).unwrap();
        }

        pub fn write(&self, name: &str, text: &str) {
            std::fs::write(self.dir.join(name), text).unwrap();
        }

        /// Modify one JSON fixture in place.
        pub fn edit_json(&self, name: &str, edit: impl FnOnce(&mut serde_json::Value)) {
            let mut doc = self.json(name);
            edit(&mut doc);
            self.write_json(name, &doc);
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    pub fn run(
        group: Group,
        source: &dyn Source,
        cfg: &Config,
        state: &mut CollectorState,
        mono: f64,
    ) -> GroupResult {
        let ctx = Context {
            source,
            cfg,
            now: source.now(),
            mono,
        };
        collect(group, &ctx, &mut RoundCache::default(), state)
    }

    pub fn value_of(result: &GroupResult, id: &str, name: &str, sub: &str) -> Option<Reading> {
        result
            .samples
            .iter()
            .find(|s| s.def.id == id && s.inst.name == name && s.inst.sub == sub)
            .map(|s| s.reading.clone())
    }

    #[test]
    fn payload_formats() {
        let binary = Kind::Binary {
            on: "up",
            off: "down",
        };
        assert_eq!(Value::Bool(true).payload(binary), "up");
        assert_eq!(Value::Bool(false).payload(binary), "down");
        assert_eq!(Value::Float(1.23456, 2).payload(Kind::Sensor), "1.23");
        assert_eq!(Value::Int(-5).payload(Kind::Sensor), "-5");
    }

    #[test]
    fn hostname_from_fixture() {
        assert_eq!(hostname(&fixture("primary")), "router-primary");
    }
}
