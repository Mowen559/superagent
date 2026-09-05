pub async fn run_worker(prompt: &str, project: Option<String>) {
    use crate::services::brain::capabilities::shell::LocalShellProvider;
    use crate::services::brain::executor::AgentLoop;
    use crate::services::brain::subagent_runtime::SubagentRuntime;
    use crate::services::brain::tools::fs::{ReadFileTool, WriteFileTool};
    use crate::services::brain::tools::interaction::AskUserTool;
    use crate::services::brain::tools::lsp_tool::LspTool;
    use crate::services::brain::tools::manage_process::ManageProcessTool;
    use crate::services::brain::tools::native_graph::NativeSearchGraphTool;
    use crate::services::brain::tools::native_memory::NativeMemoryTool;
    use crate::services::brain::tools::reload_plugins::ReloadPluginsTool;
    use crate::services::brain::tools::shell::RunCommandTool;
    use crate::services::brain::tools::skill_tool::SkillTool;
    use crate::services::brain::tools::subagent::{
        CancelSubagentTool, CheckWorkerStatusTool, InterveneWorkerTool, InvokeSubagentTool,
    };
    use crate::services::brain::tools::todo::ManageTaskTool;
    use crate::services::brain::tools::tool_result::ReadToolResultTool;
    use crate::services::brain::tools::web::WebFetchTool;
    use std::sync::Arc;

    let mut agent = AgentLoop::new(
        "You are Superagent, running in a headless worker context. Your outputs are emitted as JSON lines to stdout.",
    );
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
