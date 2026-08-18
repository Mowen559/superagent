//! 大脑 B (Executor): 动态执行变异体
//! 作为子进程被 `hub` 拉起，专职负责执行具体的思维流和任务处理
//! 可以通过修改自身代码，并触发重新编译实现热重载自举

use hub_rs::services::brain::executor::AgentLoop;
use hub_rs::services::brain::tools::fs::{ReadFileTool, WriteFileTool};
use hub_rs::services::brain::tools::shell::RunCommandTool;
use hub_rs::services::brain::tools::web::WebFetchTool;
use hub_rs::services::brain::tools::mcp::McpProxyTool;
use hub_rs::services::brain::tools::self_evolve::RequestReloadTool;
use hub_rs::services::brain::tools::interaction::AskUserTool;
use hub_rs::services::brain::tools::subagent::InvokeSubagentTool;
use hub_rs::services::brain::tools::todo::ManageTaskTool;
use hub_rs::services::brain::tools::manage_process::ManageProcessTool;
use hub_rs::services::brain::tools::lsp_tool::LspTool;
use hub_rs::services::brain::tools::skill_tool::SkillTool;

#[tokio::main]
async fn main() {
    eprintln!("Superagent Worker (Brain B) is starting...");
    
    // 初始化吸收自 Harness 的核心循环执行器
    let mut engine = AgentLoop::new("你是一个强大的自我进化 AI...\n\n[Environment Context]\n- OS: windows (x86_64)\n- Current Directory: D:\\桌面\\gitai\\superagent\n\n# Engineering & Orchestration Principles\n1. Precision over Scope: Do not add features, refactor code, or make improvements beyond what was asked.\n2. Pragmatic Engineering: Solve the immediate core problem only.\n3. Diagnose Before Acting: Read errors and check assumptions before retrying.\n4. Honest Reporting: Report results accurately.\n5. Skill Engine: You have access to a specialized skill engine. Use `skill_tool` to list and read specialized project guidelines when needed.");
    // 注册基础工具
    engine.register_tool(Box::new(ReadFileTool));
    engine.register_tool(Box::new(WriteFileTool));
    engine.register_tool(Box::new(RunCommandTool));
    engine.register_tool(Box::new(WebFetchTool));
    engine.register_tool(Box::new(McpProxyTool));
    engine.register_tool(Box::new(LspTool));
    engine.register_tool(Box::new(SkillTool::new(std::env::current_dir().unwrap())));

    // 注册子智能体与任务管理工具
    engine.register_tool(Box::new(AskUserTool));
    engine.register_tool(Box::new(InvokeSubagentTool));
    engine.register_tool(Box::new(ManageTaskTool));
    engine.register_tool(Box::new(ManageProcessTool));
    
    // 注册自举重载工具
    engine.register_tool(Box::new(RequestReloadTool));
    
    // 注册人工介入工具
    engine.register_tool(Box::new(AskUserTool));

    // 注册高级认知工具 (子智能体分身与任务清单)
    engine.register_tool(Box::new(InvokeSubagentTool));
    engine.register_tool(Box::new(ManageTaskTool));
    
    // 启动 IPC 通信循环 (JSON-RPC) 监听来自 Hub 的 prompt，
    // 并将 prompt 交给 engine.run(prompt).await 执行。
    hub_rs::services::brain::ipc::start_worker_loop(engine).await;
    
    eprintln!("Worker loop exiting.");
}
