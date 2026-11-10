//! Bridges to the owner's standalone apps (DESIGN-v2 §6, P4): each one
//! is a [`lain_core::Provider`] that talks to an app over its own local
//! API and runs as a lain component (`lain component run <id> --sock
//! {sock}`), so the apps keep their own lifecycles, users and release
//! channels and lain sees them only through versioned capabilities.
//!
//! | Provider | Capability | App |
//! |---|---|---|
//! | [`parser::MODEL_ID`] | `lain.release.parse@1` | lain-parser (filename-parser) |
//! | [`animesubs::ID`] | `lain.subtitle.translate@1` | AnimeSubs' local server |
//! | [`anything::ID`] | `lain.acquire.extract@1` | the Anything daemon |
//! | [`library::ID`] | `lain.media.identify@1`, `lain.metadata.search@1`, `lain.metadata.resolve@1` | the files Anything leaves next to its downloads |
//!
//! Every bridge declines instead of failing the binding: an app that is
//! not installed, not running or not ready fails `health` with
//! `dependency-unavailable`, and the registry's ordered bindings skip
//! it. Nothing here starts the apps; they run under their own units.

pub mod animesubs;
pub mod anything;
pub mod discovery;
#[doc(hidden)]
pub mod fake;
pub mod http1;
pub mod library;
pub mod parser;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lain_core::Provider;

/// Options of `lain component run` (`--name value` pairs).
pub type Options = BTreeMap<String, String>;

/// Every bridge id with its capabilities and a one-line summary, for
/// `lain component list` and manifests.
pub fn catalog() -> Vec<(&'static str, Vec<&'static str>, &'static str)> {
    vec![
        (parser::MODEL_ID, vec![lain_identify::CAP_RELEASE_PARSE], "the lain-parser model (filename-parser)"),
        (animesubs::ID, vec![animesubs::CAP_SUBTITLE_TRANSLATE], "subtitle translation by AnimeSubs' local server"),
        (anything::ID, vec![anything::CAP_ACQUIRE_EXTRACT], "web media extraction by the Anything daemon"),
        (
            library::ID,
            vec![lain_identify::CAP_MEDIA_IDENTIFY, library::CAP_METADATA_SEARCH, library::CAP_METADATA_RESOLVE],
            "identity and metadata from the files Anything leaves next to its downloads",
        ),
    ]
}

/// Builds the bridge `id` from its options.
pub fn build(id: &str, opts: &Options) -> Result<Arc<dyn Provider>, String> {
    if let Some(k) = opts.keys().find(|k| !known_options(id).contains(&k.as_str()))
        && catalog().iter().any(|(i, _, _)| *i == id)
    {
        return Err(format!("{id} has no option --{k}"));
    }
    let data_dir = opts.get("data-dir").map(PathBuf::from);
    match id {
        parser::MODEL_ID => {
            let ep = parser::ReleaseModel::locate(opts.get("parser").map(String::as_str), data_dir.as_deref())?;
            Ok(Arc::new(parser::ReleaseModel::new(ep)))
        }
        animesubs::ID => {
            let loc = animesubs::AnimeSubs::locator(opts.get("home").map(PathBuf::from), opts.get("url").cloned());
            let model = opts.get("model-kind").map(|kind| lain_api::TranslateModel {
                kind: kind.clone(),
                model: opts.get("model").cloned().unwrap_or_default(),
                base_url: opts.get("base-url").cloned().unwrap_or_default(),
                api_key: String::new(),
            });
            Ok(Arc::new(animesubs::AnimeSubs::new(loc, model)))
        }
        anything::ID => {
            let loc = anything::Anything::locator(opts.get("home").map(PathBuf::from), opts.get("url").cloned());
            Ok(Arc::new(anything::Anything::new(loc)))
        }
        library::ID => Ok(Arc::new(library::AnythingLibrary)),
        other => Err(format!("unknown bridge {other}")),
    }
}

/// The option names each bridge reads (`lain component install`
/// refuses others, so a typo is not silently ignored). Tokens are never
/// options: they come from the app's token file or its own environment.
pub fn known_options(id: &str) -> &'static [&'static str] {
    match id {
        parser::MODEL_ID => &["parser", "data-dir"],
        animesubs::ID => &["home", "url", "model-kind", "model", "base-url", "data-dir"],
        anything::ID => &["home", "url", "data-dir"],
        library::ID => &["data-dir"],
        _ => &[],
    }
}

/// The manifest that runs bridge `id` from this executable: dropped
/// into `<data-dir>/plugins/<id>.json`, the provisioner spawns
/// `<exe> component run <id> --sock {sock} --<k> <v>…`.
pub fn manifest(
    id: &str,
    exe: &std::path::Path,
    version: &str,
    opts: &Options,
) -> Result<lain_component::Manifest, String> {
    let (_, caps, _) =
        catalog().into_iter().find(|(i, _, _)| *i == id).ok_or_else(|| format!("unknown bridge {id}"))?;
    let mut args = vec!["component".to_string(), "run".into(), id.into(), "--sock".into(), "{sock}".into()];
    for (k, v) in opts {
        args.push(format!("--{k}"));
        args.push(v.clone());
    }
    Ok(lain_component::Manifest {
        id: id.into(),
        version: version.into(),
        capabilities: caps.into_iter().map(String::from).collect(),
        execution: lain_component::manifest::Execution {
            kind: "process".into(),
            entrypoint: exe.to_string_lossy().into_owned(),
            args,
        },
    })
}

/// Serves bridge `id` on the host address until the host hangs up.
pub fn run(id: &str, sock: &str, opts: &Options) -> Result<(), String> {
    let p = build(id, opts)?;
    lain_component::serve(sock, p.as_ref()).map_err(|e| format!("{id}: {e}"))
}
