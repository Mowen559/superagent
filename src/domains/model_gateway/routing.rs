use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use xxhash_rust::xxh3::xxh3_64;

use crate::config::llm_settings::{has_usable_settings, LLMAccount, LLMSettings};

use crate::domains::model_gateway::model_attempt_router::{ModelAttemptRouter, ModelErrorClass};
use crate::domains::model_gateway::capabilities::{CapabilityAssessment, ProviderCapabilityRuntime, assess_context_limit, assess_model_profile, CapabilityFreshness, CapabilityProfile, CapabilityVerdict};
use crate::services::brain::context_budget::estimate_json_tokens;

static ROUTE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One finite, credential-bearing Provider Route candidate. This type is
/// intentionally not `Debug`: an `LLMAccount` contains its API key.
#[derive(Clone)]
pub struct ProviderCandidate {
    pub account: LLMAccount,
    pub resolved_model: String,
}

impl ProviderCandidate {
    pub fn attempt_settings(&self, base: &LLMSettings) -> LLMSettings {
        let mut settings = base.clone();
        settings.provider = self.account.provider.clone();
        settings.base_url = self.account.base_url.clone();
        settings.api_key = self.account.api_key.clone();
        settings.project_id = self.account.project_id.clone();
        settings.model = self.resolved_model.clone();
        settings
    }

    pub fn account_health_key(&self) -> String {
        account_health_key(&self.account)
    }

    pub fn model_health_key(&self) -> String {
        model_health_key(&self.account, &self.resolved_model)
    }

    pub fn health_key(&self) -> String {
        self.model_health_key()
    }
}

pub fn account_health_key(account: &LLMAccount) -> String {
    format!(
        "acc:{}:{}:{:016x}",
        account.provider,
        account.id,
        xxh3_64(account.base_url.as_bytes())
    )
}

pub fn model_health_key(account: &LLMAccount, resolved_model: &str) -> String {
    format!(
        "mod:{}:{}:{:016x}:{}",
        account.provider,
        account.id,
        xxh3_64(account.base_url.as_bytes()),
        resolved_model
    )
}

pub fn provider_health_key(account: &LLMAccount) -> String {
    account_health_key(account)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequirements {
    pub tools: bool,
    pub vision: bool,
    pub streaming: bool,
    pub min_context_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityEvidence {
    Verified,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilityAssessment {
    pub evidence: ModelCapabilityEvidence,
    pub expired: bool,
}

impl ModelRequirements {
    pub fn from_canonical_body(body: &Value) -> Self {
        let tools = body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty());
        let vision = body.get("messages").is_some_and(contains_image_content);
        let prompt_tokens = body
            .get("messages")
            .map(estimate_json_tokens)
            .unwrap_or(0)
            .saturating_add(body.get("tools").map(estimate_json_tokens).unwrap_or(0));
        Self {
            tools,
            vision,
            streaming: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            // Minimum useful completion plus a conservative fixed safety margin.
            min_context_tokens: prompt_tokens.saturating_add(768),
        }
    }
}

#[derive(Clone)]
pub struct ProviderRoute {
    pub requested_model: String,
    pub candidates: Vec<ProviderCandidate>,
}

impl ProviderRoute {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAttemptOutcome {
    Succeeded,
    CircuitOpen,
    RateLimited,
    Transient,
    Authentication,
    ContextOverflow,
    Permanent,
}

impl From<ModelErrorClass> for ProviderAttemptOutcome {
    fn from(value: ModelErrorClass) -> Self {
        match value {
            ModelErrorClass::ContextOverflow => Self::ContextOverflow,
            ModelErrorClass::Authentication => Self::Authentication,
            ModelErrorClass::RateLimited => Self::RateLimited,
            ModelErrorClass::Transient => Self::Transient,
            ModelErrorClass::Permanent => Self::Permanent,
        }
    }
}

/// Safe telemetry for one provider attempt. It deliberately contains neither
/// credentials nor request/response bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAttemptReceipt {
    pub account_id: String,
    pub account_name: String,
    pub provider: String,
    pub resolved_model: String,
    pub attempt: usize,
    pub duration_ms: u64,
    pub outcome: ProviderAttemptOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}

impl ProviderAttemptReceipt {
    pub fn new(
        candidate: &ProviderCandidate,
        attempt: usize,
        duration_ms: u64,
        outcome: ProviderAttemptOutcome,
        status_code: Option<u16>,
    ) -> Self {
        Self {
            account_id: candidate.account.id.clone(),
            account_name: candidate.account.name.clone(),
            provider: candidate.account.provider.clone(),
            resolved_model: candidate.resolved_model.clone(),
            attempt,
            duration_ms,
            outcome,
            status_code,
        }
    }
}

/// Deep Module owning account eligibility, model mapping, priority ordering,
/// weighted rotation, fallback, and cross-account failover decisions.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelRouteRuntime {
    attempt_router: ModelAttemptRouter,
}

impl ModelRouteRuntime {
    pub fn plan(
        &self,
        settings: &LLMSettings,
        requested_model: &str,
        requested_tags: &[String],
    ) -> ProviderRoute {
        self.plan_with_requirements(
            settings,
            requested_model,
            requested_tags,
            ModelRequirements::default(),
        )
    }

    pub fn plan_with_requirements(
        &self,
        settings: &LLMSettings,
        requested_model: &str,
        requested_tags: &[String],
        requirements: ModelRequirements,
    ) -> ProviderRoute {
        if !settings.enabled {
            return ProviderRoute {
                requested_model: requested_model.to_string(),
                candidates: Vec::new(),
            };
        }

        let model = if requested_model.trim().is_empty() {
            settings.model.as_str()
        } else {
            requested_model.trim()
        };
        let mut grouped: BTreeMap<i32, Vec<LLMAccount>> = BTreeMap::new();

        for account in settings.accounts.iter().filter(|account| {
            account.enabled
                && supports_model(account, model)
                && supports_requirements(account, model, requirements)
                && requested_tags
                    .iter()
                    .all(|tag| account.tags.iter().any(|candidate| candidate == tag))
        }) {
            grouped
                .entry(account.priority)
                .or_default()
                .push(account.clone());
        }

        let sequence = ROUTE_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
        let mut accounts = Vec::new();
        for (_, mut group) in grouped.into_iter().rev() {
            if settings
                .routing_strategy
                .eq_ignore_ascii_case("round_robin")
            {
                weighted_order(&mut group, sequence);
            }
            accounts.extend(group);
        }

        if accounts.is_empty() {
            if let Some(active_id) = settings.active_account_id.as_deref() {
                if let Some(active) = settings.accounts.iter().find(|account| {
                    account.id == active_id
                        && account.enabled
                        && supports_requirements(account, model, requirements)
                }) {
                    accounts.push(active.clone());
                }
            }
        }

        if accounts.is_empty() && has_usable_settings(settings) {
            accounts.push(top_level_account(settings));
        }

        let mut seen = HashSet::new();
        let candidates = accounts
            .into_iter()
            .filter(|account| seen.insert(account.id.clone()))
            .map(|account| ProviderCandidate {
                resolved_model: account
                    .model_mapping
                    .get(model)
                    .cloned()
                    .unwrap_or_else(|| model.to_string()),
                account,
            })
            .collect();

        ProviderRoute {
            requested_model: model.to_string(),
            candidates,
        }
    }

    pub fn classify_failure(&self, error: &str) -> ModelErrorClass {
        self.attempt_router.classify(error)
    }

    pub fn classify_http_failure(&self, status_code: u16, provider_error: &str) -> ModelErrorClass {
        match status_code {
            401 | 403 => ModelErrorClass::Authentication,
            408 | 409 | 425 => ModelErrorClass::Transient,
            429 => ModelErrorClass::RateLimited,
            500..=599 => ModelErrorClass::Transient,
            _ => self.classify_failure(provider_error),
        }
    }

    pub fn should_try_next(&self, class: ModelErrorClass, has_next: bool) -> bool {
        has_next
            && matches!(
                class,
                ModelErrorClass::Authentication
                    | ModelErrorClass::RateLimited
                    | ModelErrorClass::Transient
            )
    }
}

fn supports_model(account: &LLMAccount, requested_model: &str) -> bool {
    account.models.iter().any(|model| model == requested_model)
        || account.model_mapping.contains_key(requested_model)
}

fn supports_requirements(
    account: &LLMAccount,
    requested_model: &str,
    requirements: ModelRequirements,
) -> bool {
    capability_assessment(account, requested_model, requirements).evidence
        != ModelCapabilityEvidence::Unsupported
}

pub fn capability_evidence(
    account: &LLMAccount,
    requested_model: &str,
    requirements: ModelRequirements,
) -> ModelCapabilityEvidence {
    capability_assessment(account, requested_model, requirements).evidence
}

pub fn capability_assessment(
    account: &LLMAccount,
    requested_model: &str,
    requirements: ModelRequirements,
) -> ModelCapabilityAssessment {
    let resolved_model = account
        .model_mapping
        .get(requested_model)
        .map(String::as_str)
        .unwrap_or(requested_model);
    let info = account.models_info.as_ref().and_then(|models| {
        models
            .get(resolved_model)
            .or_else(|| models.get(requested_model))
            .or_else(|| models.get(&resolved_model.to_ascii_lowercase()))
    });
    if requirements == ModelRequirements::default() {
        if let Some(info) = info.filter(|info| info.pointer("/capabilityEvidence/basic").is_some())
        {
            return model_capability_assessment(assess_model_profile(
                info,
                CapabilityProfile::Basic,
            ));
        }
        return ModelCapabilityAssessment {
            evidence: ModelCapabilityEvidence::Verified,
            expired: false,
        };
    }
    let Some(info) = info else {
        return ModelCapabilityAssessment {
            evidence: ModelCapabilityEvidence::Unknown,
            expired: false,
        };
    };

    let mut unknown = false;
    let mut expired = false;

    if requirements.tools {
        if let Some(unsupported) = merge_capability_assessment(
            assess_model_profile(info, CapabilityProfile::Tools),
            &mut unknown,
            &mut expired,
        ) {
            return unsupported;
        }
    }
    if requirements.vision {
        if let Some(unsupported) = merge_capability_assessment(
            assess_model_profile(info, CapabilityProfile::Vision),
            &mut unknown,
            &mut expired,
        ) {
            return unsupported;
        }
    }
    if requirements.streaming {
        if let Some(unsupported) = merge_capability_assessment(
            assess_model_profile(info, CapabilityProfile::Streaming),
            &mut unknown,
            &mut expired,
        ) {
            return unsupported;
        }
    }

    if requirements.min_context_tokens > 0 {
        if let Some(unsupported) = merge_capability_assessment(
            assess_context_limit(info, requirements.min_context_tokens),
            &mut unknown,
            &mut expired,
        ) {
            return unsupported;
        }
    }

    ModelCapabilityAssessment {
        evidence: if unknown {
            ModelCapabilityEvidence::Unknown
        } else {
            ModelCapabilityEvidence::Verified
        },
        expired,
    }
}

fn model_capability_assessment(
    assessment: crate::domains::model_gateway::capabilities::CapabilityAssessment,
) -> ModelCapabilityAssessment {
    ModelCapabilityAssessment {
        evidence: match assessment.verdict {
            CapabilityVerdict::Verified => ModelCapabilityEvidence::Verified,
            CapabilityVerdict::Unknown => ModelCapabilityEvidence::Unknown,
            CapabilityVerdict::Unsupported => ModelCapabilityEvidence::Unsupported,
        },
        expired: assessment.freshness == CapabilityFreshness::Expired,
    }
}

fn merge_capability_assessment(
    assessment: crate::domains::model_gateway::capabilities::CapabilityAssessment,
    unknown: &mut bool,
    expired: &mut bool,
) -> Option<ModelCapabilityAssessment> {
    *expired |= assessment.freshness == CapabilityFreshness::Expired;
    match assessment.verdict {
        CapabilityVerdict::Verified => None,
        CapabilityVerdict::Unknown => {
            *unknown = true;
            None
        }
        CapabilityVerdict::Unsupported => Some(ModelCapabilityAssessment {
            evidence: ModelCapabilityEvidence::Unsupported,
            expired: *expired,
        }),
    }
}

fn contains_image_content(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_image_content),
        Value::Object(object) => {
            matches!(
                object.get("type").and_then(Value::as_str),
                Some("image" | "image_url" | "input_image")
            ) || object.contains_key("image_url")
                || object.values().any(contains_image_content)
        }
        _ => false,
    }
}

fn top_level_account(settings: &LLMSettings) -> LLMAccount {
    LLMAccount {
        id: "__default__".to_string(),
        name: "Default provider".to_string(),
        enabled: true,
        provider: settings.provider.clone(),
        models: vec![settings.model.clone()],
        base_url: settings.base_url.clone(),
        api_key: settings.api_key.clone(),
        project_id: settings.project_id.clone(),
        ..LLMAccount::default()
    }
}

fn weighted_order(accounts: &mut [LLMAccount], sequence: u64) {
    accounts.sort_by(|left, right| {
        let left_score = rendezvous_score(sequence, left);
        let right_score = rendezvous_score(sequence, right);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn rendezvous_score(sequence: u64, account: &LLMAccount) -> f64 {
    let key = format!("{sequence}:{}", account.id);
    let unit = (xxh3_64(key.as_bytes()) as f64 + 1.0) / (u64::MAX as f64 + 2.0);
    unit.powf(1.0 / f64::from(account.weight.max(0.001)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, priority: i32, models: &[&str], tags: &[&str]) -> LLMAccount {
        LLMAccount {
            id: id.to_string(),
            name: id.to_string(),
            provider: "openai".to_string(),
            base_url: "https://example.test/v1".to_string(),
            api_key: format!("secret-{id}"),
            models: models.iter().map(|value| value.to_string()).collect(),
            tags: tags.iter().map(|value| value.to_string()).collect(),
            priority,
            ..LLMAccount::default()
        }
    }

    #[test]
    fn plans_only_eligible_accounts_by_priority_and_resolves_mapping() {
        let mut mapped = account("mapped", 20, &[], &["fast", "tools"]);
        mapped
            .model_mapping
            .insert("agent-model".to_string(), "provider-model".to_string());
        let settings = LLMSettings {
            accounts: vec![
                account("low", 1, &["agent-model"], &["fast", "tools"]),
                account("wrong-tag", 30, &["agent-model"], &["fast"]),
                mapped,
            ],
            model: "agent-model".to_string(),
            ..LLMSettings::default()
        };

        let route = ModelRouteRuntime::default().plan(
            &settings,
            "agent-model",
            &["fast".to_string(), "tools".to_string()],
        );

        assert_eq!(route.candidates.len(), 2);
        assert_eq!(route.candidates[0].account.id, "mapped");
        assert_eq!(route.candidates[0].resolved_model, "provider-model");
        assert_eq!(route.candidates[1].account.id, "low");
    }

    #[test]
    fn falls_back_to_active_account_then_top_level_configuration() {
        let active = account("active", 0, &["different-model"], &[]);
        let settings = LLMSettings {
            active_account_id: Some("active".to_string()),
            accounts: vec![active],
            model: "requested".to_string(),
            ..LLMSettings::default()
        };
        let route = ModelRouteRuntime::default().plan(&settings, "requested", &[]);
        assert_eq!(route.candidates[0].account.id, "active");

        let top_level = LLMSettings {
            accounts: Vec::new(),
            active_account_id: None,
            provider: "openai".to_string(),
            base_url: "https://example.test/v1".to_string(),
            api_key: "top-secret".to_string(),
            model: "requested".to_string(),
            ..LLMSettings::default()
        };
        let route = ModelRouteRuntime::default().plan(&top_level, "requested", &[]);
        assert_eq!(route.candidates[0].account.id, "__default__");
    }

    #[test]
    fn route_is_finite_and_deduplicated() {
        let duplicate = account("same", 10, &["model"], &[]);
        let settings = LLMSettings {
            accounts: vec![duplicate.clone(), duplicate],
            model: "model".to_string(),
            ..LLMSettings::default()
        };
        let route = ModelRouteRuntime::default().plan(&settings, "model", &[]);
        assert_eq!(route.candidates.len(), 1);
    }

    #[test]
    fn failover_policy_moves_only_recoverable_provider_failures() {
        let runtime = ModelRouteRuntime::default();
        assert!(runtime.should_try_next(ModelErrorClass::Authentication, true));
        assert!(runtime.should_try_next(ModelErrorClass::RateLimited, true));
        assert!(runtime.should_try_next(ModelErrorClass::Transient, true));
        assert!(!runtime.should_try_next(ModelErrorClass::ContextOverflow, true));
        assert!(!runtime.should_try_next(ModelErrorClass::Permanent, true));
        assert!(!runtime.should_try_next(ModelErrorClass::Transient, false));
        assert_eq!(
            runtime.classify_http_failure(403, "forbidden"),
            ModelErrorClass::Authentication
        );
        assert_eq!(
            runtime.classify_http_failure(503, "opaque provider error"),
            ModelErrorClass::Transient
        );
    }

    #[test]
    fn requirements_exclude_only_explicitly_unsupported_or_too_small_models() {
        let mut unsupported = account("unsupported", 30, &["model"], &[]);
        unsupported.models_info = Some(std::collections::HashMap::from([(
            "model".to_string(),
            serde_json::json!({
                "supportsTools": false,
                "supportsVision": false,
                "supportsStreaming": false,
                "inputTokenLimit": 512
            }),
        )]));
        let unknown = account("unknown", 20, &["model"], &[]);
        let mut capable = account("capable", 10, &["model"], &[]);
        capable.models_info = Some(std::collections::HashMap::from([(
            "model".to_string(),
            serde_json::json!({
                "capabilities": {"tools": true, "vision": true, "streaming": true},
                "context_window": 32000
            }),
        )]));
        let settings = LLMSettings {
            accounts: vec![unsupported, unknown, capable],
            model: "model".to_string(),
            ..LLMSettings::default()
        };

        let route = ModelRouteRuntime::default().plan_with_requirements(
            &settings,
            "model",
            &[],
            ModelRequirements {
                tools: true,
                vision: true,
                streaming: true,
                min_context_tokens: 8_000,
            },
        );

        assert_eq!(route.candidates.len(), 2);
        assert_eq!(route.candidates[0].account.id, "unknown");
        assert_eq!(route.candidates[1].account.id, "capable");
        assert_eq!(
            capability_evidence(
                &route.candidates[0].account,
                "model",
                ModelRequirements {
                    tools: true,
                    vision: true,
                    streaming: true,
                    min_context_tokens: 8_000,
                }
            ),
            ModelCapabilityEvidence::Unknown
        );
        assert_eq!(
            capability_evidence(
                &route.candidates[1].account,
                "model",
                ModelRequirements {
                    tools: true,
                    vision: true,
                    streaming: true,
                    min_context_tokens: 8_000,
                }
            ),
            ModelCapabilityEvidence::Verified
        );
    }

    #[test]
    fn derives_requirements_from_canonical_request() {
        let requirements = ModelRequirements::from_canonical_body(&serde_json::json!({
            "messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,x"}}]}],
            "tools": [{"type": "function", "function": {"name": "inspect"}}],
            "stream": true
        }));
        assert!(requirements.tools);
        assert!(requirements.vision);
        assert!(requirements.streaming);
        assert!(requirements.min_context_tokens >= 768);
    }

    #[test]
    fn expired_unsupported_evidence_degrades_to_unknown_and_reenters_the_route() {
        let mut stale = account("stale", 10, &["model"], &[]);
        stale.models_info = Some(std::collections::HashMap::from([(
            "model".to_string(),
            serde_json::json!({
                "capabilities": {"tools": false},
                "context_window": 32000,
                "capabilityEvidence": {
                    "tools": {
                        "source": "probe",
                        "outcome": "unsupported",
                        "observedAt": "2020-01-01T00:00:00Z",
                        "expiresAt": "2020-01-08T00:00:00Z"
                    }
                }
            }),
        )]));
        let settings = LLMSettings {
            accounts: vec![stale],
            model: "model".to_string(),
            ..LLMSettings::default()
        };
        let requirements = ModelRequirements {
            tools: true,
            min_context_tokens: 8_000,
            ..ModelRequirements::default()
        };

        let route = ModelRouteRuntime::default().plan_with_requirements(
            &settings,
            "model",
            &[],
            requirements,
        );
        assert_eq!(route.candidates.len(), 1);
        let assessment = capability_assessment(&route.candidates[0].account, "model", requirements);
        assert_eq!(assessment.evidence, ModelCapabilityEvidence::Unknown);
        assert!(assessment.expired);
    }

    #[test]
    fn expired_basic_probe_is_not_reported_as_verified() {
        let mut stale = account("stale-basic", 10, &["model"], &[]);
        stale.models_info = Some(std::collections::HashMap::from([(
            "model".to_string(),
            serde_json::json!({
                "capabilities": {"basic": true},
                "capabilityEvidence": {
                    "basic": {
                        "source": "probe",
                        "outcome": "verified",
                        "observedAt": "2020-01-01T00:00:00Z",
                        "expiresAt": "2020-01-08T00:00:00Z"
                    }
                }
            }),
        )]));

        let assessment = capability_assessment(&stale, "model", ModelRequirements::default());
        assert_eq!(assessment.evidence, ModelCapabilityEvidence::Unknown);
        assert!(assessment.expired);
    }

    #[test]
    fn two_tier_health_keys_are_distinct_and_isolated() {
        let acc = account("acc-1", 10, &["gpt-4o", "claude-3-5"], &[]);
        let acc_key = account_health_key(&acc);
        let mod_key_1 = model_health_key(&acc, "gpt-4o");
        let mod_key_2 = model_health_key(&acc, "claude-3-5");

        assert!(acc_key.starts_with("acc:openai:acc-1:"));
        assert!(mod_key_1.starts_with("mod:openai:acc-1:"));
        assert!(mod_key_1.ends_with(":gpt-4o"));
        assert!(mod_key_2.ends_with(":claude-3-5"));
        assert_ne!(acc_key, mod_key_1);
        assert_ne!(mod_key_1, mod_key_2);

        let candidate = ProviderCandidate {
            account: acc.clone(),
            resolved_model: "gpt-4o".to_string(),
        };
        assert_eq!(candidate.account_health_key(), acc_key);
        assert_eq!(candidate.model_health_key(), mod_key_1);
    }
}
