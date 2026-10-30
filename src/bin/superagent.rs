//! 大脑 B (Executor): 动态执行变异体
//! 作为子进程被 `hub` 拉起，专职负责执行具体的思维流和任务处理
//! 可以通过修改自身代码，并触发重新编译实现热重载自举

use hub_rs::domains::agent_cognitive::executor::AgentLoop;
use hub_rs::services::brain::subagent_runtime::SubagentRuntime;
use hub_rs::services::brain::tools::fs::{ReadFileTool, WriteFileTool};
use hub_rs::services::brain::tools::harness_acp::HarnessAcpClientTool;
use hub_rs::services::brain::tools::interaction::AskUserTool;
use hub_rs::services::brain::tools::lsp_tool::LspTool;
use hub_rs::services::brain::tools::manage_process::ManageProcessTool;
use hub_rs::services::brain::tools::native_graph::NativeSearchGraphTool;
use hub_rs::services::brain::tools::native_memory::NativeMemoryTool;
use hub_rs::services::brain::tools::reload_plugins::ReloadPluginsTool;
use hub_rs::services::brain::tools::self_evolve::RequestReloadTool;
use hub_rs::services::brain::tools::shell::RunCommandTool;
use hub_rs::services::brain::tools::skill_tool::SkillTool;
use hub_rs::services::brain::tools::subagent::{
    CancelSubagentTool, CheckWorkerStatusTool, InterveneWorkerTool, InvokeSubagentTool,
};
use hub_rs::services::brain::tools::todo::ManageTaskTool;
use hub_rs::services::brain::tools::tool_result::ReadToolResultTool;
use hub_rs::services::brain::tools::web::WebFetchTool;

use hub_rs::services::brain::capabilities::shell::LocalShellProvider;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    eprintln!("Superagent Worker (Brain B) is starting...");

    let app_paths = hub_rs::config::app_paths::AppPaths::new();
    let persona = hub_rs::config::persona_settings::load_persona(&app_paths);
    let persona_section = persona.render_prompt_section();

    let cwd_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let system_prompt = hub_rs::domains::agent_cognitive::prompt_composer::PromptComposer::compose(
        &cwd_path,
        Some(&persona_section),
    );

    let mut engine = AgentLoop::new(&system_prompt);
    hub_rs::services::brain::native_capabilities::register(&mut engine,&cwd_path,None);

    // 初始化 Capability Providers (IoC 容器挂载)
    let shell_provider = Arc::new(LocalShellProvider);
    engine.ctx.provide("shell_service", shell_provider.clone());

    // 注册基础工具
    engine.register_tool(Box::new(ReadFileTool::new(cwd_path.clone())));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::ViewOutlineTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(WriteFileTool::new(cwd_path.clone())));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::ReplaceInFileTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::MultiReplaceTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::ListDirectoryTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::FindByNameTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::GrepSearchTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::fs::AstGrepTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(RunCommandTool::new(
        shell_provider.clone(),
        cwd_path.clone(),
    )));
    engine.register_tool(Box::new(WebFetchTool));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::web::HeadlessBrowserTool,
    ));
    engine.register_tool(Box::new(NativeSearchGraphTool::new(cwd_path.clone())));
    engine.register_tool(Box::new(NativeMemoryTool::new(cwd_path.clone())));
    engine.register_tool(Box::new(ReadToolResultTool::default_location()));

    // Knowledge Corpus tools share one deep runtime so extraction, Manifest,
    // replacement and retrieval invariants cannot drift between callers.
    use hub_rs::services::brain::knowledge_corpus::shared_runtime;
    use hub_rs::services::brain::tools::knowledge::{
        KnowledgeCorpusTool, KnowledgeIngestTool, KnowledgeSearchTool,
    };
    match shared_runtime(&engine.runtime_config) {
        Ok(corpus) => {
            engine.register_tool(Box::new(KnowledgeSearchTool::new(corpus.clone())));
            engine.register_tool(Box::new(KnowledgeIngestTool::new(corpus.clone())));
            engine.register_tool(Box::new(KnowledgeCorpusTool::new(corpus)));
        }
        Err(error) => eprintln!("[KnowledgeCorpus] tools unavailable: {error}"),
    }

    engine.register_tool(Box::new(LspTool));
    engine.register_tool(Box::new(SkillTool::new(std::env::current_dir().unwrap())));
    engine.register_tool(Box::new(HarnessAcpClientTool));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::blackboard_tool::BlackboardTool::new(
            hub_rs::domains::agent_cognitive::agent_blackboard::AgentBlackboard::global(),
            "Supervisor",
        ),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::self_persona_tool::UpdateSelfPersonaTool::new(
            app_paths.clone(),
        ),
    ));

    // 注册子智能体与监工介入工具

    engine.register_tool(Box::new(AskUserTool::new(
        engine.interaction_runtime.clone(),
    )));
    let parent_session_id = engine.session_id().to_string();
    let subagent_runtime = Arc::new(SubagentRuntime::new(
        &parent_session_id,
        engine.steering_queue.clone(),
    ));
    engine
        .ctx
        .provide("subagent_runtime", subagent_runtime.clone());
    engine.register_tool(Box::new(InvokeSubagentTool::new(
        subagent_runtime.clone(),
        &parent_session_id,
    )));
    engine.register_tool(Box::new(CheckWorkerStatusTool::new(
        subagent_runtime.clone(),
        &parent_session_id,
    )));
    engine.register_tool(Box::new(InterveneWorkerTool::new(subagent_runtime.clone())));
    engine.register_tool(Box::new(CancelSubagentTool::new(subagent_runtime)));

    // 注册任务与进程管理工具
    engine.register_tool(Box::new(ManageTaskTool::new(cwd_path.clone())));
    engine.register_tool(Box::new(ManageProcessTool));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::shadow_git_tool::ShadowGitTool,
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::mcp_client::McpClientTool,
    ));

    // 注册热重载插件工具与自举重载工具
    engine.register_tool(Box::new(ReloadPluginsTool::new(engine.get_tools_ref())));
    engine.register_tool(Box::new(RequestReloadTool));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::checkpoint::CreateCheckpointTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::checkpoint::RevertToCheckpointTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::checkpoint::ListCheckpointsTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::impact_radar::ImpactRadarTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::benchmark_tool::GoldenBenchmarkTool,
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::synthesize_tool::SynthesizeTool::new(
            engine.get_tools_ref(),
            cwd_path.clone(),
        ),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::repo_map_tool::RepoMapTool::new(cwd_path.clone()),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::verify_task::VerifyTaskTool::new(
            shell_provider.clone(),
            cwd_path.clone(),
        ),
    ));
    engine.register_tool(Box::new(
        hub_rs::services::brain::tools::code_review::CodeReviewTool::new(cwd_path.clone()),
    ));

    // 自动扫描与装载动态插件工具
    engine.plugin_registry.load_dynamic_plugins();

    // 如果通过命令行传入了测试文件，直接解析并执行；否则进入 IPC 循环
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let first_arg = args[1].as_str();
        if first_arg == "--capabilities"
            || first_arg == "capabilities"
            || first_arg == "-c"
            || first_arg == "--capabilities-json"
        {
            let as_json = first_arg == "--capabilities-json";
            print_capabilities(&engine, as_json);
            return;
        }

        engine.interaction_runtime.use_console();
        if args[1] == "chat" {
            eprintln!(
                "\x1b[36m============================================================\x1b[0m"
            );
            eprintln!("\x1b[36m    SuperAgent (Brain B) Interactive Console - Native Mode\x1b[0m");
            eprintln!(
                "\x1b[36m============================================================\x1b[0m"
            );
            eprintln!("Type 'exit' or 'quit' to leave.\n");
            let mut rl = rustyline::DefaultEditor::new().unwrap();
            loop {
                let readline = rl.readline("\x1b[32m> \x1b[0m");
                match readline {
                    Ok(line) => {
                        let input = line.trim();
                        if input == "exit" || input == "quit" {
                            break;
                        }
                        if input.is_empty() {
                            continue;
                        }
                        let _ = rl.add_history_entry(input);
                        eprintln!("\n\x1b[90m[SuperAgent 正在思考并行动...]\x1b[0m\n");
                        match engine.run(input).await {
                            Ok(res) => println!("\x1b[32m[Agent]\x1b[0m {}", res),
                            Err(e) => eprintln!("\x1b[31m[Error]\x1b[0m {}", e),
                        }
                        println!();
                    }
                    Err(_) => break,
                }
            }
            return;
        }

        eprintln!("[Brain B] Loading prompt from file: {}", args[1]);
        let file_content = std::fs::read_to_string(&args[1]).expect("Failed to read prompt file");
        let prompt_text = match serde_json::from_str::<serde_json::Value>(&file_content) {
            Ok(parsed) => parsed
                .get("payload")
                .and_then(|v| v.as_str())
                .unwrap_or(&file_content)
                .to_string(),
            Err(_) => file_content,
        };
        let batch_prompt = format!(
            "[Execution Directive: Single-Pass Batch Mode]\n\
            Execute the following instructions thoroughly, verify the results, and deliver your comprehensive final summary directly without asking for follow-up guidance.\n\n\
            {}",
            prompt_text
        );
        match engine.run(&batch_prompt).await {
            Ok(res) => eprintln!("[Brain B] Result: {}", res),
            Err(e) => eprintln!("[Brain B] Error: {}", e),
        }
    } else {
        // 启动 IPC 通信循环 (JSON-RPC) 监听来自 Hub 的 prompt，
        // 并将 prompt 交给 engine.run(prompt).await 执行。
        hub_rs::services::brain::ipc::start_worker_loop(engine).await;
    }

    eprintln!("Worker loop exiting.");
}

fn print_capabilities(engine: &AgentLoop, as_json: bool) {
    let tools_guard = engine.get_tools_ref();
    let tools_map = tools_guard.lock().unwrap();

    let mut tools_list: Vec<serde_json::Value> = Vec::new();
    for (name, tool) in tools_map.iter() {
        tools_list.push(serde_json::json!({
            "name": name,
            "description": tool.description(),
            "parameters": tool.parameters_schema()
        }));
    }
    tools_list.sort_by(|a, b| {
        a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
    });

    if as_json {
        let output = serde_json::json!({
            "agent": "MoZhi SuperAgent (Brain B)",
            "dual_brain": {
                "fast_brain": "Active (Token Compaction & Low Latency Turns)",
                "slow_brain": "Active (Deep Reasoning & Escalation on Failure)",
                "mid_flight_preemption": true
            },
            "tools_count": tools_list.len(),
            "tools": tools_list,
            "integrations": {
                "ruflo_hive_mind": "Compatible via MCP & Subagent runtime",
                "hermes_distillation": "Active",
                "process_manager": "Active"
            }
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    println!("\x1b[36m================================================================================\x1b[0m");
    println!("\x1b[1;36m       🚀 MoZhi SuperAgent Capability & Plugin Matrix (能力全景看板)\x1b[0m");
    println!("\x1b[36m================================================================================\x1b[0m");
    println!("\x1b[1;33m[1. Cognitive Architecture / 认知双脑架构]\x1b[0m");
    println!("  • Dual-Brain Engine: Fast Brain (Low latency / Triage) + Slow Brain (Deep Reasoning)");
    println!("  • Context Compression: RTK Compactor + Mid-flight Preemption Guard");
    println!("  • Session Event Ledger: Strong-typed event sourcing & auto-repair");
    println!("  • Composable Prompt: Modular XML with zero-compile workspace rule hot-reload");
    println!();
    println!("\x1b[1;32m[2. Registered Tool Arsenal / 工具军火库] (Total: {} registered tools)\x1b[0m", tools_list.len());

    let categories = [
        ("📁 File System & AST", vec!["read_file", "write_file", "replace_in_file", "multi_replace_file_content", "view_outline", "ast_grep"]),
        ("⚡ Terminal & Process Guard", vec!["run_command", "manage_process", "shadow_git"]),
        ("🐝 Subagent Swarm & Hive-Mind", vec!["invoke_subagent", "check_worker_status", "intervene_worker", "cancel_subagent"]),
        ("🔍 Code Intelligence & LSP", vec!["search_graph", "lsp_client", "repo_map_tool", "impact_radar", "code_review"]),
        ("🧠 Cognitive & Self-Evolution", vec!["manage_task", "ask_user", "super_memory", "verify_task", "run_benchmark", "synthesize_tool", "request_reload"]),
        ("🧩 Dynamic Plugins & Extensions", vec!["reload_plugins", "mcp_client"]),
    ];

    let mut categorized_names = std::collections::HashSet::new();
    for (cat_name, tool_names) in &categories {
        println!("  \x1b[1m{}\x1b[0m:", cat_name);
        for tname in tool_names {
            if let Some(tool) = tools_map.get(*tname) {
                categorized_names.insert((*tname).to_string());
                println!("    • \x1b[32m{:26}\x1b[0m - {}", tname, tool.description());
            }
        }
    }

    let remaining: Vec<_> = tools_map.keys().filter(|k| !categorized_names.contains(*k)).collect();
    if !remaining.is_empty() {
        println!("  \x1b[1m🔌 Dynamic Plugins & MCP Tools (动态扩展)\x1b[0m:");
        for tname in remaining {
            if let Some(tool) = tools_map.get(tname) {
                println!("    • \x1b[35m{:26}\x1b[0m - {}", tname, tool.description());
            }
        }
    }

    println!();
    println!("\x1b[1;35m[3. External Integration & Swarm Readiness / 蜂群与外部协同]\x1b[0m");
    println!("  • RuFlo Hive-Mind / Swarm Workflow: Fully compatible (via MCP & Subagent runtime)");
    println!("  • Rule Hot-Reload: Automatically loads .superagentrules / AGENTS.md / .clinerules");
    println!("  • Shell Guard: Windows PowerShell alias auto-sanitization enabled");
    println!("\x1b[36m================================================================================\x1b[0m");
}

