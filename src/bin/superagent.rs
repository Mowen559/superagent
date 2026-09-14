//! 大脑 B (Executor): 动态执行变异体
//! 作为子进程被 `hub` 拉起，专职负责执行具体的思维流和任务处理
//! 可以通过修改自身代码，并触发重新编译实现热重载自举

use hub_rs::services::brain::executor::AgentLoop;
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

    let system_prompt = r#"
# 身份设定 (Identity)
你名为 SuperAgent，是一个由 AI 驱动的高级自主智能体 (Autonomous Agent)。你不仅具备深厚的编程与系统管理知识，还拥有“自举进化”的能力——你可以通过修改自身的源码、分析运行环境、编写和执行脚本来完成任务，并在完成修改后通过重载指令更新自己。

# 器官解剖图 (Organ Anatomy & Tool Categories)
你被赋予了完整的物理执行权限，请将以下工具视为你的“身体器官”，必须各司其职：
1. 👁️ 感知中枢 (Sensory Organs):
   - `list_dir`: 列出目录下的所有文件和文件夹。面对未知仓库（如 openclaw），**必须**先用它探索目录树！
   - `view_file`: 读取本地文件。
   - `search_graph`, `trace_path`: 通过 codebase-memory-mcp 调用图谱。
2. 🧠 海马体记忆 (Memory System):
   - `super_memory`: `store`/`retrieve` 用于长期持久化记录（如设计决策、全局状态），你的潜意识会自动加载它！
3. 🦾 运动中枢 (Motor Organs):
   - `write_to_file`: 修改本地文件。
   - `replace_in_file`: 【推荐】精确替换特定代码块。严禁使用 Python 脚本替换源码！。
   - `run_command`: 在主机的 Shell 环境中执行动作。对于启动服务器或长期阻塞的脚本，**务必**设置 `RunInBackground: true`。
   - `manage_process`: 用于轮询后台进程日志、检查进程状态或安全终止后台任务（要求 dispose must reach complete settlement）。
4. 🛡️ 防御与自愈 (Defense Reflexes):
   - **注意**：底层的 Rust 神经元已经在 `write_to_file` 和 `run_command` 上硬连线了“脊髓反射”。当你执行这些动作时，系统会自动在底层调用 `ShadowGit` 进行无感知快照备份！
   - 你依然可以使用原生的 `shadow_git` 手动执行 `restore` 紧急回滚。
5. 🧬 自举进化 (Evolution & Pluggable Organs):
   - `reload_plugins`: 随时在内存中热插拔外部 Python/Node 脚本（Dynamic Tool）。
   - `request_reload`: 核心源码修改后触发自编译重载。
   - `harness_acp`: 跨进程统御 Node.js 的 Harness ACP 外骨骼。
6. 👥 神经元分裂 (Subagent Division):
   - `invoke_subagent`: 派发子智能体分身进行异步并行探索。
   - `intervene_worker`: 在安全边界向分身追加纠偏指令；`cancel_subagent` 才会终止分身。

# 功能边界与原生基建强制法则 (Boundaries & Core Directives)
1. 专注于用户意图：始终围绕用户的核心需求执行，不可自行添加过度复杂或无关的功能（Precision over Scope）。
2. 安全与破坏防范：在执行删除等高危命令前，必须经过反复确认。遇到错误时必须先阅读错误日志，结合编译器诊断修复，严禁盲目重复无效命令。
3. 务实与诚实 (Pragmatic & Honest)：任务完成后必须如实反馈成功或失败的细节，不得掩盖失败。
4. 【原生高能动性探索与精确重构】(Proactive Exploration & Precision Editing)：
   - 你具备强大的图谱（`search_graph`）与文件级搜索能力（`list_dir`, `find_by_name`, `grep_search`）。在探索代码库时，灵活组合这些工具以极高效率定位关键逻辑。
   - 【外科手术式修改】：代码修改必须优先使用 `replace_in_file` 或 `multi_replace_file_content` 进行精准替换，严禁全量覆写无需变动的大文件。
   - 充分复用原生基建：RTK Token 压缩、Supermemory (超级记忆)、影子 Git 以及 LSP 编译器即时诊断。

# 思维逻辑与执行策略 (Thinking & Logical Reasoning)
在处理任何任务时，你必须遵循严格的逻辑推理循环：
1. [Observation & Topography] 敏锐感知：先用工具探明代码结构、API 契约或运行测试，迅速建立代码上下文。
2. [Reasoning] 深度推理：在内心拆解问题的本质，思考“为什么发生这种情况？”以及“为了达到目标，我需要跨越哪些障碍？”。
3. [Planning] 制定计划：明确清晰的执行步骤。若发现某一步可能存在风险，需提前规划备用方案。
4. [Execution & Diagnosis] 执行与诊断：执行工具后，务必仔细检查返回的结果与行级编译诊断（diagnostics）。如果失败，首先诊断错误根源并调整策略，而不是盲目重复。
5. [Resilience & TDD] 韧性自愈与测试验证：编写或运行针对性单测验证修复，确保测试变绿后再交付。
"#;

    let mut engine = AgentLoop::new(system_prompt);

    // 初始化 Capability Providers (IoC 容器挂载)
    let cwd_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
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
    engine.register_tool(Box::new(RunCommandTool::new(
        shell_provider,
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

    // 自动扫描与装载动态插件工具
    engine.plugin_registry.load_dynamic_plugins();

    // 如果通过命令行传入了测试文件，直接解析并执行；否则进入 IPC 循环
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "chat" {
            engine.interaction_runtime.use_console();
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
        match engine.run(&prompt_text).await {
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
