use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::composition::{BindingMode, Composition};
use crate::error::{Error, codes};

/// One replaceable implementation behind one or more capabilities.
///
/// `invoke` is the uniform call surface and mirrors the component wire
/// (`cap`, `op`, JSON `input`), so an in-process provider and a
/// component are interchangeable. A wrong input shape is an
/// `invalid-message` error, never a panic. `health` is cheap and side
/// effect free: swap calls it before mutating anything.
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> Vec<String>;
    fn health(&self) -> Result<(), Error>;
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error>;
}

/// A composition change or recovery action, kept for diagnostics
/// (`events` on `GET /api/plugins`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub at: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub capability: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub generation: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// One registered provider for operator surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub capabilities: Vec<String>,
    pub healthy: bool,
}

/// One provider's failure inside a merge-many call.
#[derive(Debug, Clone)]
pub struct ProviderError {
    pub provider: String,
    pub error: Error,
}

const MAX_EVENTS: usize = 256;

struct Inner {
    providers: BTreeMap<String, Arc<dyn Provider>>,
    comp: Composition,
    /// capability -> provider id that served before the last swap.
    last_good: HashMap<String, String>,
    /// capability -> module that suspended it (D-154 disable).
    suspended: BTreeMap<String, String>,
    events: VecDeque<Event>,
}

impl Inner {
    fn log(&mut self, kind: &str, capability: &str, provider: &str, generation: u64, detail: String) {
        self.events.push_back(Event {
            at: crate::now_unix(),
            kind: kind.to_string(),
            capability: capability.to_string(),
            provider: provider.to_string(),
            generation,
            detail,
        });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    /// Removes `id` from every binding naming it, falling back to the
    /// remaining providers or keeping the binding marked degraded.
    fn drop_from_bindings(&mut self, id: &str) {
        let mut log = Vec::new();
        for (cap, b) in self.comp.bindings.iter_mut() {
            if !b.providers.iter().any(|p| p == id) {
                continue;
            }
            let kept: Vec<String> = b.providers.iter().filter(|p| *p != id).cloned().collect();
            if kept.is_empty() {
                log.push((
                    "withdraw-degraded",
                    cap.clone(),
                    b.generation,
                    "no alternative provider; binding kept but degraded".to_string(),
                ));
                continue;
            }
            b.providers = kept.clone();
            b.generation += 1;
            log.push(("withdraw", cap.clone(), b.generation, format!("fallback to {kept:?}")));
        }
        for (kind, cap, generation, detail) in log {
            self.log(kind, &cap, id, generation, detail);
        }
    }
}

/// The runtime authority over the composition. All mutation goes
/// through swap/withdraw/suspend with health checks and generation
/// fencing: a broken provider fails validation while the previous
/// generation keeps serving.
pub struct Registry {
    inner: RwLock<Inner>,
}

fn serves(p: &dyn Provider, capability: &str) -> bool {
    p.capabilities().iter().any(|c| c == capability)
}

impl Registry {
    pub fn new(comp: Composition) -> Self {
        Registry {
            inner: RwLock::new(Inner {
                providers: BTreeMap::new(),
                comp,
                last_good: HashMap::new(),
                suspended: BTreeMap::new(),
                events: VecDeque::new(),
            }),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds (or replaces, by id) a provider. Registration alone never
    /// changes which provider serves a binding; only swap does. A
    /// component registering an existing id takes over its bindings.
    pub fn register(&self, p: Arc<dyn Provider>) {
        let mut g = self.write();
        let id = p.id().to_string();
        g.providers.insert(id.clone(), p);
        g.log("register", "", &id, 0, String::new());
    }

    /// Moves a capability to a new provider list after every
    /// candidate's health passes. `expected_gen == 0` skips fencing;
    /// otherwise a mismatch fails with `stale-generation`. On error
    /// the active generation is returned with it.
    pub fn swap(&self, capability: &str, providers: &[String], expected_gen: u64) -> Result<u64, (Error, u64)> {
        let (candidates, generation) = {
            let g = self.read();
            let Some(b) = g.comp.bindings.get(capability) else {
                return Err((Error::invalid(format!("unknown capability {capability}")), 0));
            };
            if expected_gen != 0 && expected_gen != b.generation {
                return Err((
                    Error::new(
                        codes::STALE_GENERATION,
                        format!("expected generation {expected_gen}, active is {}", b.generation),
                    ),
                    b.generation,
                ));
            }
            if b.mode == BindingMode::ExactlyOne && providers.len() != 1 {
                return Err((Error::invalid("exactly-one needs 1 provider"), b.generation));
            }
            if providers.is_empty() {
                return Err((Error::invalid("at least one provider required"), b.generation));
            }
            let mut out = Vec::new();
            for id in providers {
                let Some(p) = g.providers.get(id) else {
                    return Err((Error::invalid(format!("unknown provider {id}")), b.generation));
                };
                if !serves(p.as_ref(), capability) {
                    return Err((Error::invalid(format!("provider {id} does not serve {capability}")), b.generation));
                }
                out.push(p.clone());
            }
            (out, b.generation)
        };
        // Health runs outside the lock: a component's liveness op is a
        // socket round-trip and must not stall every other caller.
        for p in &candidates {
            if let Err(e) = p.health() {
                let mut g = self.write();
                g.log("swap-rejected", capability, p.id(), generation, e.to_string());
                return Err((Error::unavailable(format!("provider {} unhealthy: {e}", p.id())), generation));
            }
        }
        let mut g = self.write();
        let b = g.comp.bindings.get_mut(capability).expect("binding vanished");
        if b.generation != generation {
            let active = b.generation;
            return Err((
                Error::new(codes::STALE_GENERATION, format!("binding changed during swap; active is {active}")),
                active,
            ));
        }
        let prev = std::mem::replace(&mut b.providers, providers.to_vec());
        b.generation += 1;
        let next = b.generation;
        if prev.len() == 1 {
            g.last_good.insert(capability.to_string(), prev[0].clone());
        }
        g.log("swap", capability, &providers[0], next, format!("from {prev:?}"));
        Ok(next)
    }

    /// Removes a provider from serving without deleting it. Bindings
    /// that referenced it fall back to the remaining providers; one with
    /// no alternative stays marked degraded instead of silently serving
    /// nothing.
    pub fn withdraw(&self, id: &str) -> Result<(), Error> {
        let mut g = self.write();
        if !g.providers.contains_key(id) {
            return Err(Error::invalid(format!("unknown provider {id}")));
        }
        g.drop_from_bindings(id);
        Ok(())
    }

    /// Withdraws a provider from every binding and deletes its
    /// registration (a component whose manifest vanished).
    pub fn unregister(&self, id: &str) -> Result<(), Error> {
        let mut g = self.write();
        if !g.providers.contains_key(id) {
            return Err(Error::invalid(format!("unknown provider {id}")));
        }
        g.drop_from_bindings(id);
        g.providers.remove(id);
        g.last_good.retain(|_, pid| pid != id);
        g.log("unregister", "", id, 0, String::new());
        Ok(())
    }

    /// Unbinds the capabilities of a disabled module (D-154): calls
    /// answer `module-disabled`, the binding's generation moves so no
    /// earlier reference stays valid, and the provider list is kept so
    /// enabling restores it exactly. Unknown capabilities are skipped.
    pub fn suspend(&self, module: &str, capabilities: &[String]) -> Vec<String> {
        let mut g = self.write();
        let mut changed = Vec::new();
        for cap in capabilities {
            if g.suspended.contains_key(cap) {
                continue;
            }
            let Some(b) = g.comp.bindings.get_mut(cap) else { continue };
            b.generation += 1;
            let generation = b.generation;
            g.suspended.insert(cap.clone(), module.to_string());
            g.log("module-disabled", cap, "", generation, format!("module {module}"));
            changed.push(cap.clone());
        }
        changed
    }

    /// Rebinds what `suspend` unbound for this module.
    pub fn resume(&self, module: &str, capabilities: &[String]) -> Vec<String> {
        let mut g = self.write();
        let mut changed = Vec::new();
        for cap in capabilities {
            if g.suspended.get(cap).map(String::as_str) != Some(module) {
                continue;
            }
            g.suspended.remove(cap);
            if let Some(b) = g.comp.bindings.get_mut(cap) {
                b.generation += 1;
                let generation = b.generation;
                g.log("module-enabled", cap, "", generation, format!("module {module}"));
            }
            changed.push(cap.clone());
        }
        changed
    }

    /// The module that suspended a capability, if any.
    pub fn suspended_by(&self, capability: &str) -> Option<String> {
        self.read().suspended.get(capability).cloned()
    }

    fn bound(&self, capability: &str) -> Result<(Vec<Arc<dyn Provider>>, u64), Error> {
        let g = self.read();
        let Some(b) = g.comp.bindings.get(capability) else {
            return Err(Error::invalid(format!("unknown capability {capability}")));
        };
        if let Some(module) = g.suspended.get(capability) {
            return Err(Error::new(
                codes::MODULE_DISABLED,
                format!("{capability} belongs to the disabled module {module}"),
            ));
        }
        let provs = b.providers.iter().filter_map(|id| g.providers.get(id).cloned()).collect();
        Ok((provs, b.generation))
    }

    /// The healthy providers for a capability, in binding order, with
    /// the binding's generation.
    pub fn ordered(&self, capability: &str) -> Result<(Vec<Arc<dyn Provider>>, u64), Error> {
        let (provs, generation) = self.bound(capability)?;
        let healthy: Vec<_> = provs.into_iter().filter(|p| p.health().is_ok()).collect();
        if healthy.is_empty() {
            return Err(Error::unavailable(format!("no healthy provider for {capability}")));
        }
        Ok((healthy, generation))
    }

    /// Invokes an exactly-one capability. If the active provider fails
    /// at call time, the last-good provider is tried once and the
    /// incident is logged. Returns the output and the serving id.
    pub fn call_one(&self, capability: &str, op: &str, input: Value) -> Result<(Value, String), Error> {
        let (provs, generation) = self.ordered(capability)?;
        let active = &provs[0];
        let err = match active.invoke(capability, op, input.clone()) {
            Ok(out) => return Ok((out, active.id().to_string())),
            Err(e) => e,
        };
        let fallback = {
            let g = self.read();
            g.last_good.get(capability).and_then(|id| g.providers.get(id).cloned())
        };
        if let Some(fb) = fallback {
            if fb.id() != active.id() && fb.health().is_ok() {
                if let Ok(out) = fb.invoke(capability, op, input) {
                    let mut g = self.write();
                    g.log("fallback", capability, fb.id(), generation, format!("active {} failed: {err}", active.id()));
                    return Ok((out, fb.id().to_string()));
                }
            }
        }
        Err(err)
    }

    /// Tries ordered providers until one's output is accepted. `None`
    /// means nobody accepted; the last provider error is returned when
    /// every provider failed.
    pub fn call_first(
        &self,
        capability: &str,
        op: &str,
        input: Value,
        accepted: impl Fn(&Value) -> bool,
    ) -> Result<Option<(Value, String)>, Error> {
        let (provs, _) = self.ordered(capability)?;
        let mut last_err = None;
        for p in provs {
            match p.invoke(capability, op, input.clone()) {
                Ok(out) if accepted(&out) => return Ok(Some((out, p.id().to_string()))),
                Ok(_) => {}
                Err(e) => last_err = Some(e),
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }

    /// Fans out to every healthy provider of a merge-many capability.
    /// Failures degrade the merge and are reported; only total failure
    /// is `dependency-unavailable`. Outputs come back with their ids.
    pub fn call_merge_report(
        &self,
        capability: &str,
        op: &str,
        input: Value,
    ) -> Result<(Vec<(String, Value)>, Vec<ProviderError>), Error> {
        let (provs, _) = self.ordered(capability)?;
        let mut outputs = Vec::new();
        let mut failed = Vec::new();
        for p in provs {
            match p.invoke(capability, op, input.clone()) {
                Ok(out) => outputs.push((p.id().to_string(), out)),
                Err(error) => failed.push(ProviderError { provider: p.id().to_string(), error }),
            }
        }
        if outputs.is_empty() {
            return Err(Error::unavailable(format!("all providers failed for {capability}")));
        }
        Ok((outputs, failed))
    }

    /// Delivers to every healthy provider of a fan-out capability and
    /// reports each provider's own result. No provider's failure stops
    /// delivery to the others.
    pub fn call_fan_out(
        &self,
        capability: &str,
        op: &str,
        input: Value,
    ) -> Result<Vec<(String, Result<Value, Error>)>, Error> {
        let (provs, _) = self.ordered(capability)?;
        Ok(provs.into_iter().map(|p| (p.id().to_string(), p.invoke(capability, op, input.clone()))).collect())
    }

    /// Calls one named provider of a capability, still through
    /// authority: registered, bound, serving and healthy.
    pub fn invoke_provider(&self, capability: &str, provider: &str, op: &str, input: Value) -> Result<Value, Error> {
        let (provs, _) = self.bound(capability)?;
        let g = self.read();
        if !g.providers.contains_key(provider) {
            return Err(Error::invalid(format!("unknown provider {provider}")));
        }
        drop(g);
        let Some(p) = provs.into_iter().find(|p| p.id() == provider) else {
            return Err(Error::invalid(format!("provider {provider} not bound to {capability}")));
        };
        if !serves(p.as_ref(), capability) {
            return Err(Error::invalid(format!("provider {provider} does not serve {capability}")));
        }
        if p.health().is_err() {
            return Err(Error::unavailable(format!("provider {provider} unhealthy")));
        }
        p.invoke(capability, op, input)
    }

    /// Whether a provider id is registered.
    pub fn has_provider(&self, id: &str) -> bool {
        self.read().providers.contains_key(id)
    }

    /// Registered provider ids, sorted.
    pub fn providers(&self) -> Vec<String> {
        self.read().providers.keys().cloned().collect()
    }

    /// Provider details with live health, sorted by id.
    pub fn provider_infos(&self) -> Vec<ProviderInfo> {
        let provs: Vec<_> = self.read().providers.values().cloned().collect();
        provs
            .into_iter()
            .map(|p| ProviderInfo {
                id: p.id().to_string(),
                capabilities: p.capabilities(),
                healthy: p.health().is_ok(),
            })
            .collect()
    }

    /// Whether every provider bound to a capability is registered and
    /// at least one is healthy (module health).
    pub fn capability_healthy(&self, capability: &str) -> bool {
        self.ordered(capability).is_ok()
    }

    /// The recent composition/recovery log (never null on the wire).
    pub fn events(&self) -> Vec<Event> {
        self.read().events.iter().cloned().collect()
    }

    /// A copy of the live composition.
    pub fn composition(&self) -> Composition {
        self.read().comp.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Binding;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Stub {
        id: String,
        caps: Vec<String>,
        healthy: AtomicBool,
        fail: AtomicBool,
        calls: AtomicUsize,
        answer: Value,
    }

    fn stub(id: &str, caps: &[&str], answer: Value) -> Arc<Stub> {
        Arc::new(Stub {
            id: id.into(),
            caps: caps.iter().map(|s| s.to_string()).collect(),
            healthy: AtomicBool::new(true),
            fail: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            answer,
        })
    }

    impl Provider for Stub {
        fn id(&self) -> &str {
            &self.id
        }
        fn capabilities(&self) -> Vec<String> {
            self.caps.clone()
        }
        fn health(&self) -> Result<(), Error> {
            if self.healthy.load(Ordering::SeqCst) { Ok(()) } else { Err(Error::unavailable("down")) }
        }
        fn invoke(&self, _cap: &str, _op: &str, _input: Value) -> Result<Value, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::internal("boom"));
            }
            Ok(self.answer.clone())
        }
    }

    const CAT: &str = "lain.catalog.read@1";
    const IDENT: &str = "lain.media.identify@1";
    const META: &str = "lain.metadata.search@1";

    fn registry() -> (Registry, Arc<Stub>, Arc<Stub>, Arc<Stub>, Arc<Stub>) {
        let mut bindings = BTreeMap::new();
        bindings.insert(CAT.to_string(), Binding::new(BindingMode::ExactlyOne, &["cat-a"]));
        bindings.insert(IDENT.to_string(), Binding::new(BindingMode::OrderedMany, &["id-a", "id-b"]));
        bindings.insert(META.to_string(), Binding::new(BindingMode::MergeMany, &["id-a", "id-b"]));
        let reg = Registry::new(Composition { version: 1, bindings });
        let cat_a = stub("cat-a", &[CAT], Value::from("a"));
        let cat_b = stub("cat-b", &[CAT], Value::from("b"));
        let id_a = stub("id-a", &[IDENT, META], serde_json::json!({"title": ""}));
        let id_b = stub("id-b", &[IDENT, META], serde_json::json!({"title": "Frieren"}));
        reg.register(cat_a.clone());
        reg.register(cat_b.clone());
        reg.register(id_a.clone());
        reg.register(id_b.clone());
        (reg, cat_a, cat_b, id_a, id_b)
    }

    #[test]
    fn swap_is_health_gated_and_fenced() {
        let (reg, _a, b, _, _) = registry();
        b.healthy.store(false, Ordering::SeqCst);
        let (err, generation) = reg.swap(CAT, &["cat-b".into()], 1).unwrap_err();
        assert_eq!(err.code, codes::DEPENDENCY_UNAVAILABLE);
        assert_eq!(generation, 1);
        assert_eq!(reg.composition().bindings[CAT].providers, vec!["cat-a".to_string()]);
        b.healthy.store(true, Ordering::SeqCst);
        assert_eq!(reg.swap(CAT, &["cat-b".into()], 1).unwrap(), 2);
        let (err, generation) = reg.swap(CAT, &["cat-a".into()], 1).unwrap_err();
        assert_eq!(err.code, codes::STALE_GENERATION);
        assert_eq!(generation, 2);
        assert_eq!(reg.swap(CAT, &["cat-a".into(), "cat-b".into()], 0).unwrap_err().0.code, codes::INVALID_MESSAGE);
        assert_eq!(reg.swap(CAT, &["id-a".into()], 0).unwrap_err().0.code, codes::INVALID_MESSAGE);
        assert_eq!(reg.swap("lain.nope@1", &["cat-a".into()], 0).unwrap_err().0.code, codes::INVALID_MESSAGE);
        assert!(reg.events().iter().any(|e| e.kind == "swap-rejected"));
    }

    #[test]
    fn call_one_falls_back_to_last_good() {
        let (reg, a, b, _, _) = registry();
        reg.swap(CAT, &["cat-b".into()], 0).unwrap();
        b.fail.store(true, Ordering::SeqCst);
        let (out, served) = reg.call_one(CAT, "get", Value::Null).unwrap();
        assert_eq!((out, served.as_str()), (Value::from("a"), "cat-a"));
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        assert!(reg.events().iter().any(|e| e.kind == "fallback"));
        a.fail.store(true, Ordering::SeqCst);
        assert_eq!(reg.call_one(CAT, "get", Value::Null).unwrap_err().msg, "boom");
    }

    #[test]
    fn call_first_takes_the_first_accepted() {
        let (reg, _, _, a, b) = registry();
        let accepted = |v: &Value| v["title"].as_str().is_some_and(|t| !t.is_empty());
        let (out, served) = reg.call_first(IDENT, "identify", Value::Null, accepted).unwrap().unwrap();
        assert_eq!(served, "id-b");
        assert_eq!(out["title"], "Frieren");
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        b.healthy.store(false, Ordering::SeqCst);
        assert!(reg.call_first(IDENT, "identify", Value::Null, accepted).unwrap().is_none());
    }

    #[test]
    fn merge_reports_failures_and_fails_only_when_all_fail() {
        let (reg, _, _, a, b) = registry();
        a.fail.store(true, Ordering::SeqCst);
        let (outs, failed) = reg.call_merge_report(META, "search", Value::Null).unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(failed[0].provider, "id-a");
        b.fail.store(true, Ordering::SeqCst);
        assert_eq!(reg.call_merge_report(META, "search", Value::Null).unwrap_err().code, codes::DEPENDENCY_UNAVAILABLE);
    }

    #[test]
    fn withdraw_falls_back_or_degrades() {
        let (reg, _, _, _, _) = registry();
        reg.withdraw("id-a").unwrap();
        let comp = reg.composition();
        assert_eq!(comp.bindings[IDENT].providers, vec!["id-b".to_string()]);
        assert_eq!(comp.bindings[IDENT].generation, 2);
        reg.withdraw("cat-a").unwrap();
        // exactly-one with no alternative: kept, degraded, logged.
        assert_eq!(reg.composition().bindings[CAT].providers, vec!["cat-a".to_string()]);
        assert!(reg.events().iter().any(|e| e.kind == "withdraw-degraded"));
        assert!(reg.has_provider("cat-a"));
        assert!(reg.withdraw("nobody").is_err());
    }

    #[test]
    fn unregister_removes_the_provider_and_last_good() {
        let (reg, _, b, _, _) = registry();
        reg.swap(CAT, &["cat-b".into()], 0).unwrap();
        reg.unregister("cat-a").unwrap();
        assert!(!reg.has_provider("cat-a"));
        b.fail.store(true, Ordering::SeqCst);
        assert_eq!(reg.call_one(CAT, "get", Value::Null).unwrap_err().msg, "boom");
    }

    #[test]
    fn suspend_answers_module_disabled_and_resume_restores() {
        let (reg, _, _, _, _) = registry();
        let caps = vec![IDENT.to_string()];
        assert_eq!(reg.suspend("identify", &caps), caps);
        assert_eq!(reg.composition().bindings[IDENT].generation, 2);
        let err = reg.call_first(IDENT, "identify", Value::Null, |_| true).unwrap_err();
        assert_eq!(err.code, codes::MODULE_DISABLED);
        assert_eq!(reg.suspended_by(IDENT).as_deref(), Some("identify"));
        // Another module cannot resume what it did not suspend.
        assert!(reg.resume("watch", &caps).is_empty());
        assert_eq!(reg.resume("identify", &caps), caps);
        assert_eq!(reg.composition().bindings[IDENT].providers, vec!["id-a".to_string(), "id-b".to_string()]);
        assert_eq!(reg.composition().bindings[IDENT].generation, 3);
        assert!(reg.call_first(IDENT, "identify", Value::Null, |_| true).unwrap().is_some());
    }

    #[test]
    fn invoke_provider_checks_authority() {
        let (reg, _, _, _, _) = registry();
        assert!(reg.invoke_provider(IDENT, "id-b", "identify", Value::Null).is_ok());
        assert_eq!(
            reg.invoke_provider(IDENT, "cat-a", "identify", Value::Null).unwrap_err().code,
            codes::INVALID_MESSAGE
        );
        assert_eq!(
            reg.invoke_provider(IDENT, "ghost", "identify", Value::Null).unwrap_err().code,
            codes::INVALID_MESSAGE
        );
    }

    #[test]
    fn event_log_is_bounded() {
        let (reg, _, _, _, _) = registry();
        for _ in 0..300 {
            reg.register(stub("x", &[CAT], Value::Null));
        }
        assert_eq!(reg.events().len(), MAX_EVENTS);
    }
}
