//! One module per screen; each is `impl LainApp` with its state struct.

pub mod acquire;
pub mod connect;
pub mod downloads;
pub mod extensions;
pub mod home;
pub mod item;
pub mod library;
pub mod links;
pub mod player;
pub mod reader;
pub mod search;
pub mod settings;
pub mod transcode;

/// A library's name as shown: a locked private library comes without one
/// (D-293), so it reads under one neutral label instead of a blank.
pub fn library_name(l: &lain_client::types::Library) -> String {
    if l.name.is_empty() && l.locked == Some(true) { "Private library".into() } else { l.name.clone() }
}
