//! Launch BYOND worlds with the debug library and connect to running worlds.
//! Used by the CLI, debug adapter, tests and `debug_client`.

pub mod byond;
pub mod game;
#[cfg(windows)]
mod inject;
pub mod local;

// Share protocol types as source because the cdylib cannot export Rust types.
#[allow(dead_code)]
#[path = "../../debug_server/src/server_types.rs"]
pub mod server_types;

// Read a world's resource table as the debug server does.
#[path = "../../debug_server/src/dmb_tables.rs"]
pub mod dmb_tables;

// Share macro expansion logic with the debug server.
#[allow(dead_code)]
#[path = "../../debug_server/src/macros.rs"]
pub mod macros;
