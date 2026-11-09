//! The lain Rust server (D-171), milestone P1a: the gateway for the
//! core routes the clients use, `GET /api/modules` (D-154), `GET
//! /api/events` (D-169), service mode (D-164), private libraries (D-156)
//! and portable roots (D-157), plus the ported modules: core
//! administration (theme, backup, integrations, avatars; `lain-admin`),
//! lists (`lists.rs` over `lain-lists`), social (`social.rs` over
//! `lain-social`),
//! read (`lain-read`) and watch (D-190: `watch.rs`, byte serving in
//! `serve.rs`). It starts from an empty data dir; nothing is imported
//! from the Go server (D-177). The binary is `lain-rs`.

pub mod app;
pub mod catalog;
pub mod contracts;
pub mod datadir;
pub mod events;
pub mod files;
pub mod gateway;
pub mod libraries;
pub mod lists;
pub mod logging;
pub mod population;
pub mod privacy;
pub mod roots;
pub mod serve;
pub mod service;
pub mod social;
pub mod watch;

use std::net::SocketAddr;
use std::sync::Arc;

/// The version string `/health` reports.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-rs");

/// The catalog of a data dir as `lain.catalog-export@3` (D-175), paths
/// resolved with that dir's `roots.json`. The server must not be
/// running (the store is locked while it is).
pub fn export(data_dir: &std::path::Path) -> Result<contracts::ExportDoc, String> {
    let store = Arc::new(lain_store::Store::open(data_dir).map_err(|e| e.to_string())?);
    let roots = roots::Roots::load(data_dir)?;
    catalog::Catalog::with_roots(store, Arc::new(std::sync::RwLock::new(roots))).export().map_err(|e| e.to_string())
}

/// Replaces the catalog of a data dir with a `lain.catalog-export@3`
/// document and returns the item count. The server must not be running.
pub fn import(data_dir: &std::path::Path, doc: &contracts::ExportDoc) -> Result<usize, String> {
    let store = Arc::new(lain_store::Store::open(data_dir).map_err(|e| e.to_string())?);
    catalog::Catalog::new(store).import(doc).map_err(|e| e.to_string())
}

/// Serves `app` on `listener` until `shutdown` resolves.
pub async fn serve(
    app: Arc<app::App>,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let router = gateway::router(app.clone());
    let addr: Option<SocketAddr> = listener.local_addr().ok();
    tracing::info!(addr = %addr.map(|a| a.to_string()).unwrap_or_default(), data_dir = %app.data_dir.display(), "lain-rs listening");
    let app2 = app.clone();
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown.await;
            // Event streams never end on their own: close them so the
            // graceful shutdown can finish.
            app2.shutdown();
        })
        .await
}
