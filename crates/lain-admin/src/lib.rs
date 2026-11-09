//! Core administration (D-155 core module): the web UI theme
//! (`lain.ui.theme@1`), backups (`lain.backup.create@1`), the operator's
//! integration settings and uploaded avatars. The gateway keeps thin
//! handlers over these.

pub mod avatars;
pub mod backup;
pub mod integrations;
pub mod theme;
