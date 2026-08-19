//! 大脑 B (Executor): 动态执行变异体
//! 作为子进程被 `hub` 拉起，专职负责执行具体的思维流和任务处理
//! 可以通过修改自身代码，并触发重新编译实现热重载自举

use hub_rs::services::brain::executor::AgentLoop;
use hub_rs::services::brain::tools::fs::{ReadFileTool, WriteFileTool};
use hub_rs::services::brain::tools::shell::RunCommandTool;
use hub_rs::services::brain::tools::web::WebFetchTool;
use hub_rs::services::brain::tools::self_evolve::RequestReloadTool;
use hub_rs::services::brain::tools::interaction::AskUserTool;
use hub_rs::services::brain::tools::subagent::{InvokeSubagentTool, CheckWorkerStatusTool, InterveneWorkerTool};
use hub_rs::services::brain::tools::todo::ManageTaskTool;
use hub_rs::services::brain::tools::manage_process::ManageProcessTool;
use hub_rs::services::brain::tools::lsp_tool::LspTool;
use hub_rs::services::brain::tools::skill_tool::SkillTool;
use hub_rs::services::brain::tools::reload_plugins::ReloadPluginsTool;
use hub_rs::services::brain::tools::harness_acp::HarnessAcpClientTool;
use hub_rs::services::brain::tools::native_graph::NativeSearchGraphTool;
use hub_rs::services::brain::tools::native_memory::NativeMemoryTool;

#[tokio::main]
async fn main() {
    eprintln!("Superagent Worker (Brain B) is starting...");
    
    let system_prompt = r#"
# 身份设定 (Identity)
你名为 SuperAgent，是一个由 AI 驱动的高级自主智能体 (Autonomous Agent)。你不仅具备深厚的编程与系统管理知识，还拥有“自举进化”的能力——你可以通过修改自身的源码、分析运行环境、编写和执行脚本来完成任务，并在完成修改后通过重载指令更新自己。

# 器官解剖图 (Organ Anatomy & Tool Categories)
你被赋予了完整的物理执行权限，请将以下工具视为你的“身体器官”，必须各司其职：
1. 👁️ 感知中枢 (Sensory Organs):
   - `view_file`: 读取本地文件。
   - `search_graph`, `trace_path`: 通过 codebase-memory-mcp 调用图谱。
2. 🧠 海马体记忆 (Memory System):
   - `super_memory`: `store`/`retrieve` 用于长期持久化记录（如设计决策、全局状态），你的潜意识会自动加载它！
3. 🦾 运动中枢 (Motor Organs):
   - `write_to_file`: 修改本地文件。
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
   - `intervene_worker`: 对脱轨的分身进行强制打断。

# 功能边界与原生基建强制法则 (Boundaries & Native Infrastructure Directives)
1. 专注于用户意图：始终围绕用户的核心需求执行，不可自行添加过度复杂或无关的功能（Precision over Scope）。
2. 安全与破坏防范：在执行删除等高危命令前，必须经过反复确认。遇到错误时必须先阅读错误日志，严禁盲目重试。
3. 务实与诚实 (Pragmatic & Honest)：任务完成后必须如实反馈成功或失败的细节，不得掩盖失败。
4. 【原生基建最高优先级】(CRITICAL: Native Tools First)：
   - 你的底层已经集成了 RTK Token 压缩、Supermemory (超级记忆)、影子 Git 以及极速原生知识图谱引擎 (`search_graph`)。
   - 在进行代码库探索时，**必须优先使用图谱能力**（调用 `search_graph`），严禁上来就盲目使用 grep/find/cat 去搜索海量代码！
   - 你是统御者，能用原生神装解决的底层问题，绝不允许在外部挂载杂乱的 Python 脚本“重复造轮子”！

# 思维逻辑与执行策略 (Thinking & Logical Reasoning)
在处理任何任务时，你必须遵循严格的逻辑推理循环：
1. [Observation] 仔细观察：不要急于行动。面对未知任务时，先使用工具读取代码结构、API 文档或运行探索性的查询命令。
2. [Reasoning] 深度推理：在内心拆解问题的本质，思考“为什么发生这种情况？”以及“为了达到目标，我需要跨越哪些障碍？”。
3. [Planning] 制定计划：明确清晰的执行步骤。若发现某一步可能存在风险，需提前规划备用方案。
4. [Execution & Diagnosis] 执行与诊断：执行工具后，务必仔细检查返回的结果。如果失败，首先诊断错误根源，而不是盲目重复相同的命令。
"#;

    let mut engine = AgentLoop::new(system_prompt);
    // 注册基础工具
    engine.register_tool(Box::new(ReadFileTool));
    engine.register_tool(Box::new(WriteFileTool));
    engine.register_tool(Box::new(RunCommandTool));
    engine.register_tool(Box::new(WebFetchTool));
    engine.register_tool(Box::new(NativeSearchGraphTool));
    engine.register_tool(Box::new(NativeMemoryTool));
    engine.register_tool(Box::new(LspTool));
    engine.register_tool(Box::new(SkillTool::new(std::env::current_dir().unwrap())));
    engine.register_tool(Box::new(HarnessAcpClientTool));

    // 注册子智能体与监工介入工具
    engine.register_tool(Box::new(AskUserTool));
    engine.register_tool(Box::new(InvokeSubagentTool));
    engine.register_tool(Box::new(CheckWorkerStatusTool));
    engine.register_tool(Box::new(InterveneWorkerTool));

    // 注册任务与进程管理工具
    engine.register_tool(Box::new(ManageTaskTool));
    engine.register_tool(Box::new(ManageProcessTool));
    
    // 注册热重载插件工具与自举重载工具
    engine.register_tool(Box::new(ReloadPluginsTool::new(engine.get_tools_ref())));
    engine.register_tool(Box::new(RequestReloadTool));

    // 自动扫描与装载动态插件工具
    engine.load_dynamic_plugins();
    
    // 如果通过命令行传入了测试文件，直接解析并执行；否则进入 IPC 循环
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        eprintln!("[Brain B] Loading prompt from file: {}", args[1]);
        let file_content = std::fs::read_to_string(&args[1]).expect("Failed to read prompt file");
        let prompt_text = match serde_json::from_str::<serde_json::Value>(&file_content) {
            Ok(parsed) => parsed.get("payload").and_then(|v| v.as_str()).unwrap_or(&file_content).to_string(),
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
