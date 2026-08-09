use clap::{Parser, Subcommand};
use std::ffi::OsString;
use crate::server;

/// Global CLI configuration and arguments
#[derive(Parser, Debug)]
#[command(name = "hub", about = "DeepCloud Hub Super Agent CLI", version, author)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Install this application globally to the system PATH
    Install,
    /// Start the backend service
    Service {
        /// Port to listen on
        #[arg(default_value_t = server::DEFAULT_PORT)]
        port: u16,
    },
    /// Launch the interactive chat session
    Chat,
    /// Run the MCP server
    McpServer,
    /// Agent operations
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Task operations
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Configuration operations
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Sync agents operations
    Sync,
    /// Skill operations
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// MCP operations
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Run an agent task
    Run {
        /// The agent backend to use (e.g., claude, codex, qwen)
        #[arg(short, long, default_value = "claude")]
        agent: String,
        
        /// Project directory to act on
        #[arg(long)]
        project: Option<String>,
        
        /// Model string to use
        #[arg(long)]
        model: Option<String>,
        
        /// Task description prompt
        prompt: Vec<String>,
    },
    /// List available agents
    List,
    /// Install an agent
    Install {
        agent: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskCommand {
    /// List active tasks
    List,
    /// Watch a specific task
    Watch {
        task_id: String,
    }
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Set a configuration value
    Set {
        key: String,
        value: String,
    },
    /// Get a configuration value
    Get {
        key: String,
    },
    /// Configure the model provider and authentication
    Model,
    /// List available models from the provider
    Models,
    /// Configure routing strategy for load balancing
    Routing,
    /// Toggle enable/disable status for a specific LLM account configuration
    Toggle {
        /// Account ID to toggle
        account_id: String,
        /// Optional boolean state (true/false). If omitted, toggles current state.
        state: Option<bool>,
    },
    /// Remove a specific LLM account configuration
    Remove {
        /// Account ID to remove
        account_id: String,
    },
    /// Show current quota and usage for configured models
    Quota,
}

#[derive(Subcommand, Debug)]
pub enum SkillCommand {
    /// List installed skills
    List,
    /// Install a skill
    Install {
        name: String,
    },
    /// Remove a skill
    Remove {
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// List configured MCP servers
    List,
    /// Install an MCP server
    Install {
        name: String,
    },
    /// Remove an MCP server
    Remove {
        name: String,
    },
}

/// Entry point invoked from `main`. Never returns for the hook role (it exits
/// with rtk's process code); returns after the server stops for the service
/// role.
pub fn dispatch() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            e.exit();
        }
    };

    match cli.command {
        Some(Commands::Install) => crate::cli_commands::handle_install(),
        Some(Commands::Service { port }) => run_service(port),
        Some(Commands::Chat) => crate::repl::run_chat(),
        Some(Commands::McpServer) => crate::mcp_server::run_mcp(),
        Some(Commands::Agent { command }) => crate::cli_commands::handle_agent(command),
        Some(Commands::Task { command }) => crate::cli_commands::handle_task(command),
        Some(Commands::Config { command }) => crate::cli_commands::handle_config(command),
        Some(Commands::Sync) => crate::cli_commands::handle_sync(),
        Some(Commands::Skill { command }) => crate::cli_commands::handle_skill(command),
        Some(Commands::Mcp { command }) => crate::cli_commands::handle_mcp(command),
        Some(Commands::External(ext)) => {
            if ext.first().map(|s| s.to_string_lossy()) == Some("mcp".into()) && ext.len() == 1 {
                 crate::mcp_server::run_mcp()
            } else {
                 run_hook()
            }
        },
        None => run_hook(),
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

const HOOK_STACK_SIZE: usize = 8 * 1024 * 1024;

fn run_hook() {
    let thread = std::thread::Builder::new()
        .name("rtk_hook".to_string())
        .stack_size(HOOK_STACK_SIZE)
        .spawn(move || {
            std::process::exit(rtk::cli::run());
        })
        .expect("failed to spawn hook thread");
    let _ = thread.join().expect("hook thread panicked");
}
