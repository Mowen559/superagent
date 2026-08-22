use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{
    config::llm_settings::{self, LLMAccount, LLMSettings},
    services::{
        brain::{
            context_budget::ContextBudget,
            model_attempt_router::ModelErrorClass,
            model_route_runtime::{
                ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome,
                ProviderAttemptReceipt, ProviderCandidate,
            },
            provider_health_runtime::{ProviderAdmission, ProviderHealthRuntime},
        },
        protocol_converter::upstream::{adapter_for_provider, UpstreamAdapter},
    },
};

/// Map a BCP-47 locale (e.g. `zh-CN` / `en-US`) to an output-language
/// instruction for prompts. Defaults to Simplified Chinese.
pub fn lang_directive(lang: &str) -> (&'static str, &'static str) {
    if lang.to_lowercase().starts_with("en") {
        ("Respond in English.", "en")
    } else {
        ("请用简体中文回答。", "zh")
    }
}

/// Strip Markdown code fences (e.g. ```json ... ```) that LLMs often wrap
/// around structured payloads, returning the inner content ready for parsing.
pub fn strip_code_fences(response: &str) -> String {
    let trimmed = response.trim();
    if !trimmed.contains("```") {
        return trimmed.to_string();
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        // Drop the info string (e.g. "json") up to the first newline.
        let body = after.split_once('\n').map(|(_, b)| b).unwrap_or(after);
        if let Some(end) = body.find("```") {
            return body[..end].trim().to_string();
        }
        return body.trim().to_string();
    }
    trimmed.to_string()
}

#[derive(Debug, Default, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost: f64,
    pub duration_ms: u64,
    pub account_id: Option<String>,
    pub provider: Option<String>,
    pub attempts: u8,
    pub attempt_receipts: Vec<ProviderAttemptReceipt>,
}

/// Canonical LLM Gateway facade used by the agent runtime.
pub struct LlmProvider {
    settings: LLMSettings,
    client: reqwest::Client,
}

fn max_context_chars_for(account: Option<&LLMAccount>, model: &str) -> usize {
    let normalized = model.to_ascii_lowercase();
    if let Some(info_map) = account.and_then(|account| account.models_info.as_ref()) {
        if let Some(info) = info_map.get(model).or_else(|| info_map.get(&normalized)) {
            let configured_limit = info
                .get("inputTokenLimit")
                .or_else(|| info.get("context_length"))
                .or_else(|| info.get("context_window"))
                .or_else(|| info.get("max_position_embeddings"))
                .and_then(Value::as_u64);
            if let Some(limit) = configured_limit {
                return limit.saturating_mul(4).min(usize::MAX as u64) as usize;
            }
        }
    }

    if normalized.contains("gemini-1.5-pro") {
        8_000_000
    } else if normalized.contains("gemini") {
        4_000_000
    } else if normalized.contains("claude") {
        800_000
    } else if normalized.contains("gpt-4o")
        || normalized.contains("gpt-4-turbo")
        || normalized.contains("gpt-5")
    {
        500_000
    } else {
        100_000
    }
}

fn failure_receipt(
    candidate: &ProviderCandidate,
    attempt: usize,
    started: Instant,
    class: ModelErrorClass,
    status_code: Option<u16>,
) -> ProviderAttemptReceipt {
    ProviderAttemptReceipt::new(
        candidate,
        attempt,
        elapsed_ms(started),
        class.into(),
        status_code,
    )
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn usage_from_standard(response: &Value) -> TokenUsage {
    let usage = response.get("usage").unwrap_or(&Value::Null);
    TokenUsage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        completion_tokens: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        ..TokenUsage::default()
    }
}

fn render_receipts(receipts: &[ProviderAttemptReceipt]) -> String {
    serde_json::to_string(receipts).unwrap_or_else(|_| "[]".to_string())
}

fn failed_route_message(receipts: &[ProviderAttemptReceipt]) -> String {
    format!(
        "model request failed after {} Provider Route attempt(s); receipts={}",
        receipts.len(),
        render_receipts(receipts)
    )
}

impl LlmProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            settings: llm_settings::read_llm_settings(),
            client,
        }
    }

    pub fn max_context_chars(&self) -> usize {
        let account = self.settings.active_account_id.as_deref().and_then(|id| {
            self.settings
                .accounts
                .iter()
                .find(|account| account.id == id)
        });
        max_context_chars_for(account, &self.settings.model)
    }

    pub fn max_context_tokens(&self) -> usize {
        self.max_context_chars() / 4
    }

    pub fn is_available(&self) -> bool {
        !ModelRouteRuntime::default()
            .plan(&self.settings, &self.settings.model, &[])
            .is_empty()
    }

    /// Send a chat completion request with Tool schemas
    pub async fn chat_with_tools(
        &self,
        messages: Vec<Value>,
        system: Option<&str>,
        tools: Option<Value>,
    ) -> Result<(Value, TokenUsage), String> {
        // Reload on every autonomous step so account switching does not require
        // restarting a long-running agent session.
        let settings = llm_settings::read_llm_settings();
        let mut combined_system = system.unwrap_or("").to_string();
        if tools.is_none() {
            combined_system.push_str(&format!(
                "\n{}\n{}",
                crate::services::i18n::localized_system_prompt(),
                crate::services::i18n::no_tools_directive()
            ));
        } else {
            combined_system.push_str(&format!(
                "\n{}",
                crate::services::i18n::localized_system_prompt()
            ));
        }
        let mut msgs = Vec::new();
        msgs.push(json!({ "role": "system", "content": combined_system }));
        msgs.extend(messages);

        let mut request_shape = json!({
            "messages": msgs,
            "stream": false,
        });
        if let Some(tools) = tools.as_ref() {
            request_shape["tools"] = tools.clone();
        }
        let requirements = ModelRequirements::from_canonical_body(&request_shape);
        let runtime = ModelRouteRuntime::default();
        let route = runtime.plan_with_requirements(&settings, &settings.model, &[], requirements);
        if route.is_empty() {
            return Err(
                "LLM is not configured or no Provider Route satisfies Model Requirements".into(),
            );
        }

        self.execute_route(&settings, route.candidates, msgs, tools)
            .await
    }

    /// Send a chat completion request
    pub async fn chat(
        &self,
        messages: Vec<Value>,
        system: Option<&str>,
    ) -> Result<(String, TokenUsage), String> {
        let mut last_err = String::new();
        for _ in 0..2 {
            match self.chat_once(&messages, system).await {
                Ok((content, usage)) if !content.trim().is_empty() => return Ok((content, usage)),
                Ok(_) => {
                    last_err = "LLM returned empty content after a no-tools request".to_string()
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_err)
    }

    async fn chat_once(
        &self,
        messages: &[Value],
        system: Option<&str>,
    ) -> Result<(String, TokenUsage), String> {
        let (message, usage) = self
            .chat_with_tools(messages.to_vec(), system, None)
            .await?;
        Ok((
            message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            usage,
        ))
    }

    async fn execute_route(
        &self,
        settings: &LLMSettings,
        candidates: Vec<ProviderCandidate>,
        messages: Vec<Value>,
        tools: Option<Value>,
    ) -> Result<(Value, TokenUsage), String> {
        let runtime = ModelRouteRuntime::default();
        let health = ProviderHealthRuntime::global();
        let started = Instant::now();
        let candidate_count = candidates.len();
        let mut receipts = Vec::with_capacity(candidate_count);

        for (index, mut candidate) in candidates.into_iter().enumerate() {
            let attempt = index + 1;
            let mut attempt_started = Instant::now();
            let context_window =
                max_context_chars_for(Some(&candidate.account), &candidate.resolved_model) / 4;
            let request_budget = ContextBudget {
                context_window_tokens: context_window,
                requested_completion_tokens: settings.max_tokens as usize,
            }
            .plan(&messages, tools.as_ref());

            if request_budget.requires_compaction() {
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    elapsed_ms(attempt_started),
                    ProviderAttemptOutcome::ContextOverflow,
                    None,
                ));
                return Err(format!(
                    "context budget exceeded before request: estimated prompt {} tokens, overflow {} tokens; compact the session and retry; receipts={}",
                    request_budget.estimated_prompt_tokens,
                    request_budget.overflow_tokens,
                    render_receipts(&receipts)
                ));
            }

            let mut refresh_attempted = false;

            loop {
                let mut standard_body = json!({
                    "model": candidate.resolved_model,
                    "max_tokens": request_budget.completion_tokens,
                    "temperature": settings.temperature,
                    "messages": messages,
                });
                if let Some(tools) = tools.as_ref() {
                    standard_body["tools"] = tools.clone();
                }

                let attempt_settings = candidate.attempt_settings(settings);
                let adapter = adapter_for_provider(&attempt_settings.provider);
                let (target_url, headers, upstream_body) =
                    match adapter.prepare_request(&attempt_settings, standard_body) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            let class = runtime.classify_failure(&error.to_string());
                            receipts.push(failure_receipt(
                                &candidate,
                                attempt,
                                attempt_started,
                                class,
                                None,
                            ));
                            if runtime.should_try_next(class, attempt < candidate_count) {
                                break;
                            }
                            return Err(failed_route_message(&receipts));
                        }
                    };

                let health_key = candidate.health_key();
                if let ProviderAdmission::Rejected { retry_after_ms } =
                    health.begin_attempt(&health_key)
                {
                    tracing::warn!(
                        account_id = %candidate.account.id,
                        provider = %candidate.account.provider,
                        retry_after_ms,
                        "Provider Circuit is open; skipping candidate"
                    );
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        elapsed_ms(attempt_started),
                        ProviderAttemptOutcome::CircuitOpen,
                        None,
                    ));
                    if attempt < candidate_count {
                        break;
                    }
                    return Err(failed_route_message(&receipts));
                }

                tracing::info!(
                    account_id = %candidate.account.id,
                    provider = %candidate.account.provider,
                    model = %candidate.resolved_model,
                    attempt,
                    "executing Provider Route candidate"
                );
                let mut request = self
                    .client
                    .post(target_url)
                    .timeout(Duration::from_secs(180));
                for (name, value) in &headers {
                    request = request.header(name, value);
                }

                let response = match request.json(&upstream_body).send().await {
                    Ok(response) => response,
                    Err(error) => {
                        let class = runtime.classify_failure(&format!("request failed: {error}"));
                        health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                        receipts.push(failure_receipt(
                            &candidate,
                            attempt,
                            attempt_started,
                            class,
                            None,
                        ));
                        if runtime.should_try_next(class, attempt < candidate_count) {
                            break;
                        }
                        return Err(failed_route_message(&receipts));
                    }
                };

                let status = response.status();
                if !status.is_success() {
                    let provider_error = response.text().await.unwrap_or_default();
                    let class = runtime.classify_http_failure(status.as_u16(), &provider_error);
                    health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));

                    if (status.as_u16() == 401 || status.as_u16() == 403) && !refresh_attempted {
                        if let Some(ref rt) = candidate.account.refresh_token {
                            tracing::info!(
                                account_id = %candidate.account.id,
                                "Token is unauthorized (HTTP {}), attempting auto-refresh",
                                status.as_u16()
                            );
                            eprintln!("Auto-refreshing token for account {}", candidate.account.id);
                            if let Some(new_token) = crate::services::subscription::refresh_gemini_token(&self.client, rt).await {
                                eprintln!("Auto-refresh SUCCESS for {}", candidate.account.id);
                                // Save to config
                                let new_token_clone = new_token.clone();
                                let account_id = candidate.account.id.clone();
                                if let Err(e) = crate::config::llm_settings::update_llm_settings(move |settings| {
                                    for a in &mut settings.accounts {
                                        if a.id == account_id {
                                            a.api_key = new_token_clone.clone();
                                            break;
                                        }
                                    }
                                    Ok(())
                                }) {
                                    tracing::error!("Failed to save refreshed token to llm.json: {}", e);
                                }
                                
                                candidate.account.api_key = new_token;
                                refresh_attempted = true;
                                attempt_started = std::time::Instant::now(); // Reset timer for the retry
                                continue;
                            } else {
                                eprintln!("Auto-refresh failed for account {}", candidate.account.id);
                                tracing::error!("Auto-refresh failed for account {}", candidate.account.id);
                            }
                        }
                    }

                    receipts.push(failure_receipt(
                        &candidate,
                        attempt,
                        attempt_started,
                        class,
                        Some(status.as_u16()),
                    ));
                    if runtime.should_try_next(class, attempt < candidate_count) {
                        break;
                    }
                    return Err(failed_route_message(&receipts));
                }

                let provider_body: Value = match response.json().await {
                    Ok(body) => body,
                    Err(_) => {
                        health.finish_attempt(
                            &health_key,
                            ProviderAttemptOutcome::Permanent,
                            elapsed_ms(attempt_started),
                        );
                        receipts.push(failure_receipt(
                            &candidate,
                            attempt,
                            attempt_started,
                            ModelErrorClass::Permanent,
                            Some(status.as_u16()),
                        ));
                        return Err(failed_route_message(&receipts));
                    }
                };
                let standard_response = match adapter.response_to_standard(provider_body) {
                    Ok(response) => response,
                    Err(error) => {
                        let class = runtime.classify_failure(&error.to_string());
                        health.finish_attempt(&health_key, class.into(), elapsed_ms(attempt_started));
                        receipts.push(failure_receipt(
                            &candidate,
                            attempt,
                            attempt_started,
                            class,
                            Some(status.as_u16()),
                        ));
                        if runtime.should_try_next(class, attempt < candidate_count) {
                            break;
                        }
                        return Err(failed_route_message(&receipts));
                    }
                };
                let Some(message) = standard_response.pointer("/choices/0/message") else {
                    health.finish_attempt(
                        &health_key,
                        ProviderAttemptOutcome::Permanent,
                        elapsed_ms(attempt_started),
                    );
                    receipts.push(failure_receipt(
                        &candidate,
                        attempt,
                        attempt_started,
                        ModelErrorClass::Permanent,
                        Some(status.as_u16()),
                    ));
                    return Err(failed_route_message(&receipts));
                };

                let attempt_duration_ms = elapsed_ms(attempt_started);
                health.finish_attempt(
                    &health_key,
                    ProviderAttemptOutcome::Succeeded,
                    attempt_duration_ms,
                );
                receipts.push(ProviderAttemptReceipt::new(
                    &candidate,
                    attempt,
                    attempt_duration_ms,
                    ProviderAttemptOutcome::Succeeded,
                    Some(status.as_u16()),
                ));
                let mut usage = usage_from_standard(&standard_response);
                usage.duration_ms = elapsed_ms(started);
                usage.account_id = Some(candidate.account.id.clone());
                usage.provider = Some(candidate.account.provider.clone());
                usage.attempts = receipts.len().min(u8::MAX as usize) as u8;
                usage.cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
                    &candidate.resolved_model,
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    0,
                    0,
                );
                usage.attempt_receipts = receipts;
                return Ok((message.clone(), usage));
            }
        }
        Err(failed_route_message(&receipts))
    }

    /// Analyze a single file with LLM
    pub async fn analyze_file(&self, file_path: &str, content: &str) -> Result<Value, String> {
        let truncated = if content.len() > self.settings.ua_max_file_chars as usize {
            &content[..self.settings.ua_max_file_chars as usize]
        } else {
            content
        };

        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "Analyze this source file and return JSON with: fileSummary (string), tags (array), complexity (simple|moderate|complex).\n\nFile: {}\n\n```\n{}\n```",
                file_path, truncated
            )
        })];

        let (response, _) = self
            .chat(
                messages,
                Some("You are a code analysis assistant. Respond only with valid JSON."),
            )
            .await?;
        let cleaned = strip_code_fences(&response);

        serde_json::from_str(&cleaned)
            .map_err(|_| {
                // If not valid JSON, wrap in a basic structure
                json!({
                    "fileSummary": response,
                    "tags": [],
                    "complexity": "moderate"
                })
            })
            .or_else(|_| {
                Ok(json!({
                    "fileSummary": response,
                    "tags": [],
                    "complexity": "moderate"
                }))
            })
    }

    /// Produce a natural-language comprehension of a single file from its
    /// compressed structural skeleton plus graph-derived structure facts.
    /// Returns Markdown in the requested output language.
    pub async fn explain_file(
        &self,
        file_path: &str,
        skeleton: &str,
        structure: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let skeleton_block = if skeleton.trim().is_empty() {
            String::from("(源码不可用，仅依据代码图谱结构信息分析)")
        } else {
            format!("代码骨架：\n```\n{}\n```", skeleton)
        };
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析这个源文件，输出简洁的 Markdown，包含三部分：\n1) 文件职责：这个文件在项目中的角色与主要职责；\n2) 关键符号：挑选最重要的函数/类型说明其作用；\n3) 依赖关系：结合它依赖的文件与被引用的文件，说明它如何融入整体。\n{}\n\n文件：{}\n\n结构信息(来自代码图谱)：\n{}\n\n{}",
                directive, file_path, structure, skeleton_block
            )
        })];
        self.chat(messages, Some("你是资深代码分析助手，擅长快速讲清一个文件在项目中的作用，回答务必简洁、准确、结构化。")).await.map(|(c, _)| c)
    }

    /// Produce a project-level architecture overview from graph statistics.
    /// Returns Markdown in the requested output language.
    pub async fn explain_project(
        &self,
        structure: &str,
        lang: &str,
        project_language: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "总结这个项目的整体架构，输出简洁的 Markdown，包含：\n1) 项目概述：整体是做什么的；\n2) 主要模块：结合顶层模块与文件分布说明各自职责；\n3) 技术栈：结合语言分布做观察；\n4) 核心符号：高频符号反映出的关键能力。\n{}\n\n项目统计(来自代码图谱)：\n{}",
                directive, structure
            )
        })];
        self.chat(messages, Some(&format!("你是资深软件架构分析师，擅长从代码图谱统计推断项目结构，回答务必简洁、准确、结构化。当前项目的主力编程语言是：{}。在分析和输出报告时，请务必贴合该语言的生态特性、代码惯例（如包管理、框架范式）提供最专业的建议。", project_language))).await.map(|(c, _)| c)
    }

    /// 解释修改带来的爆炸半径 (Impact Analyzer)
    pub async fn explain_impact(
        &self,
        changed_files: &str,
        impacted_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下代码变更文件的影响面（爆炸半径），输出简洁的 Markdown 报告：\n1) 直接影响：修改了哪些核心文件；\n2) 级联风险：基于依赖关系，哪些上游调用方受到了影响；\n3) 测试建议：重点回归测试哪些模块。\n{}\n\n变更的文件：\n{}\n\n受影响的上游节点(Graph推演)：\n{}",
                directive, changed_files, impacted_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深安全与质量工程师，擅长评估代码修改带来的级联风险。"),
        )
        .await
        .map(|(c, _)| c)
    }

    /// 根据节点特征预测架构分层 (Architecture Analyzer)
    pub async fn explain_architecture(
        &self,
        file_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下文件集合，尝试将它们进行架构分层（如 Controller/Router, Service/Domain, Repository/Database, UI/View 等），输出 Markdown 报告：\n{}\n\n文件列表：\n{}",
                directive, file_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深架构师，擅长从文件命名和路径推断其在架构设计中的层级位置。"),
        )
        .await
        .map(|(c, _)| c)
    }

    /// 逆向推演需求：找出拟修改的入口文件
    pub async fn predict_requirement_entry(
        &self,
        requirement: &str,
        file_context: &str,
        lang: &str,
        project_language: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": format!(
                "需求描述:\n{}\n\n代码库关键文件上下文:\n{}\n\n请输出: \n1. 拟修改的文件列表（提取具体的文件路径）。\n2. 修改这些文件可能对系统造成的影响面评估。\n{}",
                requirement, file_context, directive
            )
        })];

        self.chat(messages, Some(&format!("你是一个高级代码架构师引擎。请根据用户的需求和当前项目的代码库上下文，推演出为了实现该需求，大概率需要修改哪些核心入口文件，以及可能产生的影响面。\n当前项目的主力编程语言是：{}。在分析和输出报告时，请务必贴合该语言的生态特性、代码惯例提供最专业的建议。\n【关键编排准则】：在评估时请重点分析“是否有侵入性更小、更务实的替代方案？”。请根据诊断和事实如实报告，杜绝无根据的臆测。", project_language))).await.map(|(c, _)| c)
    }

    /// 解释业务骨架和调用链路 (Domain Analyzer)
    pub async fn explain_domain_flow(
        &self,
        entry_point: &str,
        downstream_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "你是一个业务分析专家。以下是围绕某个入口节点提取的业务调用链路（DFS遍历），请将这段链路翻译成通俗易懂的业务流程文档：\n{}\n\n入口点：{}\n\n下游依赖节点：\n{}",
                directive, entry_point, downstream_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深业务架构师，擅长将底层的方法调用图翻译为清晰的业务处理流程。"),
        )
        .await
        .map(|(c, _)| c)
    }
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, routing::post, Json, Router};

    use super::*;

    fn candidate(id: &str, base_url: String) -> ProviderCandidate {
        ProviderCandidate {
            account: LLMAccount {
                id: id.to_string(),
                name: id.to_string(),
                provider: "openai".to_string(),
                models: vec!["test-model".to_string()],
                base_url,
                api_key: "test-key".to_string(),
                ..LLMAccount::default()
            },
            resolved_model: "test-model".to_string(),
        }
    }

    async fn test_server() -> String {
        let app = Router::new()
            .route(
                "/rate-limited/v1/chat/completions",
                post(|| async {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"error": {"message": "quota"}})),
                    )
                }),
            )
            .route(
                "/invalid/v1/chat/completions",
                post(|| async {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": {"message": "invalid schema"}})),
                    )
                }),
            )
            .route(
                "/healthy/v1/chat/completions",
                post(|| async {
                    Json(json!({
                        "id": "chatcmpl-test",
                        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 11, "completion_tokens": 3}
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn gateway_fails_over_and_returns_safe_attempt_receipts() {
        let base = test_server().await;
        let provider = LlmProvider::new(reqwest::Client::new());
        let settings = LLMSettings {
            model: "test-model".to_string(),
            max_tokens: 128,
            ..LLMSettings::default()
        };

        let (message, usage) = provider
            .execute_route(
                &settings,
                vec![
                    candidate("limited", format!("{base}/rate-limited")),
                    candidate("healthy", format!("{base}/healthy")),
                ],
                vec![json!({"role": "user", "content": "hello"})],
                None,
            )
            .await
            .unwrap();

        assert_eq!(message["content"], "ok");
        assert_eq!(usage.account_id.as_deref(), Some("healthy"));
        assert_eq!(usage.attempts, 2);
        assert_eq!(
            usage.attempt_receipts[0].outcome,
            ProviderAttemptOutcome::RateLimited
        );
        assert_eq!(
            usage.attempt_receipts[1].outcome,
            ProviderAttemptOutcome::Succeeded
        );
        let receipts = serde_json::to_string(&usage.attempt_receipts).unwrap();
        assert!(!receipts.contains("test-key"));
        assert!(!receipts.contains("hello"));
    }

    #[tokio::test]
    async fn gateway_stops_on_permanent_request_failure() {
        let base = test_server().await;
        let provider = LlmProvider::new(reqwest::Client::new());
        let settings = LLMSettings {
            model: "test-model".to_string(),
            max_tokens: 128,
            ..LLMSettings::default()
        };

        let error = provider
            .execute_route(
                &settings,
                vec![
                    candidate("invalid", format!("{base}/invalid")),
                    candidate("healthy", format!("{base}/healthy")),
                ],
                vec![json!({"role": "user", "content": "hello"})],
                None,
            )
            .await
            .unwrap_err();

        assert!(error.contains("after 1 Provider Route attempt"));
        assert!(error.contains("permanent"));
        assert!(!error.contains("invalid schema"));
    }

    #[tokio::test]
    async fn gateway_skips_open_circuit_without_contacting_provider() {
        let base = test_server().await;
        let provider = LlmProvider::new(reqwest::Client::new());
        let settings = LLMSettings {
            model: "test-model".to_string(),
            max_tokens: 128,
            ..LLMSettings::default()
        };
        let blocked = candidate("circuit-blocked", format!("{base}/invalid"));
        let blocked_key = blocked.health_key();
        let health = ProviderHealthRuntime::global();
        health.reset(&blocked_key);
        for _ in 0..3 {
            health.finish_attempt(&blocked_key, ProviderAttemptOutcome::Transient, 1);
        }

        let (message, usage) = provider
            .execute_route(
                &settings,
                vec![
                    blocked,
                    candidate("circuit-fallback", format!("{base}/healthy")),
                ],
                vec![json!({"role": "user", "content": "hello"})],
                None,
            )
            .await
            .unwrap();

        assert_eq!(message["content"], "ok");
        assert_eq!(
            usage.attempt_receipts[0].outcome,
            ProviderAttemptOutcome::CircuitOpen
        );
        assert_eq!(usage.attempts, 2);
        health.reset(&blocked_key);
    }
}
