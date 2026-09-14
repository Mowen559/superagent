pub async fn run_worker(prompt: &str, project: Option<String>) {
    use crate::services::brain::capabilities::shell::LocalShellProvider;
    use crate::services::brain::executor::AgentLoop;
    use crate::services::brain::subagent_runtime::SubagentRuntime;
    use crate::services::brain::tools::fs::{
        FindByNameTool, GrepSearchTool, ListDirectoryTool, MultiReplaceTool, ReadFileTool,
        ReplaceInFileTool, WriteFileTool,
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
    use crate::services::brain::tools::todo::ManageTaskTool;
    use crate::services::brain::tools::tool_result::ReadToolResultTool;
    use crate::services::brain::tools::web::WebFetchTool;
    use std::sync::Arc;

    let worker_system_prompt = r#"# 身份设定 (Identity)
你名为 SuperAgent，是一个由 AI 驱动的高级自主智能体 (Autonomous Principal Engineer)。你运行在隔离的任务工作区中，拥有完整的物理文件与终端操作权限。

# 核心工程能动性宣言 (Proactive High-Agency Directives)
1. 探索先行与精准定位：
   - 面对未知代码或任务目标，主动使用 `list_dir` 浏览目录、使用 `find_by_name` 搜索目标文件、使用 `grep_search` 定位符号与报错关键词。
   - 充分利用原生代码图谱 `search_graph` 与海马体记忆 `super_memory`。
2. 外科手术式精确修改（Surgical Edits）：
   - 【严禁盲目重写整个文件】：修改代码时，必须优先使用 `replace_in_file` 或 `multi_replace_file_content` 进行局部块级精确替换。只有在新建文件或内容完全重构时才使用 `write_to_file`。
   - 注意保持代码原有的缩进格式、命名习惯与未修改的注释。
3. 闭环自愈与假设检验（Compiler-in-the-Loop & Self-Healing）：
   - 当文件工具返回非空的 `diagnostics`（语法/编译器错误）时，必须在下一步优先纠正，绝不带着编译错误推进后续逻辑。
   - 工具执行报错时，不要畏缩，应分析错误输出、调整假设并迅速重试。
4. TDD 确定性验证：
   - 完成修改后，优先调用 `run_command` 执行针对性测试（如 `mvn test -Dtest=...`、`npm test`、`cargo test`），确认测试变绿后再交付结果。
"#;

    let mut agent = AgentLoop::new(worker_system_prompt);
    agent.emit_jsonl = true;

    let provider = Arc::new(LocalShellProvider);
    let cwd_path = project.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });

    agent.register_tool(Box::new(RunCommandTool::new(
        provider.clone(),
        cwd_path.clone(),
    )));
    agent.register_tool(Box::new(ReadFileTool::new(cwd_path.clone())));
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
