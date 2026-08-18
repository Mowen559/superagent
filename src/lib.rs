#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_assignments)]
//! `hub` binary entry point.
//!
//! A single executable with two roles selected by its first argument (see
//! [`cli`]): `hub service [PORT]` runs the web backend; any other invocation
//! acts as the rtk hook. `main` stays trivial — it only wires up the module
//! tree and hands off to [`cli::dispatch`].

pub mod cli;
pub mod config;
pub mod models;
pub mod routes;
pub mod reverse_engineering;
pub mod server;
pub mod services;
pub mod repl;
pub mod mcp_server;
pub mod cli_commands;

// Re-exported at the crate root so route modules keep referring to
// `crate::AppState` after the state moved into `server`.
pub use server::AppState;
