//! Argument dispatch for the `hub` binary.
//!
//! The single `hub` executable plays two roles, selected by its first argument:
//!
//! * `hub service [PORT]` — launch the web backend (defaults to
//!   [`server::DEFAULT_PORT`] when the port is omitted or unparseable).
//! * anything else (including no arguments) — act as the rtk hook: forward the
//!   full argument vector to [`rtk::cli::run`], so Claude's `PreToolUse` hook
//!   (`hub hook claude`) and the rewritten commands (`hub read ...`,
//!   `hub git ...`) resolve to this same binary. No separate `rtk` binary is
//!   needed.
//!
//! Keeping this split here — rather than in `main` — lets `main` stay a trivial
//! entry point and keeps the two roles decoupled from one another.

use crate::server;

/// Subcommand that selects the web-backend role.
const SERVICE_SUBCOMMAND: &str = "service";

/// Entry point invoked from `main`. Never returns for the hook role (it exits
/// with rtk's process code); returns after the server stops for the service
/// role.
pub fn dispatch() {
    match parse_mode(std::env::args().skip(1)) {
        Mode::Service { port } => run_service(port),
        Mode::Chat => crate::repl::run_chat(),
        Mode::Mcp => crate::mcp_server::run_mcp(),
        Mode::Hook => run_hook(),
    }
}

/// The role selected from the command line.
enum Mode {
    /// Run the HTTP backend on `port`.
    Service { port: u16 },
    /// Run the interactive REPL chat.
    Chat,
    /// Run the MCP server over stdio.
    Mcp,
    /// Delegate the whole invocation to rtk's CLI dispatch.
    Hook,
}

/// Decide the role from the argument list (everything after `argv[0]`).
///
/// Only a leading `service` token switches to the web backend; every other
/// shape — including an empty argument list — is treated as a hook call.
fn parse_mode(mut args: impl Iterator<Item = String>) -> Mode {
    match args.next().as_deref() {
        Some(SERVICE_SUBCOMMAND) => {
            let port = args
                .next()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(server::DEFAULT_PORT);
            Mode::Service { port }
        }
        Some("chat") => Mode::Chat,
        Some("mcp") => Mode::Mcp,
        _ => Mode::Hook,
    }
}

/// Web-backend role: spin up a Tokio runtime and serve until shutdown.
fn run_service(port: u16) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    runtime.block_on(server::serve(port));
}

/// Stack size for the rtk hook thread. rtk's own binary reserves 8 MiB via a
/// linker arg in its `build.rs`, because clap parsing the full command graph
/// can overflow the default 1 MiB Windows main-thread stack during startup.
/// A dependency's link arg does not propagate to this binary, so we reproduce
/// the headroom by running the dispatch on a thread with an explicit stack.
const HOOK_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Hook role: run rtk's CLI over the current process arguments and exit with
/// its status code. rtk re-reads `std::env::args_os()` itself, so the hook
/// sees the arguments verbatim (e.g. `hub hook claude`, `hub read foo.rs`).
///
/// Runs on a dedicated large-stack thread (see [`HOOK_STACK_SIZE`]).
fn run_hook() -> ! {
    let code = std::thread::Builder::new()
        .stack_size(HOOK_STACK_SIZE)
        .spawn(rtk::cli::run)
        .expect("failed to spawn rtk hook thread")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(args: &[&str]) -> Mode {
        parse_mode(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn service_without_port_uses_default() {
        assert!(matches!(
            mode(&["service"]),
            Mode::Service { port } if port == server::DEFAULT_PORT
        ));
    }

    #[test]
    fn service_with_port_parses_it() {
        assert!(matches!(mode(&["service", "3000"]), Mode::Service { port: 3000 }));
    }

    #[test]
    fn service_with_bad_port_falls_back_to_default() {
        assert!(matches!(
            mode(&["service", "notaport"]),
            Mode::Service { port } if port == server::DEFAULT_PORT
        ));
    }

    #[test]
    fn no_args_is_hook() {
        assert!(matches!(mode(&[]), Mode::Hook));
    }

    #[test]
    fn hook_claude_is_hook() {
        assert!(matches!(mode(&["hook", "claude"]), Mode::Hook));
    }

    #[test]
    fn read_command_is_hook() {
        assert!(matches!(mode(&["read", "foo.rs"]), Mode::Hook));
    }
}
