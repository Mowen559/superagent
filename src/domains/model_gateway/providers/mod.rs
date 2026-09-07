use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{
    config::llm_settings::{self, LLMAccount, LLMSettings},
    services::brain::context_budget::ContextBudget,
    domains::model_gateway::{
        model_attempt_router::ModelErrorClass,
        routing::{ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt, ProviderCandidate},
        health::{ProviderAdmission, ProviderHealthRuntime},
        protocols::upstream::{adapter_for_provider, UpstreamAdapter},
    }
};


use super::*;

pub mod analysis;
pub mod types;

pub use analysis::*;
pub use types::*;

/// Canonical LLM Gateway facade used by the agent runtime.
pub struct LlmProvider {
    settings: LLMSettings,
    client: reqwest::Client,
    model_override: Option<String>,
}


impl LlmProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            settings: llm_settings::read_llm_settings(),
            client,
            model_override: None,
        }
    }

    pub fn with_model_override(client: reqwest::Client, model: Option<String>) -> Self {
        let model_override = model.filter(|value| !value.trim().is_empty());
        let mut settings = llm_settings::read_llm_settings();
        if let Some(model) = &model_override {
            settings.model = model.clone();
        }
        Self {
            settings,
            client,
            model_override,
        }
    }

    fn effective_settings(&self) -> LLMSettings {
        let mut settings = llm_settings::read_llm_settings();
        if let Some(model) = &self.model_override {
            settings.model = model.clone();
        }
        settings
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
        let settings = self.effective_settings();
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

    /// Send a raw chat completion request without injecting system prompts for tools or personalities
    pub async fn chat_raw(
        &self,
        messages: Vec<Value>,
        system: Option<&str>,
    ) -> Result<(String, TokenUsage), String> {
        let settings = self.effective_settings();
        let mut msgs = Vec::new();
        if let Some(sys) = system {
            msgs.push(json!({ "role": "system", "content": sys }));
        }
        msgs.extend(messages);

        let request_shape = json!({
            "messages": msgs,
            "stream": false,
        });
        let requirements = ModelRequirements::from_canonical_body(&request_shape);
        let runtime = ModelRouteRuntime::default();
        let route = runtime.plan_with_requirements(&settings, &settings.model, &[], requirements);
        if route.is_empty() {
            return Err(
                "LLM is not configured or no Provider Route satisfies Model Requirements".into(),
            );
        }

        let (response, usage) = self.execute_route(&settings, route.candidates, msgs, None).await?;
        let text = response
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok((text, usage))
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
                if let Err(e) = crate::services::cli_auth::AuthRuntime::global().get_valid_token(&mut candidate.account, &self.client).await {
                    receipts.push(ProviderAttemptReceipt::new(
                        &candidate,
                        attempt,
                        elapsed_ms(attempt_started),
                        ModelErrorClass::Authentication.into(),
                        Some(401),
                    ));
                    if runtime.should_try_next(ModelErrorClass::Authentication, attempt < candidate_count) {
                        break;
                    }
                    return Err(format!("Auth error: {}", e));
                }
                
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
                let adapter = adapter_for_provider(&attempt_settings);
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
                        health.finish_attempt(
                            &health_key,
                            class.into(),
                            elapsed_ms(attempt_started),
                        );
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

                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        tracing::warn!(
                            account_id = %candidate.account.id,
                            "Token is unauthorized (HTTP {}), Gateway returning Auth error directly",
                            status.as_u16()
                        );
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
                        health.finish_attempt(
                            &health_key,
                            class.into(),
                            elapsed_ms(attempt_started),
                        );
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


}

#[cfg(test)]
mod tests;
