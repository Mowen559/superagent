pub async fn run_worker(
    prompt: &str,
    project: Option<String>,
    model: Option<String>,
    resume_session_id: Option<String>,
    tool_policy: Option<String>,
    image_paths: Vec<String>,
) {
    // 解析工作模式：写入 HUB_TOOL_POLICY_MODE 供 ToolExecutionGateway 在
    // AgentLoop 构造期消费（plan→只读、build→审批、edit→自动改、yolo→放行、guarded→危险审批）。
    let work_mode = tool_policy
        .as_deref()
        .and_then(crate::services::brain::tool_policy::WorkMode::parse);
    if let Some(mode) = work_mode {
        unsafe {
            std::env::set_var("HUB_TOOL_POLICY_MODE", mode.as_str());
        }
        eprintln!(
            "\x1b[33m[WorkMode] 🔧 Superagent worker starting in '{}' mode\x1b[0m",
            mode.as_str()
        );
    }

    // 确保每个 worker 拥有专属干净的 session，避免背负历史默认 session 的海量事件包袱
    if let Some(ref sid) = resume_session_id {
        if !sid.trim().is_empty() {
            unsafe {
                std::env::set_var("HUB_AGENT_SESSION_ID", sid);
            }
        }
    } else if std::env::var("HUB_AGENT_SESSION_ID")
        .map(|s| s == "default" || s.is_empty())
        .unwrap_or(true)
    {
        let new_session_id = format!("run_{}", uuid::Uuid::new_v4().simple());
        unsafe {
            std::env::set_var("HUB_AGENT_SESSION_ID", new_session_id);
        }
    }
    use crate::services::brain::capabilities::shell::LocalShellProvider;
    use crate::domains::agent_cognitive::executor::AgentLoop;
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
    use crate::services::brain::tools::project_index_tool::ProjectIndexTool;
    use crate::services::brain::tools::openclaw::OpenclawTool;
    use crate::services::brain::tools::hermes::HermesTool;
    use crate::services::brain::tools::native_harness::NativeHarnessTool;
    use std::sync::Arc;

    let app_paths = crate::config::app_paths::AppPaths::new();
    let persona = crate::config::persona_settings::load_persona(&app_paths);
    let persona_section = persona.render_prompt_section();

    let worker_system_prompt = format!(
        r#"{persona_section}

# 核心思维逻辑契约 (Self-Governing Cognitive Directives)
1. 自我能力审视与敏捷探索先行 (Capability Self-Awareness & Agile Inspection First)：
   - 面对任务，首先审视手头具备的工具武器库：`project_index`（全仓架构拓扑与 AST 画像，毫秒级返回！）、`search_graph`（代码图谱调用链）、`view_outline`（类骨架与路由）、`grep_search`（关键词精准检索）。
   - 【极其重要】：勘测架构现状、模块依赖或定位代码时，**必须优先调用轻量级感知工具**（首选 `project_index` 或 `search_graph`，耗时仅数十毫秒），**严禁在初次勘测与提问阶段滥用耗时长达一分钟的 `mvn test` 全仓构建**！只有在实际修改了代码需要验证回归时，才针对性运行构建测试。
2. 边查库边反问的双轨协作范式 (Interleaved Dual-Track Inquiry Protocol)：
   - 面对宏观需求、信息缺失或关键架构分支选型，采用“边查边问”工作法：一边调用感知工具（如 `project_index`）翻查现有代码与能力库，一边主动向用户提出高质量的推敲提问。
   - 提问必须具备深度与专业度，严格遵循结构化三段式：
     ① 【我已查实的事实】：列举调用 `project_index` 或代码检索查到的现有代码结构与服务现状；
     ② 【技术方案权衡矩阵】：清晰罗列【方案 A】与【方案 B】的优劣势、架构解耦度与潜在风险；
     ③ 【专业推荐与选项】：给出明确推荐（如“强烈推荐方案 A”），并调用 `ask_user` 工具或在回复中列出选项供用户决策。
3. 外科手术式精确修改（Surgical Edits）：
   - 【严禁盲目重写整个文件】：修改代码时，必须优先使用 `replace_in_file` 或 `multi_replace_file_content` 进行局部块级精确替换。只有在新建文件或内容完全重构时才使用 `write_to_file`。
   - 【绝对约束】：严禁修改 common 目录！所有改动必须在业务服务内部自闭环。
   - 注意保持代码原有的缩进格式、命名习惯与未修改的注释。
4. 验证闭环与物理事实准则 (Grounding & Action Integrity)：
   - 每次代码修改后，必须主动执行对应的构建检查或单测命令（如针对具体模块的 `mvn test -pl <module>`、`cargo test`、`npm test`）验证改动。
   - 牢记 SWE-bench 双向测试契约：测试必须产生真实物理增量，旧测试必须保持全绿，绝不允许拿存量测试蒙混过关。
   - 牢记 OpenClaw 门禁：所有声称“已修改/已写入”的结论必须有真实的写盘工具收据，严禁虚假承诺！
5. 自主进化与超级记忆闭环 (Continuous Evolution & SuperMemory Directives):
   - 你拥有本地持久化超级记忆库 `super_memory` 与动态技能库 `skill_tool`。
   - 面对复杂的架构选型、接口规范或疑难报错，可随时调用 `super_memory(action='search')` 或 `skill_tool(action='list')` 检索既往经验。
   - 在攻克关键架构节点、总结出通用标准化模式或完成自愈后，主动调用 `super_memory(action='store')` 沉淀核心规则，或调用 `skill_tool(action='save')` 固化为工程技能！
6. 元认知画像与自适应人设演进 (Self-Persona Evolution):
   - 你拥有自我画像更新工具 `update_self_persona`。
   - 当你在研发实践中沉淀了新的技术专长（如精通 Rust 异步运行时）、领悟了新的工作哲学，或与用户形成了专属默契时，可随时调用 `update_self_persona` 调整座右铭、增补特质标签或追加生平经历。
"#
    );

    // 定位工作区与宿主工程真实路径 (沙箱逃逸自愈定位)
    let cwd_path = project.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });
    let resolved_project_path = crate::infrastructure::fs_path::resolve_origin_project_from_baseline(
        &cwd_path,
        &[".supermemory"],
    );
    let project_path_str = resolved_project_path.to_string_lossy().to_string();

    // 潜意识唤醒：从 SuperMemory 检索与当前 Prompt 最相关的项目规范与历史经验
    let app_paths = crate::config::app_paths::AppPaths::new();
    let runtime_config = crate::config::runtime::RuntimeConfig::from_env(&app_paths);
    let store = crate::services::supermemory_store::SupermemoryStore::new(
        &runtime_config,
        Some(&project_path_str),
    );

    let relevant_memories = store.search(prompt, 5);
    let mut full_system_prompt = worker_system_prompt.to_string();
    if let Some(mode) = work_mode {
        full_system_prompt.push_str("\n\n# 当前协作工作模式 (Active Collaboration Mode)\n");
        full_system_prompt.push_str(match mode {
            crate::services::brain::tool_policy::WorkMode::Plan => {
                "当前处于【计划模式 Plan Mode】：只读探索、严禁改动。\n\
                 - 你只能调用只读工具（project_index / search_graph / view_outline / grep_search / read_file 等）完成调研。\n\
                 - 写文件、执行命令等变更工具已被系统硬性拒绝，不要尝试调用。\n\
                 - 你必须产出一份结构化实施计划：目标、改动清单（文件+要点）、风险与回滚、验收标准，等待用户批准后再切换模式执行。"
            }
            crate::services::brain::tool_policy::WorkMode::Build => {
                "当前处于【构建模式 Build Mode】：变更前确认。\n\
                 - 只读工具自由使用；写文件、执行命令等变更操作会触发用户审批门禁。\n\
                 - 收到审批拒绝时，必须调整方案而不是反复重试同一操作。"
            }
            crate::services::brain::tool_policy::WorkMode::Edit => {
                "当前处于【自动编辑模式 Edit Mode】：代码改动直接落地。\n\
                 - 文件级编辑（write_to_file / replace_in_file / multi_replace_file_content）无需确认，直接外科手术式落地。\n\
                 - 执行命令、联网等高风险操作仍会触发审批；请优先用编辑工具完成改动并用轻量命令验证。"
            }
            crate::services::brain::tool_policy::WorkMode::Yolo => {
                "当前处于【完全自主模式 Yolo Mode】：全程无人工确认。\n\
                 - 所有工具直接放行；请保持外科手术式修改与验证闭环，仍然严禁虚假承诺。"
            }
            crate::services::brain::tool_policy::WorkMode::Guarded => {
                "当前处于【守护自主模式 Guarded Mode】：自主执行、危险操作审批。\n\
                 - 常规读写与命令直接执行；不可逆破坏性操作（回滚/删除/重置）会触发用户审批。"
            }
        });
        full_system_prompt.push('\n');
    }
    if !relevant_memories.is_empty() {
        full_system_prompt.push_str("\n\n# 潜意识架构记忆库 (SuperMemory Subconscious Context)\n");
        full_system_prompt.push_str("【系统已从宿主工程 SuperMemory 记忆库中预先召回与当前任务高度相关的工程规范与沉淀经验】：\n");
        for (idx, mem) in relevant_memories.iter().enumerate() {
            full_system_prompt.push_str(&format!(
                "{}. [{}] {}\n   - 规范摘要: {}\n",
                idx + 1,
                mem.node_type,
                mem.name,
                mem.summary.trim()
            ));
        }
        full_system_prompt.push_str("【执行红线】：你必须严格遵守上述记忆中的架构约束与经验。如有需要，可使用 `super_memory` 或 `skill_tool` 进行检索或沉淀。\n");
        eprintln!(
            "\x1b[35m[Subconscious Awakening] 🧠 Retrieved {} relevant memories from SuperMemory for prompt: {}\x1b[0m",
            relevant_memories.len(),
            prompt.chars().take(60).collect::<String>()
        );
    }

    // 动态技能侦测与挂载 (Proactive Domain Skill Scouting & Mounting)
    let matched_skills = crate::domains::agent_cognitive::capability_scout::CapabilityScout::scout_and_match_skills(prompt, &cwd_path);
    if !matched_skills.is_empty() {
        full_system_prompt.push_str("\n\n# 🎯 动态装载的业务技能规约 (Active Domain Skill Directives)\n");
        full_system_prompt.push_str("【中枢已根据用户意图自动识别并装载以下专属业务技能契约，你必须强制严格遵守】：\n\n");
        for skill in &matched_skills {
            full_system_prompt.push_str(&skill.render_system_prompt_section());
            eprintln!(
                "\x1b[32;1m[Skill Scout] 🎯 Successfully matched and loaded Skill '{}' (score: {:.1})\x1b[0m",
                skill.name, skill.score
            );
        }
    }

    let mut agent = AgentLoop::with_model(&full_system_prompt, model.as_deref());
    agent.emit_jsonl = true;

    // 注册核心能力与工具箱 (Fully Armed Superagent Toolset)
    let provider = Arc::new(LocalShellProvider);

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
    agent.register_tool(Box::new(ProjectIndexTool::new(cwd_path.clone())));
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
            crate::domains::agent_cognitive::agent_blackboard::AgentBlackboard::global(),
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
    agent.register_tool(Box::new(OpenclawTool));
    agent.register_tool(Box::new(HermesTool::default()));
    agent.register_tool(Box::new(NativeHarnessTool));
    agent.register_tool(Box::new(
        crate::services::brain::tools::self_persona_tool::UpdateSelfPersonaTool::new(
            app_paths.clone(),
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

    let result_store = crate::services::brain::tool_result_store::ToolResultStore::new(cwd_path.clone());
    agent.register_tool(Box::new(ReadToolResultTool::new(result_store)));

    // 启动后台 Stdin 异步消息循环，打通与 Hub 主进程的双向交互（支持 AskUser 回复与动态干预）
    let interaction_runtime = agent.interaction_runtime.clone();
    let steering_queue = agent.steering_queue.clone();
    let stdin_handle = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use crate::services::brain::ipc::IpcMessage;
        use crate::services::brain::steering_queue::SteeringSource;

        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line).await {
            if n == 0 {
                break;
            }
            if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                match msg {
                    IpcMessage::UserResponse {
                        request_id,
                        response,
                    } => {
                        let _ = interaction_runtime.deliver_response(&request_id, &response);
                    }
                    IpcMessage::Intervene(instruction) => {
                        let _ = steering_queue.enqueue(
                            SteeringSource::Supervisor,
                            &instruction,
                            None,
                        );
                    }
                    _ => {}
                }
            }
            line.clear();
        }
    });

    let run_res = agent.run_with_images(prompt, load_image_data_urls(&image_paths)).await;
    stdin_handle.abort();

    // 自动触发会话轨迹经验自省与自愈提炼，持久化写入 SuperMemory
    let persisted_count = crate::domains::agent_cognitive::plan_reflection::ReflectionEngine::distill_and_persist(
        &agent.events,
        &runtime_config,
        Some(&project_path_str),
    );
    if persisted_count > 0 {
        eprintln!(
            "\x1b[32m[SuperMemory] 🧬 Distilled & persisted {} golden rules to {}\x1b[0m",
            persisted_count, project_path_str
        );
    }

    // 自动技能结晶飞轮 (Auto-Skill Crystallization Flywheel)
    if let Some(crystallized) = crate::domains::agent_cognitive::skill_crystallizer::AutoSkillCrystallizer::crystallize_from_trajectory(
        prompt,
        &agent.events,
        &cwd_path,
    ) {
        eprintln!(
            "\x1b[36;1m[Skill Crystallizer] 💎 Successfully crystallized new durable Skill '{}' at {}\x1b[0m",
            crystallized.name, crystallized.path
        );
    }

    let usage_val = agent.last_token_usage.as_ref().map(|u| {
        serde_json::json!({
            "prompt_tokens": u.prompt_tokens,
            "completion_tokens": u.completion_tokens,
            "total_tokens": u.prompt_tokens + u.completion_tokens,
            "cost": u.cost,
            "duration_ms": u.duration_ms
        })
    });

    match run_res {
        Ok(result) => {
            println!(
                "{}",
                serde_json::json!({
                    "type": "status",
                    "status": "completed",
                    "result": result,
                    "usage": usage_val
                })
            );
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "type": "error", "content": e }));
        }
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// 将落盘图片读取为 data URL（worker 侧一次性内联，投影阶段零 IO）。
/// 已是 data URL 的输入原样透传；读取失败或超出体积上限的条目跳过并告警，
/// 不阻断整次运行。
fn load_image_data_urls(paths: &[String]) -> Vec<String> {
    const MAX_IMAGE_BYTES: usize = 6 * 1024 * 1024;
    let mut urls = Vec::with_capacity(paths.len());
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("data:") {
            urls.push(trimmed.to_string());
            continue;
        }
        let mime = match std::path::Path::new(trimmed)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/png",
        };
        match std::fs::read(trimmed) {
            Ok(bytes) if bytes.len() <= MAX_IMAGE_BYTES => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                urls.push(format!("data:{mime};base64,{encoded}"));
            }
            Ok(bytes) => {
                eprintln!(
                    "\x1b[33m[Images] skipped '{trimmed}': {} bytes exceeds {MAX_IMAGE_BYTES} limit\x1b[0m",
                    bytes.len()
                );
            }
            Err(error) => {
                eprintln!("\x1b[33m[Images] failed to read '{trimmed}': {error}\x1b[0m");
            }
        }
    }
    urls
}
