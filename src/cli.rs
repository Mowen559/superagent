use crate::server;
use clap::{Parser, Subcommand};
use std::ffi::OsString;

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
    /// Headless worker for Superagent
    SuperagentWorker {
        /// Base prompt
        #[arg(long)]
        prompt: String,
        /// Resume previous session ID
        #[arg(long)]
        resume_session_id: Option<String>,
        /// Project workspace root
        #[arg(long)]
        project: Option<String>,
        /// Target model name override
        #[arg(long)]
        model: Option<String>,
    },
    /// Start the backend service
    Service {
        /// Port to listen on
        #[arg(default_value_t = server::DEFAULT_PORT)]
        port: u16,
    },
    /// Launch the interactive chat session
    Chat {
        /// Target model name override (e.g. gpt-4o, claude-3-5-sonnet, deepseek-chat)
        #[arg(short, long)]
        model: Option<String>,

        /// Project workspace root directory
        #[arg(short, long)]
        project: Option<String>,

        /// Custom system prompt override
        #[arg(short, long)]
        system: Option<String>,
    },
    /// Ask a single question directly without entering interactive chat
    Ask {
        /// The question or prompt to ask
        prompt: Vec<String>,

        /// Target model name override
        #[arg(short, long)]
        model: Option<String>,

        /// Project workspace root directory
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Quick shortcut to list all available and configured models
    Models,
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
    /// Authentication operations
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
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
    /// List agent session checkpoints
    Sessions {
        /// Maximum number of sessions to list
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Resume an interrupted agent session from its checkpoint
    Resume {
        /// The session ID to resume (or omit to resume the most recent interrupted session)
        session_id: Option<String>,
        /// Target model name override
        #[arg(short, long)]
        model: Option<String>,
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
    Install { agent: String },
    /// Update an agent
    Update { agent: String },
    /// Ingest a document or book into the knowledge base
    Ingest {
        /// Path to the file or directory to ingest
        path: String,

        /// Optional global context summary
        #[arg(short, long)]
        context: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskCommand {
    /// List active tasks
    List,
    /// Watch a specific task
    Watch { task_id: String },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Show all current configuration values and active models
    List,
    /// Set default chat model (e.g. hub config set-model deepseek-chat)
    SetModel { model: String },
    /// Set fast brain model (e.g. hub config set-fast-model deepseek-chat)
    SetFastModel { model: String },
    /// Bind a dedicated LLM model to a specific swarm role (e.g. hub config set-role-model coder deepseek-coder)
    SetRoleModel { role: String, model: String },
    /// Set a configuration value
    Set { key: String, value: String },
    /// Get a configuration value
    Get { key: String },
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
pub enum AuthCommand {
    /// Log in to DeepCloud
    Login,
    /// Check authentication status
    Status,
    /// Log out of DeepCloud
    Logout,
}

#[derive(Subcommand, Debug)]
pub enum SkillCommand {
    /// List installed skills
    List,
    /// Install a skill
    Install { name: String },
    /// Remove a skill
    Remove { name: String },
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// List configured MCP servers
    List,
    /// Install an MCP server
    Install { name: String },
    /// Remove an MCP server
    Remove { name: String },
}

/// Entry point invoked from `main`. Never returns for the hook role (it exits
/// with rtk's process code); returns after the server stops for the service
/// role.
pub fn dispatch() {
    // 强制 Windows DPI Aware，确保截屏获取真实物理像素
    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        type SetProcessDpiAwareness = unsafe extern "system" fn(i32) -> i32;
        if let Ok(lib) = unsafe { libloading::Library::new("shcore.dll") } {
            unsafe {
                if let Ok(func) = lib.get::<SetProcessDpiAwareness>(b"SetProcessDpiAwareness") {
                    let _ = func(2); // PROCESS_PER_MONITOR_DPI_AWARE
                }
            }
        }
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            e.exit();
        }
    };

    match cli.command {
        Some(Commands::SuperagentWorker { prompt, project, model, resume_session_id }) => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                crate::services::brain::worker::run_worker(&prompt, project, model, resume_session_id).await;
            });
            std::process::exit(0);
        }
        Some(Commands::Install) => crate::cli_commands::handle_install(),
        Some(Commands::Service { port }) => run_service(port),
        Some(Commands::Chat { model, project, system }) => {
            crate::repl::run_chat(model, project, system);
        }
        Some(Commands::Ask { prompt, model, project }) => {
            crate::cli_commands::handle_ask(prompt, model, project);
        }
        Some(Commands::Models) => {
            crate::cli_commands::handle_models_shortcut();
        }
        Some(Commands::McpServer) => crate::mcp_server::run_mcp(),
        Some(Commands::Agent { command }) => crate::cli_commands::handle_agent(command),
        Some(Commands::Task { command }) => crate::cli_commands::handle_task(command),
        Some(Commands::Config { command }) => crate::cli_commands::handle_config(command),
        Some(Commands::Auth { command }) => crate::cli_commands::handle_auth(command),
        Some(Commands::Sync) => crate::cli_commands::handle_sync(),
        Some(Commands::Skill { command }) => crate::cli_commands::handle_skill(command),
        Some(Commands::Mcp { command }) => crate::cli_commands::handle_mcp(command),
        Some(Commands::Sessions { limit }) => crate::cli_commands::handle_sessions(limit),
        Some(Commands::Resume { session_id, model }) => crate::cli_commands::handle_resume(session_id, model),
        Some(Commands::External(ext)) => {
            if ext.first().map(|s| s.to_string_lossy()) == Some("mcp".into()) && ext.len() == 1 {
                crate::mcp_server::run_mcp()
            } else {
                run_hook()
            }
        }
        None => {
            let current_exe = std::env::current_exe().unwrap_or_default();
            let install_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".hub")
                .join("bin");
            let cargo_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".cargo")
                .join("bin");

            if current_exe.starts_with(&install_dir) || current_exe.starts_with(&cargo_dir) {
                println!(
                    "DeepCloud Hub is installed. Starting service on default port {}...",
                    crate::server::DEFAULT_PORT
                );
                run_service(crate::server::DEFAULT_PORT);
            } else {
                use dialoguer::Confirm;
                println!("============================================================");
                println!("Welcome to DeepCloud Hub Super Agent!");
                println!("It looks like you are running this executable for the first time.");
                println!("============================================================");

                let install = Confirm::new()
                    .with_prompt("Do you want to automatically install and register hub to your system PATH?")
                    .default(true)
                    .interact()
                    .unwrap_or(false);

                if install {
                    crate::cli_commands::handle_install();
                } else {
                    println!("Installation skipped. You can run 'hub install' manually later.");
                }

                println!("\nPress Enter to exit...");
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
            }
        }
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
            rtk::cli::run();
            std::process::exit(0);
        })
        .expect("failed to spawn hook thread");
    let _ = thread.join().expect("hook thread panicked");
}
