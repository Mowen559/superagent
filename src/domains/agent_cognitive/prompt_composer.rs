//! 模块化开源标准系统提示词组装引擎 (Composable Prompt Composer)
//! 借鉴 Claude Code, Aider 与 SWE-agent 的架构设计，提供分层语义化 XML 结构与工作区规则零编译热载入。

use std::path::Path;

pub struct PromptComposer;

impl PromptComposer {
    /// 组装轻量、灵活、对齐现代开源编码标准的系统提示词
    pub fn compose(workspace_root: &Path, persona_section: Option<&str>) -> String {
        let os_name = if cfg!(windows) { "Windows" } else { "Linux / Unix" };
        let cwd_str = workspace_root.display().to_string();

        let mut prompt = String::with_capacity(4096);

        // 1. 核心角色与结对工程定位
        prompt.push_str(
            "<role>\n\
            You are MoZhi (莫智), an autonomous senior fullstack software engineer and pair programmer.\n\
            You excel in system architecture, frontend & backend engineering, debugging, and test-driven development.\n\
            You approach every engineering problem with pragmatism, clarity, and precision.\n\
            </role>\n\n"
        );

        // 2. 人设补充（若开启）
        if let Some(persona) = persona_section {
            let trimmed = persona.trim();
            if !trimmed.is_empty() {
                prompt.push_str(&format!("<persona>\n{}\n</persona>\n\n", trimmed));
            }
        }

        // 3. 当前运行时环境事实 (Runtime Environment Facts)
        let shell_info = if cfg!(windows) {
            "- Shell: Windows PowerShell. NOTE: Use `curl.exe` (or `Invoke-RestMethod`) for HTTP verification to avoid alias hangs.\n"
        } else {
            "- Shell: POSIX sh / bash.\n"
        };

        prompt.push_str(&format!(
            "<environment>\n\
            - Operating System: {}\n\
            - Workspace Root: {}\n\
            {}\
            </environment>\n\n",
            os_name, cwd_str, shell_info
        ));

        // 4. 通用工程协作准则 (Core Engineering Guidelines)
        prompt.push_str(
            "<guidelines>\n\
            1. Precision over Scope: Strictly focus on the user's explicit objective. Avoid unsolicited re-architecture or extraneous dependencies.\n\
            2. Grounded Exploration: Inspect existing project files, directory layouts, and configuration before authoring or modifying code.\n\
            3. Surgical Edits: Prefer fine-grained replacement or structured AST tools (`replace_in_file`, `ast_grep`) over whole-file rewrites whenever possible.\n\
            4. Closed-loop Verification: After making changes or generating new projects, verify correctness using tests, builds, or commands (`verify_task_tool`, `run_command`).\n\
            5. Structured Collaboration: When tackling complex multi-step tasks, use `manage_task` to track your plan and provide transparent progress.\n\
            6. Honest Delivery: Present concrete facts, test outcomes, and file locations in your final response.\n\
            </guidelines>\n\n"
        );

        // 5. 工作区与用户私有规则自动挂载 (External Rule Overlay - Zero-compile Hot Reload)
        let mut loaded_rules = Vec::new();

        // 5.1 工作区私有规则: .superagentrules / AGENTS.md / .clinerules
        for rule_file_name in &[".superagentrules", "AGENTS.md", ".clinerules"] {
            let rule_path = workspace_root.join(rule_file_name);
            if rule_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&rule_path) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        loaded_rules.push(format!("<!-- Source: {} -->\n{}", rule_file_name, trimmed));
                    }
                }
            }
        }

        // 5.2 全局用户规则: ~/.superagent/system_prompt.md
        if let Some(home) = crate::config::app_paths::AppPaths::home_dir() {
            let global_prompt = home.join(".superagent").join("settings").join("system_prompt.md");
            if global_prompt.is_file() {
                if let Ok(content) = std::fs::read_to_string(&global_prompt) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        loaded_rules.push(format!("<!-- Source: ~/.superagent/system_prompt.md -->\n{}", trimmed));
                    }
                }
            }
        }

        if !loaded_rules.is_empty() {
            prompt.push_str("<custom_rules>\n");
            for r in loaded_rules {
                prompt.push_str(&r);
                prompt.push_str("\n\n");
            }
            prompt.push_str("</custom_rules>\n\n");
        }

        prompt
    }
}
