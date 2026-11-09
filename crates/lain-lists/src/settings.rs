//! `lain.settings.integrations@1` (D-080, D-198): the operator's
//! external-platform credentials, served by `lain-settings-integrations`
//! over `lain_admin::integrations` (the `meta` table's `integrations`
//! document). Exactly one provider owns it; secrets never leave the
//! server.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use lain_admin::integrations::{self, IntegrationSettings};
use lain_core::{Error, Provider};
use lain_store::Store;

use crate::CAP_INTEGRATION_SETTINGS;

pub const PROVIDER_ID: &str = "lain-settings-integrations";

/// Op names: `get` → the settings; `put {settings}` stores them;
/// `ensure {base}` seeds only when no document exists.
pub mod op {
    pub const GET: &str = "get";
    pub const PUT: &str = "put";
    pub const ENSURE: &str = "ensure";
}

pub struct IntegrationsProvider {
    store: Arc<Store>,
}

impl IntegrationsProvider {
    pub fn new(store: Arc<Store>) -> Self {
        IntegrationsProvider { store }
    }
}

#[derive(Deserialize, Default)]
struct PutIn {
    #[serde(default)]
    settings: IntegrationSettings,
}

#[derive(Deserialize, Default)]
struct EnsureIn {
    #[serde(default)]
    base: IntegrationSettings,
}

impl Provider for IntegrationsProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_INTEGRATION_SETTINGS.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_INTEGRATION_SETTINGS {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let bad = |what: &str| Error::invalid(format!("{what} required"));
        match op {
            op::GET => Ok(json!(integrations::get(&self.store))),
            op::PUT => {
                let i: PutIn = serde_json::from_value(input).map_err(|_| bad("IntPutInput"))?;
                integrations::put(&self.store, &i.settings).map_err(|e| Error::internal(e.to_string()))?;
                Ok(json!(i.settings))
            }
            op::ENSURE => {
                let i: EnsureIn = serde_json::from_value(input).map_err(|_| bad("IntEnsureInput"))?;
                Ok(json!(integrations::ensure(&self.store, &i.base).map_err(|e| Error::internal(e.to_string()))?))
            }
            _ => Err(Error::invalid(format!("unknown op {op} of {cap}"))),
        }
    }
}
