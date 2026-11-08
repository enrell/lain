use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// How a capability composes providers. The mode is part of the
/// contract: two installed providers never resolve by "who started
/// first wins".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingMode {
    /// A single authoritative owner (catalog, userstate).
    #[serde(rename = "exactly-one")]
    ExactlyOne,
    /// Providers in declared order; the first accepted answer wins.
    #[serde(rename = "ordered-many")]
    OrderedMany,
    /// Tried in order until one accepts (planners).
    #[serde(rename = "first-accepted")]
    FirstAccepted,
    /// Every provider answers; results merge with provenance (metadata).
    #[serde(rename = "merge-many")]
    MergeMany,
    /// Delivered to every provider (events, sync, webhooks).
    #[serde(rename = "fan-out")]
    FanOut,
}

impl BindingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BindingMode::ExactlyOne => "exactly-one",
            BindingMode::OrderedMany => "ordered-many",
            BindingMode::FirstAccepted => "first-accepted",
            BindingMode::MergeMany => "merge-many",
            BindingMode::FanOut => "fan-out",
        }
    }
}

/// The active choice of providers for one capability. `generation` is
/// a monotonic epoch: replacing a provider never revalidates an old
/// reference (`stale-generation` on mismatch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub mode: BindingMode,
    pub providers: Vec<String>,
    pub generation: u64,
}

impl Binding {
    pub fn new(mode: BindingMode, providers: &[&str]) -> Self {
        Binding { mode, providers: providers.iter().map(|s| s.to_string()).collect(), generation: 1 }
    }
}

/// The installed choice of implementations (`composition.json`, the
/// same document shape the Go server writes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composition {
    pub version: u32,
    pub bindings: BTreeMap<String, Binding>,
}

/// A stable sorted view of one binding for inspection endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingView {
    pub capability: String,
    pub mode: String,
    pub providers: Vec<String>,
    pub generation: u64,
}

impl Composition {
    /// Validates against registered provider ids: every binding names
    /// at least one known provider and exactly-one names exactly one.
    pub fn validate(&self, known: &BTreeSet<String>) -> Result<(), String> {
        if self.bindings.is_empty() {
            return Err("empty composition".into());
        }
        for (cap, b) in &self.bindings {
            if b.providers.is_empty() {
                return Err(format!("capability {cap}: no providers"));
            }
            if b.mode == BindingMode::ExactlyOne && b.providers.len() != 1 {
                return Err(format!("capability {cap}: exactly-one needs 1 provider, got {}", b.providers.len()));
            }
            for id in &b.providers {
                if !known.contains(id) {
                    return Err(format!("capability {cap}: unknown provider {id}"));
                }
            }
        }
        Ok(())
    }

    /// Adds bindings for capabilities the saved composition does not
    /// know yet, keeping every override. Returns the added names.
    pub fn upgrade(&mut self, fresh: &Composition) -> Vec<String> {
        let mut added = Vec::new();
        for (cap, b) in &fresh.bindings {
            if !self.bindings.contains_key(cap) {
                self.bindings.insert(cap.clone(), b.clone());
                added.push(cap.clone());
            }
        }
        if self.version < fresh.version {
            self.version = fresh.version;
        }
        added
    }

    /// Rewrites retired provider ids in place, reporting what moved.
    pub fn migrate_provider_ids(&mut self, retired: &[(&str, &str)]) -> Vec<String> {
        let mut moved = Vec::new();
        for b in self.bindings.values_mut() {
            for id in b.providers.iter_mut() {
                if let Some((_, next)) = retired.iter().find(|(old, _)| old == id) {
                    moved.push(format!("{id}->{next}"));
                    *id = next.to_string();
                }
            }
        }
        moved
    }

    /// Bindings sorted by capability name.
    pub fn view(&self) -> Vec<BindingView> {
        self.bindings
            .iter()
            .map(|(cap, b)| BindingView {
                capability: cap.clone(),
                mode: b.mode.as_str().to_string(),
                providers: b.providers.clone(),
                generation: b.generation,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp() -> Composition {
        let mut bindings = BTreeMap::new();
        bindings.insert("lain.catalog.read@1".into(), Binding::new(BindingMode::ExactlyOne, &["cat"]));
        bindings.insert("lain.media.identify@1".into(), Binding::new(BindingMode::OrderedMany, &["a", "b"]));
        Composition { version: 1, bindings }
    }

    #[test]
    fn validate_rejects_unknown_and_bad_exactly_one() {
        let c = comp();
        let known: BTreeSet<String> = ["cat", "a", "b"].iter().map(|s| s.to_string()).collect();
        assert!(c.validate(&known).is_ok());
        let partial: BTreeSet<String> = ["cat", "a"].iter().map(|s| s.to_string()).collect();
        assert!(c.validate(&partial).unwrap_err().contains("unknown provider b"));
        let mut two = c.clone();
        two.bindings.get_mut("lain.catalog.read@1").unwrap().providers.push("a".into());
        assert!(two.validate(&known).unwrap_err().contains("exactly-one"));
        let empty = Composition { version: 1, bindings: BTreeMap::new() };
        assert!(empty.validate(&known).is_err());
    }

    #[test]
    fn upgrade_keeps_overrides_and_adds_new() {
        let mut saved = comp();
        saved.bindings.get_mut("lain.media.identify@1").unwrap().providers = vec!["b".into()];
        let mut fresh = comp();
        fresh.version = 3;
        fresh.bindings.insert("lain.new@1".into(), Binding::new(BindingMode::FanOut, &["x"]));
        let added = saved.upgrade(&fresh);
        assert_eq!(added, vec!["lain.new@1".to_string()]);
        assert_eq!(saved.bindings["lain.media.identify@1"].providers, vec!["b".to_string()]);
        assert_eq!(saved.version, 3);
    }

    #[test]
    fn document_shape_matches_go() {
        let raw = r#"{"version":8,"bindings":{"lain.catalog.read@1":{"mode":"exactly-one","providers":["lain-catalog-bolt"],"generation":1}}}"#;
        let mut c: Composition = serde_json::from_str(raw).unwrap();
        let moved = c.migrate_provider_ids(&[("lain-catalog-bolt", "lain-catalog-redb")]);
        assert_eq!(moved, vec!["lain-catalog-bolt->lain-catalog-redb".to_string()]);
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["bindings"]["lain.catalog.read@1"]["mode"], "exactly-one");
    }
}
