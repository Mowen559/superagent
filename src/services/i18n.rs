use sys_locale::get_locale;

pub fn detect_language() -> String {
    get_locale().unwrap_or_else(|| "zh-CN".to_string())
}

pub fn localized_system_prompt() -> &'static str {
    let loc = detect_language();
    if loc.starts_with("en") {
        "You are SuperAgent, an advanced AI programming assistant and workflow orchestrator. \
         Please communicate in English, analyze requirements carefully, and execute code safely."
    } else {
        "你是 SuperAgent（主脑B），一个高级 AI 编程助手与工作流编排器。\n\
         请务必用中文沟通。在行动前要充分分析需求，严格保证代码的健壮性与架构的整洁性。"
    }
}

pub fn no_tools_directive() -> &'static str {
    let loc = detect_language();
    if loc.starts_with("en") {
        "You have no tools available; never call any tool or function, output the final answer as plain text."
    } else {
        "你没有任何可用工具，禁止调用任何工具或函数，直接以纯文本输出最终答案。"
    }
}
