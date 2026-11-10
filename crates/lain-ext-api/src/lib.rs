//! The guest SDK for lain client extensions (tier 3 of the modding model,
//! DESIGN-v2 §8.3, `docs/MODDING.md`).
//!
//! An extension is a WebAssembly component built from a `cdylib` crate
//! for `wasm32-wasip2`. It implements [`Extension`] and registers it:
//!
//! ```ignore
//! use lain_ext_api::{Extension, Rail, register_extension};
//!
//! struct Hello;
//!
//! impl Extension for Hello {
//!     fn new(_config: &str) -> Result<Self, String> {
//!         Ok(Hello)
//!     }
//!     fn rails(&mut self) -> Vec<Rail> {
//!         vec![Rail { id: "hello".into(), title: "Hello".into(), cards: vec![] }]
//!     }
//! }
//!
//! register_extension!(Hello);
//! ```
//!
//! Host calls ([`host`]) succeed only for the scopes the user granted;
//! anything else answers [`HostError::Denied`] with the missing scope.

#[doc(hidden)]
pub mod wit {
    wit_bindgen::generate!({
        path: "wit",
        world: "extension",
        pub_export_macro: true,
        default_bindings_module: "lain_ext_api::wit",
    });
}

pub use wit::lain::extension::host;
pub use wit::lain::extension::host::LogLevel;
pub use wit::lain::extension::types::{
    Card, Event, HostError, HttpResponse, Item, Library, Panel, Progress, Rail, Row,
};

/// The version of the `lain:extension` world this SDK targets.
pub const API_VERSION: &str = "0.1.0";

/// What an extension implements. Every method but [`Extension::new`] has
/// a default that does nothing, so an extension writes only what it uses.
pub trait Extension: 'static {
    /// Called once when the extension loads, with the `config` object of
    /// its `extension.json` as JSON (`{}` when absent). An error disables
    /// the extension and shows the message to the user.
    fn new(config: &str) -> Result<Self, String>
    where
        Self: Sized;

    /// Rails for Home (needs `ui:rail`).
    fn rails(&mut self) -> Vec<Rail> {
        Vec::new()
    }

    /// Panels for an item's page (needs `ui:panel`).
    fn panels(&mut self, item_id: &str) -> Vec<Panel> {
        let _ = item_id;
        Vec::new()
    }

    /// An event (needs `events:subscribe`). Call [`host::invalidate`] to
    /// have the host ask for [`Extension::rails`] again.
    fn on_event(&mut self, event: Event) {
        let _ = event;
    }
}

/// Logs at `info` (the app's log, prefixed with the extension id).
pub fn log(message: impl AsRef<str>) {
    host::log(LogLevel::Info, message.as_ref());
}

/// Logs at `warn`.
pub fn warn(message: impl AsRef<str>) {
    host::log(LogLevel::Warn, message.as_ref());
}

/// Whether the user granted `scope`.
pub fn granted(scope: &str) -> bool {
    host::granted_scopes().iter().any(|s| s == scope)
}

/// The whole catalog, paged by 500 (`catalog:read`).
pub fn catalog() -> Result<Vec<Item>, HostError> {
    let mut out = Vec::new();
    loop {
        let page = host::list_catalog(out.len() as u32, 500)?;
        let n = page.len();
        out.extend(page);
        if n < 500 {
            return Ok(out);
        }
    }
}

#[doc(hidden)]
pub mod __private {
    use std::cell::RefCell;

    use super::{Event, Extension, Panel, Rail};

    thread_local! {
        static EXT: RefCell<Option<Box<dyn Extension>>> = RefCell::new(None);
    }

    pub fn activate<T: Extension>(config: String) -> Result<(), String> {
        let ext = T::new(&config)?;
        EXT.with(|e| *e.borrow_mut() = Some(Box::new(ext)));
        Ok(())
    }

    fn with<R>(default: R, f: impl FnOnce(&mut dyn Extension) -> R) -> R {
        EXT.with(|e| match e.borrow_mut().as_mut() {
            Some(ext) => f(ext.as_mut()),
            None => default,
        })
    }

    pub fn rails() -> Vec<Rail> {
        with(Vec::new(), |e| e.rails())
    }

    pub fn panels(item_id: String) -> Vec<Panel> {
        with(Vec::new(), |e| e.panels(&item_id))
    }

    pub fn on_event(event: Event) {
        with((), |e| e.on_event(event))
    }
}

/// Registers the type that implements [`Extension`] as this component's
/// exports. Use it once, at the crate root of a `cdylib`.
#[macro_export]
macro_rules! register_extension {
    ($ty:ty) => {
        const _: () = {
            struct __LainExtension;

            impl $crate::wit::Guest for __LainExtension {
                fn activate(config: String) -> Result<(), String> {
                    $crate::__private::activate::<$ty>(config)
                }
                fn rails() -> Vec<$crate::Rail> {
                    $crate::__private::rails()
                }
                fn panels(item_id: String) -> Vec<$crate::Panel> {
                    $crate::__private::panels(item_id)
                }
                fn on_event(event: $crate::Event) {
                    $crate::__private::on_event(event)
                }
            }

    $crate::wit::export!(__LainExtension with_types_in $crate::wit);
        };
    };
}
