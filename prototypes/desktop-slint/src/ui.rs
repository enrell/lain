//! Compiles the UI at run time with slint-interpreter. The app's own files
//! are embedded; `card.slint` comes from the skin file when one is given.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use slint_interpreter::{Compiler, ComponentDefinition};

const APP: &str = include_str!("../ui/app.slint");
const THEME: &str = include_str!("../ui/theme.slint");
const CARD: &str = include_str!("../ui/card.slint");

/// Easing curves a theme may name. Slint's own names pass through; the
/// rest map to the closest Slint curve.
pub const CURVES: &[(&str, &str)] = &[
    ("linear", "linear"),
    ("ease", "ease"),
    ("ease-in", "ease-in"),
    ("ease-out", "ease-out"),
    ("ease-in-out", "ease-in-out"),
    ("ease-in-quad", "ease-in-quad"),
    ("ease-out-quad", "ease-out-quad"),
    ("ease-in-out-quad", "ease-in-out-quad"),
    ("ease-in-cubic", "cubic-bezier(0.32, 0, 0.67, 0)"),
    ("ease-out-cubic", "cubic-bezier(0.33, 1, 0.68, 1)"),
    ("ease-in-out-cubic", "cubic-bezier(0.65, 0, 0.35, 1)"),
    ("ease-in-quart", "ease-in-quart"),
    ("ease-out-quart", "ease-out-quart"),
    ("ease-in-out-quart", "ease-in-out-quart"),
    ("ease-out-quint", "ease-out-quint"),
    ("ease-out-expo", "ease-out-expo"),
    ("ease-in-out-expo", "ease-in-out-expo"),
    ("ease-out-back", "ease-out-back"),
    ("ease-out-circ", "ease-out-circ"),
    ("ease-out-elastic", "ease-out-elastic"),
    ("ease-out-bounce", "ease-out-bounce"),
    // Slint 1.18 has a real spring curve; 0.3 gives a visible overshoot
    ("spring", "spring(0.3)"),
];

/// The `Theme.curve` index for curve `name`; a theme `cubic-bezier(...)`
/// is the extra slot after the named ones (startup only).
pub fn curve_index(name: &str) -> Option<usize> {
    if let Some(i) = CURVES.iter().position(|(n, _)| *n == name) {
        return Some(i);
    }
    name.starts_with("cubic-bezier(").then_some(CURVES.len())
}

fn curve_selector(custom: Option<&str>) -> String {
    let mut e = custom.unwrap_or("ease-out").to_string();
    for (i, (_, expr)) in CURVES.iter().enumerate().rev() {
        e = format!("curve == {i} ? {expr} : ({e})");
    }
    e
}

pub struct Built {
    pub def: ComponentDefinition,
    pub compile_ms: f64,
    pub skin: String,
}

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// `fonts`: font files to register (imported into the app source).
/// `custom_curve`: a theme `cubic-bezier(...)` to make available.
pub fn build(skin: Option<&Path>, fonts: &[PathBuf], custom_curve: Option<&str>) -> Result<Built, String> {
    let t = std::time::Instant::now();
    // LAIN_UI_DIR: read the app's own .slint files from disk (development)
    let dev = std::env::var_os("LAIN_UI_DIR").map(PathBuf::from);
    let read = |name: &str, embedded: &str| -> String {
        dev.as_ref().and_then(|d| std::fs::read_to_string(d.join(name)).ok()).unwrap_or_else(|| embedded.to_string())
    };
    let skin_src = match skin {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("skin {}: {e}", p.display()))?,
        None => read("card.slint", CARD),
    };
    let skin_name = skin
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().trim_start_matches("card-").to_string())
        .unwrap_or_else(|| "poster (built-in)".into());

    let mut src = String::new();
    for f in fonts {
        src.push_str(&format!("import \"{}\";\n", f.display().to_string().replace('\\', "/")));
    }
    src.push_str(&read("app.slint", APP));

    let mut files: HashMap<&'static str, String> = HashMap::new();
    files.insert("theme.slint", read("theme.slint", THEME).replace("/*@curves*/ ease-out", &curve_selector(custom_curve)));
    files.insert("card.slint", skin_src);
    let mut compiler = Compiler::default();
    compiler.set_style("fluent".into());
    compiler.set_file_loader(move |path: &Path| {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let hit = if path.starts_with("lain-ui") { files.get(name).cloned() } else { None };
        Box::pin(async move { hit.map(Ok) }) as Pin<Box<dyn Future<Output = Option<std::io::Result<String>>>>>
    });
    let result = block_on(compiler.build_from_source(src, PathBuf::from("lain-ui/app.slint")));
    let mut errors = Vec::new();
    for d in result.diagnostics() {
        if d.level() == slint_interpreter::DiagnosticLevel::Error {
            errors.push(d.to_string());
        } else {
            eprintln!("slint: {d}");
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let def = result.component("AppWindow").ok_or("AppWindow not found")?;
    Ok(Built { def, compile_ms: t.elapsed().as_secs_f64() * 1000.0, skin: skin_name })
}
