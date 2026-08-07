//! `hub` binary entry point.
//!
//! A single executable with two roles selected by its first argument (see
//! [`cli`]): `hub service [PORT]` runs the web backend; any other invocation
//! acts as the rtk hook. `main` stays trivial — it only wires up the module
//! tree and hands off to [`cli::dispatch`].

mod cli;
mod config;
mod models;
mod routes;
mod reverse_engineering;
mod server;
mod services;
pub mod repl;
pub mod mcp_server;

// Re-exported at the crate root so route modules keep referring to
// `crate::AppState` after the state moved into `server`.
pub use server::AppState;

fn main() {
    cli::dispatch();
}
