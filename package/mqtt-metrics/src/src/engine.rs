// SPDX-License-Identifier: GPL-2.0-or-later
//! Publish engine: resolves samples into entities, schedules collection per
//! group, decides what to publish, and tracks the retained topics it owns.
//!
//! The engine performs no network I/O. It returns [`Publication`]s that the
//! daemon sends, which keeps the logic testable against fixtures.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{self, expand, expand_topic, Family, Group, Instance, MetricDef};
use crate::collect::{self, net, CollectorState, Context, Reading, RoundCache, Sample};
use crate::config::Config;
use crate::ha::{self, EntityView, Identity};
use crate::legacy;
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

/// Whether `key` can be the key of a metric made from an object template: the
/// literal parts of the template appear in it, in order.
fn key_fits(template: &str, key: &str) -> bool {
    let mut literals = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        literals.push(sanitize_key(&rest[..start]));
        rest = &rest[start + len + 1..];
    }
    literals.push(sanitize_key(rest));
    let Some((first, others)) = literals.split_first() else {
        return false;
    };
    let Some(mut left) = key.strip_prefix(first.as_str()) else {
        return false;
    };
    let Some((last, middle)) = others.split_last() else {
        return left.is_empty();
    };
    for literal in middle {
        match left.find(literal.as_str()) {
            Some(at) => left = &left[at + literal.len()..],
            None => return false,
        }
    }
    left.ends_with(last.as_str())
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
        .unwrap_or_else(|| expand_topic(def.topic, &sample.inst));
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
    /// Groups whose source could not be read. Instances missing from such a
    /// group have not necessarily gone away.
    pub failed: Vec<Group>,
}

/// Distinct instances can collapse to the same key once their names are
/// sanitized (two WireGuard peers with the same description, `eth0.1` and
/// `eth0_1`). Number the later ones so that every instance keeps its own
/// entity instead of silently overwriting the earlier one.
fn unique_instance(seen: &mut BTreeSet<String>, def: &MetricDef, inst: Instance) -> Instance {
    let mut candidate = inst.clone();
    let mut n = 1;
    while !seen.insert(metric_key(def, &candidate)) {
        n += 1;
        candidate = inst.clone();
        let suffix = format!("_{n}");
        if candidate.sub.is_empty() {
            candidate.name.push_str(&suffix);
        } else {
            candidate.sub.push_str(&suffix);
        }
    }
    candidate
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
    let mut seen = BTreeSet::new();
    for group in groups {
        let result = collect::collect(*group, &ctx, &mut cache, state);
        if result.failed {
            round.failed.push(*group);
        }
        for sample in result.samples {
            let inst = unique_instance(&mut seen, sample.def, sample.inst);
            round.entries.push(resolve(
                Sample::new(sample.def, inst, sample.reading),
                cfg,
                id,
            ));
        }
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
            out.insert(format!("{}/{}", id.base, expand_topic(def.topic, &inst)));
        }
    }
    out
}

/// What the broker holds for an entry's state topic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Held {
    /// Not known yet (fresh start): the persisted set of owned topics decides.
    #[default]
    Unknown,
    /// No retained value.
    Nothing,
    /// We left a retained value on this topic.
    On(String),
}

#[derive(Debug, Clone, Default)]
struct Track {
    last_payload: Option<String>,
    last_publish: Option<f64>,
    published_at: Option<u64>,
    announced: Option<String>,
    held: Held,
}

impl Track {
    /// The retained value is gone (cleared or never there).
    fn forget_state(&mut self) {
        self.held = Held::Nothing;
        self.last_payload = None;
        self.last_publish = None;
    }
}

/// Whether the broker may still hold a retained value on `topic`.
fn holds_state(track: Option<&Track>, owned: &BTreeSet<String>, topic: &str) -> bool {
    match track.map(|t| &t.held) {
        Some(Held::Nothing) => false,
        Some(Held::On(held)) => held == topic,
        Some(Held::Unknown) | None => owned.contains(topic),
    }
}

fn clear_message(topic: &str) -> Publication {
    Publication {
        topic: topic.to_string(),
        payload: String::new(),
        retain: true,
        qos: 1,
    }
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
        clear_message(topic)
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

    /// Interval for a group that publishes nothing right now. It is still
    /// worth reading when a template that is on by default has no entry (its
    /// source is absent or failed) or when an override switches on a metric
    /// of an instance that is not there, so that the metric comes back when
    /// its source does.
    fn idle_interval(&self, group: Group) -> Option<u32> {
        catalog::ALL
            .iter()
            .copied()
            .filter(|def| def.group == group)
            .flat_map(|def| {
                let by_default = (def.legacy && !self.entries.values().any(|e| e.def.id == def.id))
                    .then(|| interval_for(&self.cfg, def, None));
                let overridden = self
                    .cfg
                    .metrics
                    .iter()
                    .filter(move |(key, over)| {
                        over.publish == Some(true) && key_fits(def.object, key)
                    })
                    .map(move |(_, over)| interval_for(&self.cfg, def, over.interval));
                by_default.into_iter().chain(overridden)
            })
            .min()
    }

    fn reschedule(&mut self, group: Group, mono: f64) {
        match self
            .group_interval(group)
            .or_else(|| self.idle_interval(group))
        {
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
        // Adopting the legacy script's topics clears whatever this
        // configuration no longer wants. While the script is still scheduled
        // it would publish them again, so wait until it is gone.
        let legacy_running = legacy::cron_active(&source.read(legacy::CRONTAB).unwrap_or_default());
        let seed = self.cfg.identity.discovery
            && self.cfg.identity.cleanup_legacy
            && !self.topics.legacy_seeded
            && !legacy_running;
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
        let previous = std::mem::take(&mut self.entries);
        self.entries = round
            .entries
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();
        // A group whose source failed says nothing about its instances. They
        // stay, as failed reads, so that their topics are not cleared as stale
        // and nothing old is published as new.
        for (key, mut entry) in previous {
            if !self.entries.contains_key(&key) && round.failed.contains(&entry.def.group) {
                if !entry.permanent() {
                    entry.reading = Reading::failed("the source could not be read");
                }
                self.entries.insert(key, entry);
            }
        }
        // What is gone loses its track. Its topics are cleared below when
        // nothing wants them, and if it comes back it is announced again.
        self.tracks.retain(|key, _| self.entries.contains_key(key));

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
        let mut clears = Vec::new();
        let mut forgets = Vec::new();
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
            if !e.publish {
                continue;
            }
            match &e.reading {
                Reading::Value(_) => {
                    if let Some(p) = e.payload() {
                        states.push((e.key.clone(), self.state_publication(e, p.clone()), p));
                    }
                }
                // The read failed: whatever the broker holds is still the
                // best knowledge.
                Reading::Unavailable { failed: true, .. } => {}
                // The entity cannot exist; a topic left behind is cleared
                // with the other topics that nothing wants any more.
                Reading::Unavailable {
                    permanent: true, ..
                } => forgets.push(e.key.clone()),
                // The value is absent. A value left by an earlier run or
                // pass is stale and would keep showing as current.
                Reading::Unavailable { .. } => {
                    if holds_state(self.tracks.get(&e.key), &self.topics.topics, &e.state_topic) {
                        clears.push((e.key.clone(), e.state_topic.clone()));
                    } else {
                        forgets.push(e.key.clone());
                    }
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
            t.held = Held::On(publication.topic.clone());
            out.push(publication);
        }
        for (key, topic) in clears {
            self.tracks.entry(key).or_default().forget_state();
            out.push(self.clear(&topic));
        }
        for key in forgets {
            self.tracks.entry(key).or_default().forget_state();
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
        let mut out = Vec::new();

        // Instances that disappeared from a group that was read successfully
        // are gone, and so is their value. A group whose source failed says
        // nothing about its instances, so they keep their last value.
        let vanished: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|(k, e)| {
                e.publish
                    && due.contains(&e.def.group)
                    && !round.failed.contains(&e.def.group)
                    && !fresh_keys.contains(*k)
            })
            .map(|(k, e)| (k.clone(), e.state_topic.clone()))
            .collect();
        for (key, topic) in vanished {
            if holds_state(self.tracks.get(&key), &self.topics.topics, &topic) {
                out.push(clear_message(&topic));
            }
            self.tracks.remove(&key);
        }
        self.entries.retain(|k, e| {
            !due.contains(&e.def.group)
                || fresh_keys.contains(k)
                || round.failed.contains(&e.def.group)
        });
        self.missing.retain(|(g, _, _)| !due.contains(g));
        self.missing.extend(round.missing);

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
                out.push(clear_message(&e.config_topic));
                if self.topics.topics.remove(&e.config_topic) {
                    self.topics_dirty = true;
                }
            }

            if e.publish {
                match &e.reading {
                    Reading::Value(_) => {
                        if let Some(payload) = e.payload() {
                            let elapsed = track.last_publish.map(|t| mono - t + DUE_SLACK);
                            let due = if self.cfg.publish.change_only {
                                track.last_payload.as_deref() != Some(payload.as_str())
                                    || elapsed
                                        .is_none_or(|el| el >= f64::from(self.cfg.publish.max_age))
                            } else {
                                elapsed.is_none_or(|el| el >= f64::from(e.interval))
                            };
                            if due {
                                track.last_payload = Some(payload.clone());
                                track.last_publish = Some(mono);
                                track.published_at = Some(now);
                                track.held = Held::On(e.state_topic.clone());
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
                    Reading::Unavailable { failed: true, .. } => {}
                    Reading::Unavailable { .. } => {
                        if holds_state(Some(&*track), &self.topics.topics, &e.state_topic) {
                            out.push(clear_message(&e.state_topic));
                        }
                        track.forget_state();
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
            t.forget_state();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::tests::fixture;
    use crate::config::MetricOverride;

    #[test]
    fn a_key_fits_only_the_template_it_came_from() {
        assert!(key_fits("mwan3_{n}_score", "mwan3_wan_score"));
        assert!(!key_fits("mwan3_{n}_score", "mwan3_wan_latency"));
        assert!(!key_fits("mwan3_{n}_score", "wg_wan_score"));
        assert!(key_fits("system_uptime", "system_uptime"));
        assert!(!key_fits("system_uptime", "system_uptime_2"));
        assert!(key_fits("{p}_{n}_status", "wan_wan_6_status"));
    }

    #[test]
    fn every_metric_key_fits_its_template() {
        let instances = [
            Instance::none(),
            Instance::named("eth0.1"),
            Instance::iface(Family::Wan, "wan_6"),
            Instance::iface(Family::Iface, "lan").with_sub("br-lan"),
            Instance::named("phone").with_sub("x y"),
        ];
        for def in catalog::ALL.iter().copied() {
            for inst in &instances {
                let key = metric_key(def, inst);
                assert!(
                    key_fits(def.object, &key),
                    "{} does not fit {key}",
                    def.object
                );
            }
        }
    }

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
        // Every topic is written once per pass.
        let mut seen = BTreeSet::new();
        for p in &out {
            assert!(seen.insert(&p.topic), "{} is published twice", p.topic);
        }
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

    // ---- audit regression tests ------------------------------------------

    use crate::collect::tests::Scratch;
    use crate::source::{Source, SourceResult};
    use serde_json::{json, Value};

    fn publishing(keys: &[&str]) -> Config {
        let mut cfg = Config::default();
        for key in keys {
            cfg.metrics.insert(
                (*key).to_string(),
                MetricOverride {
                    publish: Some(true),
                    ..MetricOverride::default()
                },
            );
        }
        cfg
    }

    fn payload_of<'a>(out: &'a [Publication], topic: &str) -> Option<&'a str> {
        out.iter()
            .find(|p| p.topic == topic)
            .map(|p| p.payload.as_str())
    }

    /// Take an interface down in the netifd dump: no device, addresses or routes.
    fn take_down(s: &Scratch, name: &str) {
        s.edit_json("network_interface_dump.json", |doc| {
            for iface in doc["interface"].as_array_mut().unwrap() {
                if iface["interface"] == name {
                    iface["up"] = json!(false);
                    let obj = iface.as_object_mut().unwrap();
                    obj.remove("l3_device");
                    obj.insert("ipv4-address".into(), json!([]));
                    obj.insert("ipv6-address".into(), json!([]));
                    obj.insert("route".into(), json!([]));
                }
            }
        });
    }

    #[test]
    fn a_value_that_disappears_is_cleared_once_and_comes_back() {
        let s = Scratch::new("primary", "vanish1");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        let ipv4 = "openwrt/router-primary/wan-wanb/ipv4";
        assert!(payload_of(&first, ipv4).is_some_and(|p| !p.is_empty()));

        let original = s.json("network_interface_dump.json");
        take_down(&s, "wanb");
        let down = engine.tick(&s.source, s.source.now(), 300.0);
        assert_eq!(
            payload_of(&down, "openwrt/router-primary/wan-wanb/status"),
            Some("down")
        );
        for field in ["ipv4", "device", "gateway4"] {
            let topic = format!("openwrt/router-primary/wan-wanb/{field}");
            let msg = down
                .iter()
                .find(|p| p.topic == topic)
                .unwrap_or_else(|| panic!("{topic}: stale retained value is not cleared"));
            assert!(msg.payload.is_empty() && msg.retain, "{topic}");
        }
        // Cleared once, not again on the next round.
        let again = engine.tick(&s.source, s.source.now(), 600.0);
        assert!(!again.iter().any(|p| p.topic.contains("wan-wanb/ipv4")));

        s.write_json("network_interface_dump.json", &original);
        let up = engine.tick(&s.source, s.source.now(), 900.0);
        assert!(payload_of(&up, ipv4).is_some_and(|p| !p.is_empty()));
    }

    #[test]
    fn restart_while_a_value_is_missing_clears_the_old_retained_state() {
        let s = Scratch::new("primary", "vanish2");
        let mut first = Engine::new(Config::default(), "router-primary", TopicState::default());
        first.full_pass(&s.source, s.source.now(), 0.0);
        let persisted = first.topics().clone();
        take_down(&s, "wanb");
        let mut second = Engine::new(Config::default(), "router-primary", persisted);
        let out = second.full_pass(&s.source, s.source.now(), 0.0);
        assert_eq!(
            payload_of(&out, "openwrt/router-primary/wan-wanb/ipv4"),
            Some("")
        );
    }

    #[test]
    fn a_failing_source_does_not_blank_published_values() {
        let s = Scratch::new("primary", "failing");
        let cfg = publishing(&["system_memory_used", "system_kernel"]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        let memory = "openwrt/router-primary/system/memory_used";
        let kernel = "openwrt/router-primary/system/kernel";
        assert!(payload_of(&first, memory).is_some_and(|p| !p.is_empty()));

        // ubus answers with garbage: the read failed, the value did not vanish.
        s.write("system_info.json", "not json");
        s.write("system_board.json", "not json");
        let out = engine.tick(&s.source, s.source.now(), 300.0);
        assert_eq!(payload_of(&out, memory), None);
        assert_eq!(payload_of(&out, kernel), None);
    }

    #[test]
    fn a_group_that_fails_at_the_first_pass_is_read_again() {
        // netifd is not ready yet when the daemon starts.
        let s = Scratch::new("primary", "late-netifd");
        let dump = std::fs::read_to_string(s.dir.join("network_interface_dump.json")).unwrap();
        s.write("network_interface_dump.json", "not json");
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        let status = "openwrt/router-primary/wan-wan/status";
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        assert_eq!(payload_of(&first, status), None);

        s.write("network_interface_dump.json", &dump);
        let out = engine.tick(&s.source, s.source.now(), 300.0);
        assert_eq!(payload_of(&out, status), Some("up"));
    }

    #[test]
    fn a_group_that_is_absent_at_the_first_pass_is_read_again() {
        // mwan3 is installed and started after the daemon.
        let s = Scratch::new("primary", "late-mwan3");
        let status = std::fs::read_to_string(s.dir.join("mwan3_status.json")).unwrap();
        std::fs::remove_file(s.dir.join("mwan3_status.json")).unwrap();
        let cfg = publishing(&["mwan3_wan_score"]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let score = "openwrt/router-primary/mwan3-wan/score";
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        assert_eq!(payload_of(&first, score), None);

        s.write("mwan3_status.json", &status);
        let out = engine.tick(&s.source, s.source.now(), 300.0);
        assert!(payload_of(&out, score).is_some_and(|p| !p.is_empty()));
    }

    #[test]
    fn a_group_comes_back_after_a_failed_round() {
        let s = Scratch::new("primary", "failed-round");
        let dump = std::fs::read_to_string(s.dir.join("network_interface_dump.json")).unwrap();
        let mut engine = Engine::new(Config::default(), "router-primary", TopicState::default());
        let status = "openwrt/router-primary/wan-wan/status";
        engine.full_pass(&s.source, s.source.now(), 0.0);

        s.write("network_interface_dump.json", "not json");
        let out = engine.tick(&s.source, s.source.now(), 300.0);
        assert_eq!(payload_of(&out, status), None);

        s.write("network_interface_dump.json", &dump);
        let out = engine.tick(&s.source, s.source.now(), 600.0);
        assert_eq!(payload_of(&out, status), Some("up"));
    }

    fn config_topic_of(out: &[Publication], object: &str) -> String {
        out.iter()
            .map(|p| p.topic.clone())
            .find(|t| t.ends_with("/config") && t.contains(object))
            .unwrap_or_else(|| panic!("no discovery config for {object}"))
    }

    #[test]
    fn an_entity_removed_by_a_full_pass_is_announced_again_when_it_returns() {
        let s = Scratch::new("primary", "returning");
        let status = std::fs::read_to_string(s.dir.join("mwan3_status.json")).unwrap();
        let cfg = publishing(&["mwan3_wan_score"]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        let config = config_topic_of(&first, "mwan3_wan_score");

        // mwan3 stops tracking `wan` and the connection is re-established:
        // the full pass removes the entity from Home Assistant.
        s.edit_json("mwan3_status.json", |doc| {
            doc["interfaces"].as_object_mut().unwrap().remove("wan");
        });
        let out = engine.full_pass(&s.source, s.source.now(), 100.0);
        assert_eq!(payload_of(&out, &config), Some(""));

        s.write("mwan3_status.json", &status);
        let out = engine.tick(&s.source, s.source.now(), 400.0);
        assert!(
            payload_of(&out, &config).is_some_and(|p| !p.is_empty()),
            "the entity is announced again"
        );
    }

    #[test]
    fn a_full_pass_keeps_the_entities_of_a_group_whose_source_failed() {
        let s = Scratch::new("primary", "failed-full-pass");
        let status = std::fs::read_to_string(s.dir.join("mwan3_status.json")).unwrap();
        let cfg = publishing(&["mwan3_wan_score"]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        let config = config_topic_of(&first, "mwan3_wan_score");
        let score = "openwrt/router-primary/mwan3-wan/score";

        // The connection is re-established while ubus answers with garbage.
        s.write("mwan3_status.json", "not json");
        let out = engine.full_pass(&s.source, s.source.now(), 100.0);
        assert!(
            out.iter()
                .all(|p| !p.topic.contains("mwan3") || !p.payload.is_empty()),
            "nothing about mwan3 is cleared"
        );
        assert!(payload_of(&out, &config).is_some_and(|p| !p.is_empty()));
        assert_eq!(
            payload_of(&out, score),
            None,
            "no stale value is republished"
        );

        s.write("mwan3_status.json", &status);
        let out = engine.tick(&s.source, s.source.now(), 400.0);
        assert!(payload_of(&out, score).is_some_and(|p| !p.is_empty()));
    }

    #[test]
    fn a_vanished_instance_is_cleared_but_a_failed_group_is_not() {
        let s = Scratch::new("primary", "vanished-group");
        let cfg = publishing(&["mwan3_wan_score", "mwan3_wanb_score"]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let first = engine.full_pass(&s.source, s.source.now(), 0.0);
        let wan = "openwrt/router-primary/mwan3-wan/score";
        let wanb = "openwrt/router-primary/mwan3-wanb/score";
        assert!(payload_of(&first, wan).is_some_and(|p| !p.is_empty()));

        // mwan3 stops tracking `wan`: its entry disappears from the round.
        s.edit_json("mwan3_status.json", |doc| {
            doc["interfaces"].as_object_mut().unwrap().remove("wan");
        });
        let out = engine.tick(&s.source, s.source.now(), 300.0);
        assert_eq!(payload_of(&out, wan), Some(""));
        assert_eq!(
            payload_of(&out, wanb).map(str::is_empty),
            Some(false),
            "the other instance is unaffected"
        );

        // A broken answer for the whole group must not blank anything.
        s.write("mwan3_status.json", "not json");
        let out = engine.tick(&s.source, s.source.now(), 600.0);
        assert_eq!(payload_of(&out, wanb), None);
    }

    #[test]
    fn duplicate_names_get_distinct_entities() {
        let s = Scratch::new("primary", "duplicates");
        s.edit_json("wireguard.json", |doc| {
            let peers = doc["wireguard"]["peers"].as_array_mut().unwrap();
            let mut second = peers[0].clone();
            second["public_key"] = json!("TESTKEY99AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
            peers[0]["name"] = json!("phone");
            second["name"] = json!("phone");
            peers.push(second);
            let mut third = peers[0].clone();
            third["name"] = json!("ph one");
            peers.push(third);
        });
        let cfg = publishing(&[
            "wg_wireguard_phone_connected",
            "wg_wireguard_phone_2_connected",
            "wg_wireguard_ph_one_connected",
        ]);
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());
        let out = engine.full_pass(&s.source, s.source.now(), 0.0);
        let states: BTreeSet<&str> = out
            .iter()
            .filter(|p| p.topic.contains("/connected") && !p.topic.ends_with("/config"))
            .map(|p| p.topic.as_str())
            .collect();
        assert_eq!(states.len(), 3, "{states:#?}");
        let configs: BTreeSet<String> = out
            .iter()
            .filter(|p| p.topic.ends_with("/config") && p.topic.contains("connected"))
            .map(|p| p.topic.clone())
            .collect();
        assert_eq!(configs.len(), 3, "{configs:#?}");
    }

    struct WithCron<'a> {
        inner: &'a dyn Source,
        crontab: &'a str,
    }

    impl Source for WithCron<'_> {
        fn ubus(&self, o: &str, m: &str, a: Option<&str>) -> SourceResult<Value> {
            self.inner.ubus(o, m, a)
        }
        fn read(&self, path: &str) -> SourceResult<String> {
            if path == "/etc/crontabs/root" {
                Ok(self.crontab.to_string())
            } else {
                self.inner.read(path)
            }
        }
        fn list_dir(&self, path: &str) -> SourceResult<Vec<String>> {
            self.inner.list_dir(path)
        }
        fn command(&self, program: &str, args: &[&str]) -> SourceResult<String> {
            self.inner.command(program, args)
        }
        fn now(&self) -> u64 {
            self.inner.now()
        }
    }

    #[test]
    fn legacy_adoption_waits_until_the_legacy_cron_line_is_gone() {
        let src = fixture("primary");
        let cron = WithCron {
            inner: &src,
            crontab:
                "0 3 * * 3 /etc/init.d/adblock reload\n*/5 * * * * /usr/bin/publish_metrics.sh\n",
        };
        let mut cfg = Config::default();
        cfg.identity.device_id = Some("gw1".into());
        let mut engine = Engine::new(cfg, "router-primary", TopicState::default());

        // The legacy script still publishes: leave its retained topics alone.
        // Nothing is cleared, and nothing is adopted for later cleaning.
        let out = engine.full_pass(&cron, src.now(), 0.0);
        assert!(
            out.iter().all(|p| !p.payload.is_empty()),
            "cleared while the legacy script is scheduled: {:#?}",
            out.iter()
                .filter(|p| p.payload.is_empty())
                .collect::<Vec<_>>()
        );
        assert!(!out
            .iter()
            .any(|p| p.topic.contains("openwrt_router-primary_")));
        assert!(!engine.topics().legacy_seeded);

        // Once the cron line is removed the leftovers are adopted and cleared.
        let none = WithCron {
            inner: &src,
            crontab: "0 3 * * 3 /etc/init.d/adblock reload\n",
        };
        let out = engine.full_pass(&none, src.now(), 10.0);
        let cleared = out
            .iter()
            .filter(|p| p.payload.is_empty() && p.topic.contains("openwrt_router-primary_"))
            .count();
        // All 30 legacy discovery configs: the 22 that were announced plus
        // the 8 that could never have a value.
        assert_eq!(cleared, 30);
        assert!(engine.topics().legacy_seeded);
    }
}
