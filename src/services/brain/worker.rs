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

# 核心思维与执行契约 (Autonomous Engineering Directives)
1. 行动优先与默认最佳实践 (Action-First with Sensible Defaults):
   - 你是顶级的全栈与规则架构工程师。直接用专业、高质的成果说话，严禁以客服式推诿反问用户或要求用户做选择题。
   - 面对细节未穷尽的需求，直接采用工业界最高标准（Sensible Defaults，如小红书默认展开多图图集并提取免嗅视频流）。
2. 先查实后动手 (Grounding First):
   - 修改代码或排障前，优先使用只读感知工具（`view_file`、`grep_search`、`find_by_name`、`web_fetch`）查实真实代码、逆向数据与接口，杜绝凭空臆测。
3. 外科手术式修改 (Surgical Precision):
   - 必须优先使用 `replace_in_file` 进行局部块级精确替换，保留无关注释与排版。严禁盲目全量覆写文件破坏存量逻辑。
4. 验证闭环与物理收据 (Verification Closed-Loop):
   - 代码落地后，主动执行对应测试或校验命令（如 `cargo test`、`npm test`、`validate-rule`、`test-rule`）。
   - 严禁凭空伪造海阔短口令或虚假测试通过收据。
5. 简洁交付 (Direct & Clean Delivery):
   - 回复以清晰精炼的中文为主，重点呈现改动总结、测试验证结果或安装口令，杜绝官话套话与自言自语。
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
    let effective_root = if resolved_project_path.exists() && resolved_project_path != cwd_path {
        eprintln!(
            "\x1b[36;1m[Sandbox Escape] 🚀 Auto-ascended from sandbox '{}' to real project root '{}'\x1b[0m",
            cwd_path.display(),
            resolved_project_path.display()
        );
        resolved_project_path.clone()
    } else {
        cwd_path.clone()
    };
    let project_path_str = effective_root.to_string_lossy().to_string();

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
        full_system_prompt.push_str(
            "\n【技能自治交付最高纲领 (Zero-Prompt Protocol)】：\n\
             当前任务已成功匹配并挂载专属业务技能 (Domain Skill)。\n\
             1. 你必须严格遵循挂载技能规约中的架构规范与工程方法论，作为你的内置专业本能；\n\
             2. 【严禁抛出反问】：严禁向用户抛出方案选择、技术细节或架构追问，严禁只输出空洞反问或方案，必须直接推进解决问题并给出实际交付物；\n\
             3. 【成熟智能体自主闭环 (ReAct)】：\n\
                - 若用户任务为【修复报错/修改现有代码】：仔细分析报错原因与上下文，自主调用 `read_file` 查阅代码，必要时使用 `web_fetch` 联网探针目标站点，定位根因后输出精准修复方案与修复后的代码；若涉及规则且需要验证，可调用配备的测试脚本；\n\
                - 若用户任务为【新建规则/全生命周期产出】：自主编写规则，调用测试与口令生成工具，从命令输出中提取真实的 PasteMe 云端短口令（带 16 位随机哈希）和标准离线 Base64 原生口令，以及沙箱 5 星评分真实收据，杜绝凭空捏造；\n\
                - 若用户任务为【技术咨询/原理分析】：严密论证，直接输出深入浅出的技术解答；\n\
             4. 【优雅折叠完整源码】：当交付完整规则或大段代码时，建议使用 `<details><summary><b>点击展开查看规则完整源码 (JSON)</b></summary>\n\n```json\n...\n```\n</details>` 语法包裹，兼顾紧凑与复制体验；\n\
             5. 【绝不造假】：严禁在未调用工具的情况下臆造假口令！当工具执行失败时，认真根据报错输出进行反思修正（Self-Correction）！\n"
        );
    }

    let mut agent = AgentLoop::with_model(&full_system_prompt, model.as_deref());
    agent.emit_jsonl = true;

    // 注册核心能力与工具箱 (Fully Armed Superagent Toolset)
    let provider = Arc::new(LocalShellProvider);

    agent.register_tool(Box::new(RunCommandTool::new(
        provider.clone(),
        effective_root.clone(),
    )));
    agent.register_tool(Box::new(ReadFileTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ViewOutlineTool::new(effective_root.clone())));
    agent.register_tool(Box::new(WriteFileTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ReplaceInFileTool::new(effective_root.clone())));
    agent.register_tool(Box::new(MultiReplaceTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ListDirectoryTool::new(effective_root.clone())));
    agent.register_tool(Box::new(FindByNameTool::new(effective_root.clone())));
    agent.register_tool(Box::new(GrepSearchTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ShadowGitTool));
    agent.register_tool(Box::new(WebFetchTool));
    agent.register_tool(Box::new(NativeSearchGraphTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ProjectIndexTool::new(effective_root.clone())));
    agent.register_tool(Box::new(NativeMemoryTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ManageTaskTool::new(effective_root.clone())));
    agent.register_tool(Box::new(LspTool));
    agent.register_tool(Box::new(ManageProcessTool));
    agent.register_tool(Box::new(AskUserTool::new(
        agent.interaction_runtime.clone(),
    )));
    agent.register_tool(Box::new(SkillTool::new(effective_root.clone())));
    agent.register_tool(Box::new(ReloadPluginsTool::new(agent.get_tools_ref())));
    agent.register_tool(Box::new(CreateCheckpointTool::new(effective_root.clone())));
    agent.register_tool(Box::new(RevertToCheckpointTool::new(effective_root.clone())));
    agent.register_tool(Box::new(
        crate::services::brain::tools::checkpoint::ListCheckpointsTool::new(effective_root.clone()),
    ));
    agent.register_tool(Box::new(ImpactRadarTool::new(effective_root.clone())));
    agent.register_tool(Box::new(GoldenBenchmarkTool));
    agent.register_tool(Box::new(SynthesizeTool::new(
        agent.get_tools_ref(),
        effective_root.clone(),
    )));
    agent.register_tool(Box::new(
        crate::services::brain::tools::blackboard_tool::BlackboardTool::new(
            crate::domains::agent_cognitive::agent_blackboard::AgentBlackboard::global(),
            "SubagentWorker",
        ),
    ));
    agent.register_tool(Box::new(
        crate::services::brain::tools::repo_map_tool::RepoMapTool::new(effective_root.clone()),
    ));
    agent.register_tool(Box::new(
        crate::services::brain::tools::verify_task::VerifyTaskTool::new(
            provider.clone(),
            effective_root.clone(),
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

    let result_store = crate::services::brain::tool_result_store::ToolResultStore::new(effective_root.clone());
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
