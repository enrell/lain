//! The lain Rust server (D-171), milestone P1a: the gateway for the
//! core routes the clients use, `GET /api/modules` (D-154), `GET
//! /api/events` (D-169), service mode (D-164) and the read-only import
//! of a Go data dir. The binary is `lain-rs` until cutover.

pub mod app;
pub mod catalog;
pub mod contracts;
pub mod datadir;
pub mod events;
pub mod gateway;
pub mod import;
pub mod libraries;
pub mod logging;
pub mod privacy;
pub mod roots;
pub mod service;

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
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            // Event streams never end on their own: close them so the
            // graceful shutdown can finish.
            app2.shutdown();
        })
        .await
}
