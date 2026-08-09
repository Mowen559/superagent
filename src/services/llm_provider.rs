use serde_json::{json, Value};
use crate::config::llm_settings::{self, LLMSettings};

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

/// Some OpenAI-compatible proxies (e.g. the antigravity Gemini bridge) inject
/// their own agent tools into every request; summary-style prompts then lure
/// the model into emitting `tool_calls` / `MALFORMED_FUNCTION_CALL` with an
/// empty `content`, and the proxy may hang for minutes retrying. This
/// directive is appended to the system prompt to keep the model in plain-text
/// mode.
const NO_TOOLS_DIRECTIVE: &str = "\u{4f60}\u{6ca1}\u{6709}\u{4efb}\u{4f55}\u{53ef}\u{7528}\u{5de5}\u{5177}\u{ff0c}\u{7981}\u{6b62}\u{8c03}\u{7528}\u{4efb}\u{4f55}\u{5de5}\u{5177}\u{6216}\u{51fd}\u{6570}\u{ff0c}\u{76f4}\u{63a5}\u{4ee5}\u{7eaf}\u{6587}\u{672c}\u{8f93}\u{51fa}\u{6700}\u{7ec8}\u{7b54}\u{6848}\u{3002} You have no tools available; never call any tool or function, output the final answer as plain text.";

const ENGINEERING_SYSTEM_PROMPT: &str = r#"
# Engineering & Orchestration Principles
1. Precision over Scope: Do not add features, refactor code, or make improvements beyond what was asked. A simple feature does not need extra configurability. Do not add docstrings or type annotations for code you are not modifying.
2. Pragmatic Engineering: Do not design abstractions for hypothetical future needs. Do not write fallback logic for scenarios that cannot happen. Solve the immediate core problem only.
3. Diagnose Before Acting: If a method fails, you must diagnose the cause before switching tactics—read the error, check your assumptions. Do not blindly retry the exact same operation.
4. Honest Reporting: Report results accurately. If tests fail, state it clearly. If you did not execute validation steps, say so instead of implying success. Do not gloss over errors to manufacture a green result.
"#;

#[derive(Debug, Default, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost: f64,
    pub duration_ms: u64,
}

/// LLM Provider - handles communication with OpenAI-compatible or Anthropic APIs
pub struct LlmProvider {
    settings: LLMSettings,
    client: reqwest::Client,
}

impl LlmProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            settings: llm_settings::read_llm_settings(),
            client,
        }
    }

    pub fn is_available(&self) -> bool {
        llm_settings::has_usable_settings(&self.settings)
    }

    /// Send a chat completion request
    pub async fn chat(&self, messages: Vec<Value>, system: Option<&str>) -> Result<(String, TokenUsage), String> {
        if !self.is_available() {
            return Err("LLM is not configured".into());
        }

        // Empty-content responses (proxy tool-call quirks, see NO_TOOLS_DIRECTIVE)
        // are transient: retry once before giving up.
        let mut last_err = String::new();
        for _attempt in 0..2 {
            match self.chat_once(&messages, system).await {
                Ok((content, usage)) if !content.trim().is_empty() => return Ok((content, usage)),
                Ok(_) => last_err = "LLM returned empty content (likely hijacked into a tool call by the proxy)".to_string(),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    async fn chat_once(&self, messages: &[Value], system: Option<&str>) -> Result<(String, TokenUsage), String> {
        let start_time = std::time::Instant::now();

        // Always pin the model to plain-text output; proxy-injected tools
        // otherwise derail summary-style prompts (empty content / long hangs).
        let combined_system = match system {
            Some(sys) => format!("{}\n{}\n{}", sys, ENGINEERING_SYSTEM_PROMPT, NO_TOOLS_DIRECTIVE),
            None => format!("{}\n{}", ENGINEERING_SYSTEM_PROMPT, NO_TOOLS_DIRECTIVE),
        };

        let base = self.settings.base_url.trim_end_matches('/');

        let actual_model = {
            let mapping = self.settings.active_account_id.as_ref()
                .and_then(|id| self.settings.accounts.iter().find(|a| a.id == *id))
                .map(|account| &account.model_mapping);
            
            mapping
                .and_then(|m| m.get(&self.settings.model))
                .unwrap_or(&self.settings.model)
                .clone()
        };

        if self.settings.provider == "gemini_native" {
            use crate::services::protocol_converter::upstream::UpstreamAdapter;
            use crate::services::protocol_converter::gemini_internal_upstream::GeminiInternalUpstreamAdapter;
            
            let adapter = GeminiInternalUpstreamAdapter;
            
            // Build standard body
            let mut msgs = Vec::new();
            msgs.push(json!({ "role": "system", "content": combined_system }));
            msgs.extend(messages.iter().cloned());
            
            let standard_body = json!({
                "model": actual_model,
                "max_tokens": self.settings.max_tokens,
                "temperature": self.settings.temperature,
                "messages": msgs,
            });

            let (target_url, headers_map, upstream_body) = adapter.prepare_request(&self.settings, standard_body).map_err(|e| format!("Adapter error: {}", e))?;
            
            let mut req = self.client.post(&target_url).timeout(std::time::Duration::from_secs(180));
            for (k, v) in headers_map.iter() {
                req = req.header(k, v);
            }
            
            tracing::info!("LLM Request: POST {} via GeminiInternalUpstreamAdapter", target_url);

            let response = req.json(&upstream_body).send().await.map_err(|e| format!("LLM request failed: {}", e))?;
            
            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(format!("LLM API error ({}): {}", status, text));
            }
            
            let result: Value = response.json().await.map_err(|e| format!("Failed to parse LLM response: {}", e))?;
            let standard_res = adapter.response_to_standard(result).map_err(|e| format!("Adapter response error: {}", e))?;
            
            let content = standard_res["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string();
            
            let mut usage = TokenUsage::default();
            if let Some(u) = standard_res.get("usage") {
                usage.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let comp = u.get("completion_tokens")
                    .or_else(|| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                usage.completion_tokens = comp;
            }
            usage.duration_ms = start_time.elapsed().as_millis() as u64;
            usage.cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
                &actual_model,
                usage.prompt_tokens,
                usage.completion_tokens,
                0,
                0,
            );
            
            return Ok((content, usage));
        }

        let url = if self.settings.provider == "anthropic" || base.contains("api.anthropic.com") {
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        } else if self.settings.provider.contains("gemini") || self.settings.provider.contains("google") || base.contains("generativelanguage.googleapis.com") {
            if base.ends_with("/v1beta") {
                format!("{}/openai/chat/completions", base)
            } else if base.ends_with("/v1beta/openai") {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/chat/completions", base)
            }
        } else {
            if base.ends_with("/v1") {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/v1/chat/completions", base)
            }
        };

        let body = if self.settings.provider == "anthropic" {
            let mut b = json!({
                "model": actual_model,
                "max_tokens": self.settings.max_tokens,
                "temperature": self.settings.temperature,
                "messages": messages,
            });
            b["system"] = json!(combined_system);
            b
        } else {
            let mut msgs = Vec::new();
            msgs.push(json!({ "role": "system", "content": combined_system }));
            msgs.extend(messages.iter().cloned());
            json!({
                "model": actual_model,
                "max_tokens": self.settings.max_tokens,
                "temperature": self.settings.temperature,
                "messages": msgs,
            })
        };

        let mut req = self.client
            .post(&url)
            .timeout(std::time::Duration::from_secs(180))
            .header("Content-Type", "application/json");

        req = req.header("Authorization", format!("Bearer {}", self.settings.api_key));
        if self.settings.api_key.starts_with("ya29.") {
            if let Some(pid) = &self.settings.project_id {
                req = req.header("x-goog-user-project", pid);
            }
        }

        tracing::info!("LLM Request: POST {} with Auth len {}", url, self.settings.api_key.len());

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("LLM API error ({}): {}", status, text));
        }

        let result: Value = response.json().await.map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        // Extract content based on provider
        let content = if self.settings.provider == "anthropic" {
            // Thinking-capable models (e.g. Gemini via the antigravity proxy) may emit
            // `thinking` blocks before the answer, so collect every `text` block instead
            // of assuming content[0] is the reply.
            let blocks = result["content"].as_array().cloned().unwrap_or_default();
            let text: String = blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            if !text.is_empty() {
                text
            } else {
                // Degenerate case: the model stopped after thinking only.
                blocks
                    .iter()
                    .filter_map(|b| b["thinking"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            }
        } else {
            result["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string()
        };

        // Extract usage
        let mut usage = TokenUsage::default();
        if let Some(u) = result.get("usageMetadata") {
            usage.prompt_tokens = u.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            usage.completion_tokens = u.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        } else if let Some(u) = result.get("usage") {
            usage.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            // Anthropic uses output_tokens, OpenAI uses completion_tokens
            let comp = u.get("completion_tokens")
                .or_else(|| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            usage.completion_tokens = comp;
        }

        usage.duration_ms = start_time.elapsed().as_millis() as u64;
        usage.cost = crate::services::cost_calculator::CostCalculator::calculate_cost(
            &actual_model,
            usage.prompt_tokens,
            usage.completion_tokens,
            0,
            0,
        );

        Ok((content, usage))
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

        let (response, _) = self.chat(messages, Some("You are a code analysis assistant. Respond only with valid JSON.")).await?;
        let cleaned = strip_code_fences(&response);

        serde_json::from_str(&cleaned).map_err(|_| {
            // If not valid JSON, wrap in a basic structure
            json!({
                "fileSummary": response,
                "tags": [],
                "complexity": "moderate"
            })
        }).or_else(|_| Ok(json!({
            "fileSummary": response,
            "tags": [],
            "complexity": "moderate"
        })))
    }

    /// Produce a natural-language comprehension of a single file from its
    /// compressed structural skeleton plus graph-derived structure facts.
    /// Returns Markdown in the requested output language.
    pub async fn explain_file(&self, file_path: &str, skeleton: &str, structure: &str, lang: &str) -> Result<String, String> {
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
    pub async fn explain_project(&self, structure: &str, lang: &str, project_language: &str) -> Result<String, String> {
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
    pub async fn explain_impact(&self, changed_files: &str, impacted_nodes: &str, lang: &str) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下代码变更文件的影响面（爆炸半径），输出简洁的 Markdown 报告：\n1) 直接影响：修改了哪些核心文件；\n2) 级联风险：基于依赖关系，哪些上游调用方受到了影响；\n3) 测试建议：重点回归测试哪些模块。\n{}\n\n变更的文件：\n{}\n\n受影响的上游节点(Graph推演)：\n{}",
                directive, changed_files, impacted_nodes
            )
        })];
        self.chat(messages, Some("你是资深安全与质量工程师，擅长评估代码修改带来的级联风险。")).await.map(|(c, _)| c)
    }

    /// 根据节点特征预测架构分层 (Architecture Analyzer)
    pub async fn explain_architecture(&self, file_nodes: &str, lang: &str) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下文件集合，尝试将它们进行架构分层（如 Controller/Router, Service/Domain, Repository/Database, UI/View 等），输出 Markdown 报告：\n{}\n\n文件列表：\n{}",
                directive, file_nodes
            )
        })];
        self.chat(messages, Some("你是资深架构师，擅长从文件命名和路径推断其在架构设计中的层级位置。")).await.map(|(c, _)| c)
    }

    /// 逆向推演需求：找出拟修改的入口文件
    pub async fn predict_requirement_entry(&self, requirement: &str, file_context: &str, lang: &str, project_language: &str) -> Result<String, String> {
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
    pub async fn explain_domain_flow(&self, entry_point: &str, downstream_nodes: &str, lang: &str) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "你是一个业务分析专家。以下是围绕某个入口节点提取的业务调用链路（DFS遍历），请将这段链路翻译成通俗易懂的业务流程文档：\n{}\n\n入口点：{}\n\n下游依赖节点：\n{}",
                directive, entry_point, downstream_nodes
            )
        })];
        self.chat(messages, Some("你是资深业务架构师，擅长将底层的方法调用图翻译为清晰的业务处理流程。")).await.map(|(c, _)| c)
    }

    /// Fetch available models from the provider (OpenAI compatible /v1/models endpoint)
    pub async fn fetch_models(&self) -> Result<Vec<String>, String> {
        let base = self.settings.base_url.trim_end_matches('/');
        let url = if (self.settings.provider.contains("gemini") || self.settings.provider.contains("google") || base.contains("generativelanguage.googleapis.com")) && base.ends_with("/v1beta") {
            format!("{}/openai/models", base)
        } else if base.ends_with("/v1") {
            format!("{}/models", base)
        } else {
            format!("{}/v1/models", base)
        };
        
        let response = self.client.get(&url)
            .header("Authorization", format!("Bearer {}", self.settings.api_key))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?;
            
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Failed to fetch models ({}): {}", status, text));
        }
        
        let result: Value = response.json().await.map_err(|e| format!("Failed to parse models JSON: {}", e))?;
        
        let mut models = vec![];
        if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(id) = item.get("id").and_then(|id| id.as_str()) {
                    models.push(id.to_string());
                }
            }
        } else {
            // Anthropic doesn't have a standard models endpoint in the same way, or it might just return empty/different format.
            // Some providers might just return an array directly or a different structure.
        }
        
        models.sort();
        Ok(models)
    }
}
