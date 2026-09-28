// SPDX-License-Identifier: GPL-2.0-or-later
//! Publish engine: resolves samples into entities, schedules collection per
//! group, decides what to publish, and tracks the retained topics it owns.
//!
//! The engine performs no network I/O. It returns [`Publication`]s that the
//! daemon sends, which keeps the logic testable against fixtures.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{self, expand, Family, Group, Instance, MetricDef};
use crate::collect::{self, net, CollectorState, Context, Reading, RoundCache, Sample};
use crate::config::Config;
use crate::ha::{self, EntityView, Identity};
use crate::source::Source;
use crate::state::TopicState;
use crate::text::{sanitize_id, sanitize_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
    pub qos: u8,
}

/// A sample resolved against configuration and identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Stable key (UCI section name for overrides): the default object id
    /// restricted to `[A-Za-z0-9_]`.
    pub key: String,
    pub def: &'static MetricDef,
    pub inst: Instance,
    pub object_id: String,
    pub unique_id: String,
    pub state_topic: String,
    pub config_topic: String,
    pub name: String,
    pub icon: Option<String>,
    pub publish: bool,
    /// Per-metric "Show in Home Assistant" flag.
    pub discovery: bool,
    pub ha_enabled: bool,
    pub diagnostic: bool,
    pub interval: u32,
    pub reading: Reading,
}

impl Entry {
    pub fn payload(&self) -> Option<String> {
        match &self.reading {
            Reading::Value(v) => Some(v.payload(self.def.kind)),
            Reading::Unavailable { .. } => None,
        }
    }

    fn permanent(&self) -> bool {
        matches!(
            self.reading,
            Reading::Unavailable {
                permanent: true,
                ..
            }
        )
    }

    /// Whether the entity is announced to Home Assistant.
    pub fn announce(&self, cfg: &Config) -> bool {
        cfg.identity.discovery && self.publish && self.discovery && !self.permanent()
    }

    pub fn discovery_json(&self, id: &Identity) -> String {
        ha::discovery_payload(
            &EntityView {
                def: self.def,
                unique_id: &self.unique_id,
                name: &self.name,
                state_topic: &self.state_topic,
                icon: self.icon.as_deref(),
                diagnostic: self.diagnostic,
                ha_enabled: self.ha_enabled,
            },
            id,
        )
    }
}

pub fn default_object(def: &MetricDef, inst: &Instance) -> String {
    sanitize_id(&expand(def.object, inst))
}

pub fn metric_key(def: &MetricDef, inst: &Instance) -> String {
    sanitize_key(&expand(def.object, inst))
}

/// Interval of a metric: per-metric override, group override, the metric's
/// own default for slow sources, then the global interval.
pub fn interval_for(cfg: &Config, def: &MetricDef, over_interval: Option<u32>) -> u32 {
    over_interval
        .or(cfg.group(def.group).interval)
        .or(def.default_interval)
        .unwrap_or(cfg.publish.interval)
}

pub fn resolve(sample: Sample, cfg: &Config, id: &Identity) -> Entry {
    let def = sample.def;
    let key = metric_key(def, &sample.inst);
    let over = cfg.metrics.get(&key).cloned().unwrap_or_default();
    let object_id = over
        .object_id
        .clone()
        .unwrap_or_else(|| default_object(def, &sample.inst));
    let unique_id = format!("{}_{object_id}", id.device_id);
    let suffix = over
        .topic
        .clone()
        .unwrap_or_else(|| expand(def.topic, &sample.inst));
    Entry {
        config_topic: id.config_topic(def.kind, &unique_id),
        state_topic: format!("{}/{suffix}", id.base),
        name: over
            .name
            .clone()
            .unwrap_or_else(|| expand(def.name, &sample.inst)),
        icon: over.icon.clone().or_else(|| def.icon.map(str::to_string)),
        publish: over.publish.unwrap_or(def.legacy),
        discovery: over.discovery.unwrap_or(true),
        ha_enabled: over.ha_enabled.unwrap_or(true),
        diagnostic: over.diagnostic.unwrap_or(def.diagnostic),
        interval: interval_for(cfg, def, over.interval),
        key,
        def,
        inst: sample.inst,
        object_id,
        unique_id,
        reading: sample.reading,
    }
}

/// Result of collecting a set of groups.
#[derive(Debug, Default)]
pub struct Round {
    pub entries: Vec<Entry>,
    pub missing: Vec<(Group, &'static MetricDef, String)>,
    /// UCI interfaces matching the legacy WAN rule (when the network view
    /// was available).
    pub legacy_wan: Option<Vec<String>>,
}

#[allow(clippy::too_many_arguments)]
pub fn collect_round(
    groups: &[Group],
    cfg: &Config,
    id: &Identity,
    source: &dyn Source,
    now: u64,
    mono: f64,
    state: &mut CollectorState,
    want_legacy_wan: bool,
) -> Round {
    let ctx = Context {
        source,
        cfg,
        now,
        mono,
    };
    let mut cache = RoundCache::default();
    let mut round = Round::default();
    for group in groups {
        let result = collect::collect(*group, &ctx, &mut cache, state);
        round
            .entries
            .extend(result.samples.into_iter().map(|s| resolve(s, cfg, id)));
        round
            .missing
            .extend(result.missing.into_iter().map(|(d, r)| (*group, d, r)));
    }
    if want_legacy_wan {
        if let Ok(view) = cache.net(&ctx) {
            let mut names: Vec<String> = view
                .uci
                .iter()
                .map(|i| i.name.clone())
                .filter(|n| net::legacy_wan(n))
                .collect();
            names.sort();
            round.legacy_wan = Some(names);
        }
    }
    round
}

/// Retained topics that the legacy script published for `hostname`.
pub fn legacy_topics(hostname: &str, wan: &[String]) -> BTreeSet<String> {
    let id = Identity::resolve(&Config::default(), hostname);
    let mut out = BTreeSet::new();
    for def in catalog::ALL.iter().copied().filter(|d| d.legacy) {
        let instances: Vec<Instance> = match def.group {
            Group::Interface => wan
                .iter()
                .map(|n| Instance::iface(Family::Wan, n))
                .collect(),
            _ => vec![Instance::none()],
        };
        for inst in instances {
            let unique_id = format!("{}_{}", id.device_id, default_object(def, &inst));
            out.insert(id.config_topic(def.kind, &unique_id));
            out.insert(format!("{}/{}", id.base, expand(def.topic, &inst)));
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
struct Track {
    last_payload: Option<String>,
    last_publish: Option<f64>,
    published_at: Option<u64>,
    announced: Option<String>,
}

/// Snapshot of one entity for status output.
#[derive(Debug, Clone)]
pub struct EntryStatus<'a> {
    pub entry: &'a Entry,
    pub last_payload: Option<&'a str>,
    pub published_at: Option<u64>,
}

pub struct Engine {
    cfg: Config,
    id: Identity,
    collectors: CollectorState,
    tracks: BTreeMap<String, Track>,
    entries: BTreeMap<String, Entry>,
    missing: Vec<(Group, &'static MetricDef, String)>,
    next_due: BTreeMap<Group, f64>,
    topics: TopicState,
    topics_dirty: bool,
}

/// Slack so that a round collected slightly early still publishes.
const DUE_SLACK: f64 = 1.0;

impl Engine {
    pub fn new(cfg: Config, hostname: &str, topics: TopicState) -> Engine {
        Engine {
            id: Identity::resolve(&cfg, hostname),
            cfg,
            collectors: CollectorState::default(),
            tracks: BTreeMap::new(),
            entries: BTreeMap::new(),
            missing: Vec::new(),
            next_due: BTreeMap::new(),
            topics,
            topics_dirty: false,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn identity(&self) -> &Identity {
        &self.id
    }

    /// Apply a new configuration. A full pass must follow.
    pub fn reconfigure(&mut self, cfg: Config, hostname: &str) {
        self.id = Identity::resolve(&cfg, hostname);
        self.cfg = cfg;
        self.next_due.clear();
    }

    pub fn topics(&self) -> &TopicState {
        &self.topics
    }

    /// The topic set when it changed since the last call.
    pub fn take_dirty_topics(&mut self) -> Option<TopicState> {
        if self.topics_dirty {
            self.topics_dirty = false;
            Some(self.topics.clone())
        } else {
            None
        }
    }

    pub fn statuses(&self) -> Vec<EntryStatus<'_>> {
        self.entries
            .values()
            .map(|e| {
                let t = self.tracks.get(&e.key);
                EntryStatus {
                    entry: e,
                    last_payload: t.and_then(|t| t.last_payload.as_deref()),
                    published_at: t.and_then(|t| t.published_at),
                }
            })
            .collect()
    }

    pub fn missing(&self) -> &[(Group, &'static MetricDef, String)] {
        &self.missing
    }

    fn enabled_groups(&self) -> Vec<Group> {
        Group::ALL
            .iter()
            .copied()
            .filter(|g| self.cfg.group(*g).enabled)
            .collect()
    }

    fn clear(&self, topic: &str) -> Publication {
        Publication {
            topic: topic.to_string(),
            payload: String::new(),
            retain: true,
            qos: 1,
        }
    }

    fn state_publication(&self, e: &Entry, payload: String) -> Publication {
        Publication {
            topic: e.state_topic.clone(),
            payload,
            retain: self.cfg.broker.retain,
            qos: self.cfg.broker.qos,
        }
    }

    fn group_interval(&self, group: Group) -> Option<u32> {
        self.entries
            .values()
            .filter(|e| e.def.group == group && e.publish)
            .map(|e| e.interval)
            .min()
    }

    fn reschedule(&mut self, group: Group, mono: f64) {
        match self.group_interval(group) {
            Some(i) => {
                self.next_due.insert(group, mono + f64::from(i));
            }
            None => {
                self.next_due.remove(&group);
            }
        }
    }

    /// Monotonic time of the next scheduled round.
    pub fn next_due(&self) -> Option<f64> {
        self.next_due.values().copied().reduce(f64::min)
    }

    /// Collect every enabled group, clear stale retained topics, announce
    /// discovery and publish every state. Used after connecting, after a
    /// configuration change and when Home Assistant comes online.
    pub fn full_pass(&mut self, source: &dyn Source, now: u64, mono: f64) -> Vec<Publication> {
        let groups = self.enabled_groups();
        let seed = self.cfg.identity.discovery
            && self.cfg.identity.cleanup_legacy
            && !self.topics.legacy_seeded;
        let round = collect_round(
            &groups,
            &self.cfg,
            &self.id,
            source,
            now,
            mono,
            &mut self.collectors,
            seed,
        );
        self.missing = round.missing;
        self.entries = round
            .entries
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();

        if seed {
            if let Some(wan) = &round.legacy_wan {
                self.topics
                    .topics
                    .extend(legacy_topics(&self.id.hostname, wan));
                self.topics.legacy_seeded = true;
                self.topics_dirty = true;
            }
        }

        let mut want = BTreeSet::new();
        for e in self.entries.values() {
            if e.announce(&self.cfg) {
                want.insert(e.config_topic.clone());
            }
            // A permanently unavailable value never gets a state, so a
            // retained state left by the legacy script is cleared.
            if e.publish && !e.permanent() {
                want.insert(e.state_topic.clone());
            }
        }

        let mut out: Vec<Publication> = self
            .topics
            .topics
            .difference(&want)
            .map(|t| self.clear(t))
            .collect();

        let mut announced = Vec::new();
        let mut states = Vec::new();
        for e in self.entries.values() {
            if e.announce(&self.cfg) {
                let payload = e.discovery_json(&self.id);
                out.push(Publication {
                    topic: e.config_topic.clone(),
                    payload: payload.clone(),
                    retain: true,
                    qos: 1,
                });
                announced.push((e.key.clone(), Some(payload)));
            } else {
                announced.push((e.key.clone(), None));
            }
            if e.publish {
                if let Some(p) = e.payload() {
                    states.push((e.key.clone(), self.state_publication(e, p.clone()), p));
                }
            }
        }
        for (key, payload) in announced {
            self.tracks.entry(key).or_default().announced = payload;
        }
        for (key, publication, payload) in states {
            let t = self.tracks.entry(key).or_default();
            t.last_payload = Some(payload);
            t.last_publish = Some(mono);
            t.published_at = Some(now);
            out.push(publication);
        }

        if self.topics.topics != want {
            self.topics.topics = want;
            self.topics_dirty = true;
        }
        self.next_due.clear();
        for group in groups {
            self.reschedule(group, mono);
        }
        out
    }

    /// Collect the groups that are due and publish what changed or is due.
    pub fn tick(&mut self, source: &dyn Source, now: u64, mono: f64) -> Vec<Publication> {
        let due: Vec<Group> = self
            .next_due
            .iter()
            .filter(|(_, t)| **t <= mono)
            .map(|(g, _)| *g)
            .collect();
        if due.is_empty() {
            return Vec::new();
        }
        let round = collect_round(
            &due,
            &self.cfg,
            &self.id,
            source,
            now,
            mono,
            &mut self.collectors,
            false,
        );
        let fresh_keys: BTreeSet<String> = round.entries.iter().map(|e| e.key.clone()).collect();
        self.entries
            .retain(|k, e| !due.contains(&e.def.group) || fresh_keys.contains(k));
        self.missing.retain(|(g, _, _)| !due.contains(g));
        self.missing.extend(round.missing);

        let mut out = Vec::new();
        for e in round.entries {
            let track = self.tracks.entry(e.key.clone()).or_default();
            if e.announce(&self.cfg) {
                let payload = e.discovery_json(&self.id);
                if track.announced.as_deref() != Some(payload.as_str()) {
                    out.push(Publication {
                        topic: e.config_topic.clone(),
                        payload: payload.clone(),
                        retain: true,
                        qos: 1,
                    });
                    track.announced = Some(payload);
                    if self.topics.topics.insert(e.config_topic.clone()) {
                        self.topics_dirty = true;
                    }
                }
            } else if track.announced.take().is_some() {
                out.push(Publication {
                    topic: e.config_topic.clone(),
                    payload: String::new(),
                    retain: true,
                    qos: 1,
                });
                if self.topics.topics.remove(&e.config_topic) {
                    self.topics_dirty = true;
                }
            }

            if e.publish {
                if let Some(payload) = e.payload() {
                    let elapsed = track.last_publish.map(|t| mono - t + DUE_SLACK);
                    let due = if self.cfg.publish.change_only {
                        track.last_payload.as_deref() != Some(payload.as_str())
                            || elapsed.is_none_or(|el| el >= f64::from(self.cfg.publish.max_age))
                    } else {
                        elapsed.is_none_or(|el| el >= f64::from(e.interval))
                    };
                    if due {
                        track.last_payload = Some(payload.clone());
                        track.last_publish = Some(mono);
                        track.published_at = Some(now);
                        out.push(Publication {
                            topic: e.state_topic.clone(),
                            payload,
                            retain: self.cfg.broker.retain,
                            qos: self.cfg.broker.qos,
                        });
                        if self.topics.topics.insert(e.state_topic.clone()) {
                            self.topics_dirty = true;
                        }
                    }
                }
            }
            self.entries.insert(e.key.clone(), e);
        }
        for group in due {
            self.reschedule(group, mono);
        }
        out
    }

    /// Clear every retained topic this router owns, including availability.
    pub fn purge(&mut self) -> Vec<Publication> {
        let mut out: Vec<Publication> = self.topics.topics.iter().map(|t| self.clear(t)).collect();
        out.push(self.clear(&self.id.availability_topic));
        self.topics.topics.clear();
        self.topics_dirty = true;
        for t in self.tracks.values_mut() {
            t.announced = None;
            t.last_payload = None;
            t.last_publish = None;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::fixture;
    use crate::config::MetricOverride;

    fn topics_of(p: &[Publication]) -> BTreeSet<String> {
        p.iter()
            .filter(|p| !p.payload.is_empty())
            .map(|p| p.topic.clone())
            .collect()
    }

    #[test]
    fn first_full_pass_publishes_legacy_set_and_clears_impossible_entities() {
        let src = fixture("primary");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        let out = engine.full_pass(&src, src.now(), 0.0);
        let published = topics_of(&out);
        let cleared: BTreeSet<String> = out
            .iter()
            .filter(|p| p.payload.is_empty())
            .map(|p| p.topic.clone())
            .collect();
        let legacy = legacy_topics(
            "router-primary",
            &["wan".into(), "wan_6".into(), "wanb".into(), "wanb_6".into()],
        );
        // Everything published belongs to the legacy topic set.
        assert!(
            published.is_subset(&legacy),
            "{:#?}",
            published.difference(&legacy).collect::<Vec<_>>()
        );
        // Impossible entities (IPv6 on pppoe/static, IPv4 on dhcpv6) are cleared.
        for gone in [
            "homeassistant/sensor/openwrt_router-primary_wan_wan_ipv6/config",
            "homeassistant/sensor/openwrt_router-primary_wan_wan_gateway6/config",
            "homeassistant/sensor/openwrt_router-primary_wan_wan_6_ipv4/config",
            "homeassistant/sensor/openwrt_router-primary_wan_wanb_gateway6/config",
        ] {
            assert!(cleared.contains(gone), "{gone}");
        }
        assert!(published
            .contains("homeassistant/binary_sensor/openwrt_router-primary_wan_wan_status/config"));
        assert!(published.contains("openwrt/router-primary/system/uptime"));
        assert!(engine.topics().legacy_seeded);
        assert!(engine.take_dirty_topics().is_some());
        // 6 system + 4 status + 4 device + 2 ipv4 + 2 gateway4 + 2 ipv6 + 2 gateway6 configs.
        let configs = out
            .iter()
            .filter(|p| p.topic.ends_with("/config") && !p.payload.is_empty())
            .count();
        assert_eq!(configs, 22);
    }

    #[test]
    fn shadow_mode_never_touches_discovery() {
        let src = fixture("primary");
        let mut cfg = Config::default();
        cfg.identity.base_topic = "openwrt-shadow".into();
        cfg.identity.discovery = false;
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let out = engine.full_pass(&src, src.now(), 0.0);
        assert!(
            out.iter()
                .all(|p| p.topic.starts_with("openwrt-shadow/router-primary/")),
            "{out:#?}"
        );
        assert!(!engine.topics().legacy_seeded);
    }

    #[test]
    fn hiding_a_metric_clears_it_on_the_next_pass() {
        let src = fixture("primary");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        engine.full_pass(&src, src.now(), 0.0);
        let mut cfg = Config::default();
        cfg.metrics.insert(
            "system_model".into(),
            MetricOverride {
                publish: Some(false),
                ..MetricOverride::default()
            },
        );
        cfg.metrics.insert(
            "system_load_1".into(),
            MetricOverride {
                publish: Some(true),
                ..MetricOverride::default()
            },
        );
        engine.reconfigure(cfg, "router-primary");
        let out = engine.full_pass(&src, src.now(), 10.0);
        let cleared: Vec<&str> = out
            .iter()
            .filter(|p| p.payload.is_empty())
            .map(|p| p.topic.as_str())
            .collect();
        assert!(
            cleared.contains(&"homeassistant/sensor/openwrt_router-primary_system_model/config")
        );
        assert!(cleared.contains(&"openwrt/router-primary/system/model"));
        assert!(topics_of(&out)
            .contains("homeassistant/sensor/openwrt_router-primary_system_load_1/config"));
    }

    #[test]
    fn rename_clears_old_ids() {
        let src = fixture("primary");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        engine.full_pass(&src, src.now(), 0.0);
        let mut cfg = Config::default();
        cfg.identity.device_id = Some("gw1".into());
        engine.reconfigure(cfg, "router-primary");
        let out = engine.full_pass(&src, src.now(), 10.0);
        let cleared = out
            .iter()
            .filter(|p| p.payload.is_empty() && p.topic.contains("openwrt_router-primary_"))
            .count();
        assert_eq!(cleared, 22);
        assert!(topics_of(&out).contains("homeassistant/sensor/gw1_system_uptime/config"));
    }

    #[test]
    fn tick_respects_interval_and_change_only() {
        let src = fixture("primary");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        engine.full_pass(&src, src.now(), 0.0);
        assert_eq!(engine.next_due(), Some(300.0));
        assert!(engine.tick(&src, src.now(), 100.0).is_empty());
        let out = engine.tick(&src, src.now(), 300.0);
        // All legacy states are republished; discovery is not repeated.
        assert!(out.iter().all(|p| !p.topic.ends_with("/config")));
        assert_eq!(out.len(), 6 + 4 + 4 + 2 + 2 + 2 + 2);

        let mut cfg = Config::default();
        cfg.publish.change_only = true;
        engine.reconfigure(cfg, "router-primary");
        engine.full_pass(&src, src.now(), 400.0);
        // Identical fixture values: nothing changes, nothing is published.
        assert!(engine.tick(&src, src.now(), 700.0).is_empty());
        // After max_age everything is refreshed.
        assert!(!engine.tick(&src, src.now(), 400.0 + 3600.0).is_empty());
    }

    #[test]
    fn purge_clears_everything() {
        let src = fixture("primary");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        engine.full_pass(&src, src.now(), 0.0);
        let owned = engine.topics().topics.len();
        let out = engine.purge();
        assert_eq!(out.len(), owned + 1);
        assert!(out.iter().all(|p| p.payload.is_empty() && p.retain));
        assert!(engine.topics().topics.is_empty());
    }
}
