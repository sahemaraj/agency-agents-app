//! Tauri command surface. One sub-module per cluster of related commands.
//!
//! `lib.rs` re-exports these via `commands::*` and registers them in
//! `tauri::generate_handler![]`.

pub mod doctor;
pub mod github;
pub mod mcp_clients;
pub mod settings;
pub mod updater;

// Re-export every command in flat form so `invoke_handler!` can take them.
pub use doctor::*;
pub use github::*;
pub use mcp_clients::*;
pub use settings::*;
pub use updater::*;
