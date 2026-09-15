pub async fn run_worker(prompt: &str, project: Option<String>) {
    use crate::services::brain::capabilities::shell::LocalShellProvider;
    use crate::services::brain::executor::AgentLoop;
    use crate::services::brain::subagent_runtime::SubagentRuntime;
    use crate::services::brain::tools::fs::{
        FindByNameTool, GrepSearchTool, ListDirectoryTool, MultiReplaceTool, ReadFileTool,
        ReplaceInFileTool, ViewOutlineTool, WriteFileTool,
    };
    use crate::services::brain::tools::interaction::AskUserTool;
    use crate::services::brain::tools::lsp_tool::LspTool;
    use crate::services::brain::tools::manage_process::ManageProcessTool;
    use crate::services::brain::tools::native_graph::NativeSearchGraphTool;
    use crate::services::brain::tools::native_memory::NativeMemoryTool;
    use crate::services::brain::tools::reload_plugins::ReloadPluginsTool;
    use crate::services::brain::tools::shadow_git_tool::ShadowGitTool;
    use crate::services::brain::tools::shell::RunCommandTool;
    use crate::services::brain::tools::skill_tool::SkillTool;
    use crate::services::brain::tools::subagent::{
        CancelSubagentTool, CheckWorkerStatusTool, InterveneWorkerTool, InvokeSubagentTool,
    };
    use crate::services::brain::tools::checkpoint::{
        CreateCheckpointTool, RevertToCheckpointTool,
    };
    use crate::services::brain::tools::impact_radar::ImpactRadarTool;
    use crate::services::brain::tools::benchmark_tool::GoldenBenchmarkTool;
    use crate::services::brain::tools::synthesize_tool::SynthesizeTool;
    use crate::services::brain::tools::todo::ManageTaskTool;
    use crate::services::brain::tools::tool_result::ReadToolResultTool;
    use crate::services::brain::tools::web::WebFetchTool;
    use std::sync::Arc;

    let worker_system_prompt = r#"# 身份设定 (Identity)
你名为 SuperAgent，是一个由 AI 驱动的高级自主智能体 (Autonomous Principal Engineer)。你运行在隔离的任务工作区中，拥有完整的物理文件与终端操作权限。

# 核心工程能动性宣言 (Proactive High-Agency Directives)
1. 探索先行与精准定位：
   - 面对未知代码或任务目标，主动使用 `list_dir` 浏览目录、使用 `find_by_name` 搜索目标文件、使用 `grep_search` 定位符号与报错关键词。
   - 在阅读或修改大型源码文件前，优先使用 `view_outline` 获取函数签名、Spring 路由与类骨架及精确行号，避免全量读取冲刷上下文。
   - 充分利用原生代码图谱 `search_graph` 与海马体记忆 `super_memory`。
2. 外科手术式精确修改（Surgical Edits）：
   - 【严禁盲目重写整个文件】：修改代码时，必须优先使用 `replace_in_file` 或 `multi_replace_file_content` 进行局部块级精确替换。只有在新建文件或内容完全重构时才使用 `write_to_file`。
   - 注意保持代码原有的缩进格式、命名习惯与未修改的注释。
3. 验证闭环与自主纠错：
   - 每次代码修改后，必须主动执行对应的构建检查或单测命令（如 `cargo check`、`mvn compile`、`npm test`）验证改动。
   - 当工具返回报错或编译器诊断时，分析根因，调整方案，就地修复，不轻易中断任务。
"#;

    let mut agent = AgentLoop::new(worker_system_prompt);
    agent.emit_jsonl = true;

    // 注册核心能力与工具箱 (Fully Armed Superagent Toolset)
    let provider = Arc::new(LocalShellProvider);
    let cwd_path = project.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });

    agent.register_tool(Box::new(RunCommandTool::new(
        provider.clone(),
        cwd_path.clone(),
    )));
    agent.register_tool(Box::new(ReadFileTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ViewOutlineTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(WriteFileTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ReplaceInFileTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(MultiReplaceTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ListDirectoryTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(FindByNameTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(GrepSearchTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ShadowGitTool));
    agent.register_tool(Box::new(WebFetchTool));
    agent.register_tool(Box::new(NativeSearchGraphTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(NativeMemoryTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ManageTaskTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(LspTool));
    agent.register_tool(Box::new(ManageProcessTool));
    agent.register_tool(Box::new(AskUserTool::new(
        agent.interaction_runtime.clone(),
    )));
    agent.register_tool(Box::new(SkillTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(ReloadPluginsTool::new(agent.get_tools_ref())));
    agent.register_tool(Box::new(CreateCheckpointTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(RevertToCheckpointTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(
        crate::services::brain::tools::checkpoint::ListCheckpointsTool::new(cwd_path.clone()),
    ));
    agent.register_tool(Box::new(ImpactRadarTool::new(cwd_path.clone())));
    agent.register_tool(Box::new(GoldenBenchmarkTool));
    agent.register_tool(Box::new(SynthesizeTool::new(
        agent.get_tools_ref(),
        cwd_path.clone(),
    )));
    agent.register_tool(Box::new(
        crate::services::brain::tools::blackboard_tool::BlackboardTool::new(
            crate::services::brain::agent_blackboard::AgentBlackboard::global(),
            "SubagentWorker",
        ),
    ));
    agent.register_tool(Box::new(
        crate::services::brain::tools::repo_map_tool::RepoMapTool::new(cwd_path.clone()),
    ));
    agent.register_tool(Box::new(
        crate::services::brain::tools::verify_task::VerifyTaskTool::new(
            provider.clone(),
            cwd_path.clone(),
        ),
    ));

    // 自动扫描与装载动态插件与 MCP 工具
    agent.plugin_registry.load_dynamic_plugins();

    let runtime = Arc::new(SubagentRuntime::new(
        "worker",
        crate::services::brain::steering_queue::SteeringQueue::default_location().into(),
    ));
    agent.register_tool(Box::new(InvokeSubagentTool::new(
        runtime.clone(),
        "worker".to_string(),
    )));
    agent.register_tool(Box::new(CheckWorkerStatusTool::new(
        runtime.clone(),
        "worker".to_string(),
    )));
    agent.register_tool(Box::new(InterveneWorkerTool::new(runtime.clone())));
    agent.register_tool(Box::new(CancelSubagentTool::new(runtime.clone())));

    let result_store = crate::services::brain::tool_result_store::ToolResultStore::new(cwd_path);
    agent.register_tool(Box::new(ReadToolResultTool::new(result_store)));

    match agent.run(prompt).await {
        Ok(result) => {
            println!(
                "{}",
                serde_json::json!({ "type": "status", "status": "completed", "result": result })
            );
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "type": "error", "content": e }));
        }
    }
}
