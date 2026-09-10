use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::config::llm_settings::LLMSettings;

use crate::domains::model_gateway::routing::{ModelRequirements, ModelRouteRuntime};
use crate::domains::model_gateway::health::{ProviderCircuitState, ProviderHealthRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayEvalSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEvalFinding {
    pub code: String,
    pub severity: GatewayEvalSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProfileEval {
    pub profile: String,
    pub requirements: ModelRequirements,
    pub eligible_candidates: usize,
    pub verified_candidates: usize,
    pub unknown_candidates: usize,
    pub expired_candidates: usize,
    pub open_circuits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEvalReport {
    pub passed: bool,
    pub model: String,
    pub profiles: Vec<GatewayProfileEval>,
    pub findings: Vec<GatewayEvalFinding>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GatewayEvalRuntime;

impl GatewayEvalRuntime {
    pub fn evaluate_global(&self, settings: &LLMSettings) -> GatewayEvalReport {
        self.evaluate(settings, ProviderHealthRuntime::global())
    }

    pub fn evaluate(
        &self,
        settings: &LLMSettings,
        health: &ProviderHealthRuntime,
    ) -> GatewayEvalReport {
        let mut findings = Vec::new();
        self.evaluate_configuration(settings, &mut findings);

        let profile_specs = [
            ("basic", ModelRequirements::default(), true),
            (
                "agent_tools",
                ModelRequirements {
                    tools: true,
                    min_context_tokens: 8_000,
                    ..ModelRequirements::default()
                },
                true,
            ),
            (
                "vision",
                ModelRequirements {
                    vision: true,
                    ..ModelRequirements::default()
                },
                false,
            ),
            (
                "streaming",
                ModelRequirements {
                    streaming: true,
                    ..ModelRequirements::default()
                },
                false,
            ),
            (
                "long_context_32k",
                ModelRequirements {
                    min_context_tokens: 32_000,
                    ..ModelRequirements::default()
                },
                false,
            ),
        ];
        let router = ModelRouteRuntime::default();
        let mut profiles = Vec::with_capacity(profile_specs.len());

        for (profile, requirements, required_for_agent) in profile_specs {
            let route = router.plan_with_requirements(settings, &settings.model, &[], requirements);
            let mut verified_candidates = 0;
            let mut unknown_candidates = 0;
            let mut expired_candidates = 0;
            let mut open_circuits = 0;
            for candidate in &route.candidates {
                let assessment =
                    crate::domains::model_gateway::routing::capability_assessment(&candidate.account, &settings.model, requirements);
                match assessment.evidence {
                    crate::domains::model_gateway::routing::ModelCapabilityEvidence::Verified => verified_candidates += 1,
                    crate::domains::model_gateway::routing::ModelCapabilityEvidence::Unknown => unknown_candidates += 1,
                    crate::domains::model_gateway::routing::ModelCapabilityEvidence::Unsupported => {}
                }
                if assessment.expired {
                    expired_candidates += 1;
                }
                let is_circuit_open = health
                    .snapshot(&candidate.health_key())
                    .is_some_and(|snapshot| snapshot.circuit != ProviderCircuitState::Closed)
                    || health
                        .snapshot(&candidate.account_health_key())
                        .is_some_and(|snapshot| snapshot.circuit != ProviderCircuitState::Closed);
                if is_circuit_open {
                    open_circuits += 1;
                }
            }

            if route.candidates.is_empty() {
                findings.push(GatewayEvalFinding {
                    code: format!("missing_{profile}_route"),
                    severity: if required_for_agent {
                        GatewayEvalSeverity::Error
                    } else {
                        GatewayEvalSeverity::Warning
                    },
                    message: format!(
                        "model '{}' has no Provider Route for the {profile} profile",
                        settings.model
                    ),
                    account_id: None,
                });
            } else if verified_candidates == 0 && unknown_candidates > 0 {
                findings.push(GatewayEvalFinding {
                    code: format!("unverified_{profile}_capability"),
                    severity: GatewayEvalSeverity::Info,
                    message: format!(
                        "the {profile} route is eligible only through Unknown capability metadata"
                    ),
                    account_id: None,
                });
            }
            if expired_candidates > 0 {
                findings.push(GatewayEvalFinding {
                    code: format!("expired_{profile}_capability"),
                    severity: GatewayEvalSeverity::Info,
                    message: format!(
                        "{expired_candidates} {profile} route candidate(s) use expired capability evidence and require controlled recertification"
                    ),
                    account_id: None,
                });
            }
            if open_circuits == route.candidates.len() && !route.candidates.is_empty() {
                findings.push(GatewayEvalFinding {
                    code: format!("all_{profile}_circuits_open"),
                    severity: GatewayEvalSeverity::Warning,
                    message: format!("all {profile} Provider Route candidates have open circuits"),
                    account_id: None,
                });
            }

            profiles.push(GatewayProfileEval {
                profile: profile.to_string(),
                requirements,
                eligible_candidates: route.candidates.len(),
                verified_candidates,
                unknown_candidates,
                expired_candidates,
                open_circuits,
            });
        }

        let passed = !findings
            .iter()
            .any(|finding| finding.severity == GatewayEvalSeverity::Error);
        GatewayEvalReport {
            passed,
            model: settings.model.clone(),
            profiles,
            findings,
        }
    }

    fn evaluate_configuration(
        &self,
        settings: &LLMSettings,
        findings: &mut Vec<GatewayEvalFinding>,
    ) {
        if !settings.enabled {
            findings.push(GatewayEvalFinding {
                code: "gateway_disabled".to_string(),
                severity: GatewayEvalSeverity::Error,
                message: "Canonical LLM Gateway is disabled".to_string(),
                account_id: None,
            });
        }

        let mut ids = HashSet::new();
        for account in settings.accounts.iter().filter(|account| account.enabled) {
            if !ids.insert(account.id.as_str()) {
                findings.push(GatewayEvalFinding {
                    code: "duplicate_account_id".to_string(),
                    severity: GatewayEvalSeverity::Error,
                    message: format!("enabled account id '{}' is duplicated", account.id),
                    account_id: Some(account.id.clone()),
                });
            }
            if account.base_url.trim().is_empty() {
                findings.push(GatewayEvalFinding {
                    code: "missing_base_url".to_string(),
                    severity: GatewayEvalSeverity::Error,
                    message: "enabled Provider Account has no base URL".to_string(),
                    account_id: Some(account.id.clone()),
                });
            }
            if account.api_key.trim().is_empty() {
                findings.push(GatewayEvalFinding {
                    code: "credential_not_materialized".to_string(),
                    severity: GatewayEvalSeverity::Warning,
                    message: "Provider Account has no materialized API key; an OAuth Adapter must supply one at runtime".to_string(),
                    account_id: Some(account.id.clone()),
                });
            }
            for (alias, target) in &account.model_mapping {
                if alias.trim().is_empty() || target.trim().is_empty() {
                    findings.push(GatewayEvalFinding {
                        code: "invalid_model_mapping".to_string(),
                        severity: GatewayEvalSeverity::Error,
                        message: "model mapping alias and target must both be non-empty"
                            .to_string(),
                        account_id: Some(account.id.clone()),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use serde_json::json;

    use crate::config::llm_settings::LLMAccount;

    use super::*;
    use crate::domains::model_gateway::{
        routing::{provider_health_key, ProviderAttemptOutcome},
        health::ProviderHealthPolicy,
    };

    fn account(id: &str, supports_tools: bool) -> LLMAccount {
        LLMAccount {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            provider: "openai".to_string(),
            models: vec!["agent-model".to_string()],
            models_info: Some(HashMap::from([(
                "agent-model".to_string(),
                json!({
                    "supportsTools": supports_tools,
                    "supportsVision": true,
                    "supportsStreaming": true,
                    "context_window": 64000
                }),
            )])),
            base_url: "https://example.test/v1".to_string(),
            api_key: "secret".to_string(),
            ..LLMAccount::default()
        }
    }

    fn settings(accounts: Vec<LLMAccount>) -> LLMSettings {
        LLMSettings {
            accounts,
            active_account_id: None,
            provider: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            model: "agent-model".to_string(),
            ..LLMSettings::default()
        }
    }

    #[test]
    fn eval_detects_duplicate_ids_and_missing_required_tool_route() {
        let settings = settings(vec![account("same", false), account("same", false)]);
        let report = GatewayEvalRuntime.evaluate(&settings, &ProviderHealthRuntime::default());
        assert!(!report.passed);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_account_id"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "missing_agent_tools_route"));
    }

    #[test]
    fn eval_distinguishes_verified_routes_and_open_circuits() {
        let account = account("healthy", true);
        let settings = settings(vec![account.clone()]);
        let health = ProviderHealthRuntime::with_policy(ProviderHealthPolicy {
            failure_threshold: 1,
            base_cooldown: Duration::from_secs(30),
            max_cooldown: Duration::from_secs(30),
            max_entries: 8,
        });
        health.finish_attempt(
            &provider_health_key(&account),
            ProviderAttemptOutcome::Authentication,
            1,
        );

        let report = GatewayEvalRuntime.evaluate(&settings, &health);
        assert!(report.passed);
        let tools = report
            .profiles
            .iter()
            .find(|profile| profile.profile == "agent_tools")
            .unwrap();
        assert_eq!(tools.verified_candidates, 1);
        assert_eq!(tools.open_circuits, 1);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "all_agent_tools_circuits_open"));
    }

    #[test]
    fn eval_reports_expired_capability_as_unknown_recertification_work() {
        let mut stale = account("stale", true);
        stale.models_info = Some(HashMap::from([(
            "agent-model".to_string(),
            json!({
                "capabilities": {"tools": true},
                "context_window": 64000,
                "capabilityEvidence": {
                    "tools": {
                        "source": "probe",
                        "outcome": "verified",
                        "observedAt": "2020-01-01T00:00:00Z",
                        "expiresAt": "2020-01-08T00:00:00Z"
                    }
                }
            }),
        )]));
        let report =
            GatewayEvalRuntime.evaluate(&settings(vec![stale]), &ProviderHealthRuntime::default());
        let tools = report
            .profiles
            .iter()
            .find(|profile| profile.profile == "agent_tools")
            .unwrap();
        assert_eq!(tools.verified_candidates, 0);
        assert_eq!(tools.unknown_candidates, 1);
        assert_eq!(tools.expired_candidates, 1);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "expired_agent_tools_capability"));
    }
}
