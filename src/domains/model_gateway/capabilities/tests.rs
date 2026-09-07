use std::collections::HashMap;
use std::time::Duration;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::config::llm_settings::{LLMAccount, LLMSettings};
use super::policy::*;
use super::types::*;
use super::discovery::*;
use super::assessment::*;
use super::catalog::*;
use super::probe::*;
use super::utils::*;
use super::ProviderCapabilityRuntime;

    pub(crate) fn account() -> LLMAccount {
        LLMAccount {
            id: "account-1".to_string(),
            name: "Account".to_string(),
            provider: "openai".to_string(),
            base_url: "https://provider.test".to_string(),
            api_key: "secret".to_string(),
            models: vec!["existing".to_string()],
            ..LLMAccount::default()
        }
    }

    #[test]
    pub(crate) fn catalog_is_bounded_sanitized_and_preserves_explicit_evidence() {
        let account = account();
        let catalog = normalize_catalog(
            &account,
            json!({
                "data": [{
                    "id": "model-a",
                    "supportsTools": true,
                    "supportsVision": false,
                    "secretField": "must-not-survive",
                    "context_window": 32000
                }]
            }),
            CapabilityEvidencePolicy::default(),
        )
        .unwrap();

        assert_eq!(catalog.snapshot.models, vec!["model-a"]);
        assert!(catalog.metadata["model-a"].get("secretField").is_none());
        assert!(catalog.observations.iter().any(|observation| {
            observation.profile == CapabilityProfile::Tools
                && observation.outcome == CapabilityObservationOutcome::Verified
        }));
        assert!(catalog.observations.iter().any(|observation| {
            observation.profile == CapabilityProfile::Vision
                && observation.outcome == CapabilityObservationOutcome::Unsupported
        }));
    }

    #[test]
    pub(crate) fn applying_report_merges_models_and_never_persists_failed_downgrades() {
        let mut account = account();
        account.models_info = Some(HashMap::from([(
            "model-a".to_string(),
            json!({"capabilities": {"tools": true}}),
        )]));
        let mut settings = LLMSettings {
            accounts: vec![account],
            ..LLMSettings::default()
        };
        let report = CapabilityDiscoveryReport {
            account_id: "account-1".to_string(),
            provider: "openai".to_string(),
            catalog: Some(ModelCatalogSnapshot {
                account_id: "account-1".to_string(),
                provider: "openai".to_string(),
                observed_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: "2026-01-02T00:00:00Z".to_string(),
                models: vec!["model-a".to_string()],
                truncated: false,
            }),
            observations: vec![CapabilityObservation {
                account_id: "account-1".to_string(),
                model: "model-a".to_string(),
                profile: CapabilityProfile::Tools,
                source: CapabilityEvidenceSource::Probe,
                outcome: CapabilityObservationOutcome::Failed,
                observed_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: "2026-01-08T00:00:00Z".to_string(),
                detail: "transient".to_string(),
                status_code: Some(503),
            }],
            probe_requests: 1,
            recertification_plan: None,
            catalog_metadata: HashMap::from([(
                "model-a".to_string(),
                json!({"id": "model-a", "context_window": 32000}),
            )]),
        };

        apply_discovery_report(&mut settings, &report).unwrap();
        let info = &settings.accounts[0].models_info.as_ref().unwrap()["model-a"];
        assert_eq!(
            info.pointer("/capabilities/tools"),
            Some(&Value::Bool(true))
        );
        assert_eq!(info["context_window"], 32000);
        assert!(settings.accounts[0]
            .models
            .contains(&"existing".to_string()));
    }

    #[test]
    pub(crate) fn request_requires_explicit_network_confirmation_and_bounded_probe_product() {
        let request = CapabilityDiscoveryRequest {
            account_id: "account-1".to_string(),
            models: vec!["m1".to_string()],
            profiles: vec![CapabilityProfile::Basic],
            include_catalog: false,
            run_probes: true,
            confirm_network: false,
            revalidate_expired: false,
        };
        assert_eq!(
            validate_request(request).unwrap_err().kind,
            CapabilityDiscoveryErrorKind::InvalidRequest
        );

        let request = CapabilityDiscoveryRequest {
            account_id: "account-1".to_string(),
            models: vec!["m1".into(), "m2".into(), "m3".into()],
            profiles: vec![
                CapabilityProfile::Basic,
                CapabilityProfile::Tools,
                CapabilityProfile::Vision,
            ],
            include_catalog: false,
            run_probes: true,
            confirm_network: true,
            revalidate_expired: false,
        };
        assert_eq!(
            validate_request(request).unwrap_err().kind,
            CapabilityDiscoveryErrorKind::InvalidRequest
        );
    }

    #[test]
    pub(crate) fn evidence_expires_to_unknown_at_the_exact_ttl_boundary() {
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let policy = CapabilityEvidencePolicy {
            catalog_ttl: ChronoDuration::hours(24),
            probe_ttl: ChronoDuration::hours(168),
        };
        let info = json!({
            "capabilities": {"tools": false},
            "capabilityEvidence": {
                "tools": {
                    "source": "probe",
                    "outcome": "unsupported",
                    "observedAt": "2026-08-15T00:00:00Z",
                    "expiresAt": "2026-08-22T00:00:00Z"
                }
            }
        });

        let fresh = assess_model_profile_at(
            &info,
            CapabilityProfile::Tools,
            now - ChronoDuration::seconds(1),
            policy,
        );
        assert_eq!(fresh.verdict, CapabilityVerdict::Unsupported);
        assert_eq!(fresh.freshness, CapabilityFreshness::Fresh);

        let expired = assess_model_profile_at(&info, CapabilityProfile::Tools, now, policy);
        assert_eq!(expired.verdict, CapabilityVerdict::Unknown);
        assert_eq!(expired.freshness, CapabilityFreshness::Expired);
    }

    #[test]
    pub(crate) fn invalid_timestamps_fail_closed_to_unknown_and_legacy_metadata_stays_unversioned() {
        let policy = CapabilityEvidencePolicy::default();
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let invalid = json!({
            "capabilities": {"vision": true},
            "capabilityEvidence": {
                "vision": {
                    "source": "catalog",
                    "outcome": "verified",
                    "observedAt": "not-a-time"
                }
            }
        });
        let assessment = assess_model_profile_at(&invalid, CapabilityProfile::Vision, now, policy);
        assert_eq!(assessment.verdict, CapabilityVerdict::Unknown);
        assert_eq!(assessment.freshness, CapabilityFreshness::Invalid);

        let legacy = assess_model_profile_at(
            &json!({"supportsVision": true}),
            CapabilityProfile::Vision,
            now,
            policy,
        );
        assert_eq!(legacy.verdict, CapabilityVerdict::Verified);
        assert_eq!(legacy.freshness, CapabilityFreshness::Unversioned);
    }

    #[test]
    pub(crate) fn recertification_plan_is_deterministic_bounded_and_expired_only() {
        let now = parse_timestamp("2026-08-22T00:00:00Z").unwrap();
        let policy = CapabilityEvidencePolicy {
            catalog_ttl: ChronoDuration::hours(24),
            probe_ttl: ChronoDuration::hours(168),
        };
        let mut account = account();
        account.models.clear();
        let mut models_info = HashMap::new();
        for index in 0..5 {
            let model = format!("model-{index}");
            account.models.push(model.clone());
            models_info.insert(
                model,
                json!({
                    "capabilities": {"tools": true, "vision": true},
                    "capabilityEvidence": {
                        "tools": {
                            "source": "probe", "outcome": "verified",
                            "observedAt": "2026-08-01T00:00:00Z",
                            "expiresAt": "2026-08-08T00:00:00Z"
                        },
                        "vision": {
                            "source": "catalog", "outcome": "verified",
                            "observedAt": "2026-08-01T00:00:00Z",
                            "expiresAt": "2026-08-02T00:00:00Z"
                        },
                        "streaming": {
                            "source": "probe", "outcome": "verified",
                            "observedAt": "2026-08-21T00:00:00Z",
                            "expiresAt": "2026-08-28T00:00:00Z"
                        }
                    }
                }),
            );
        }
        account.models_info = Some(models_info);

        let plan = plan_recertification_for_account_at(&account, now, policy);
        assert_eq!(plan.probe_requests, MAX_PROBE_REQUESTS);
        assert_eq!(plan.items.len(), MAX_SELECTED_MODELS);
        assert!(plan.truncated);
        assert_eq!(plan.items[0].model, "model-0");
        assert!(plan.items.iter().all(|item| {
            item.profiles == vec![CapabilityProfile::Tools, CapabilityProfile::Vision]
        }));
    }

    #[tokio::test]
    pub(crate) async fn controlled_recertification_executes_only_the_expired_plan() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["tool_choice"], "required");
                Json(json!({
                    "choices": [{"message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1", "type": "function",
                            "function": {"name": "hub_capability_probe", "arguments": "{\"value\":\"OK\"}"}
                        }]
                    }}]
                }))
            }),
        );
        let mut configured = account();
        configured.base_url = spawn_test_server(app).await;
        configured.models = vec!["probe-model".to_string()];
        configured.models_info = Some(HashMap::from([(
            "probe-model".to_string(),
            json!({
                "capabilities": {"tools": true, "streaming": true},
                "capabilityEvidence": {
                    "tools": {
                        "source": "probe", "outcome": "verified",
                        "observedAt": "2020-01-01T00:00:00Z",
                        "expiresAt": "2020-01-08T00:00:00Z"
                    },
                    "streaming": {
                        "source": "probe", "outcome": "verified",
                        "observedAt": "2099-01-01T00:00:00Z",
                        "expiresAt": "2099-01-08T00:00:00Z"
                    }
                }
            }),
        )]));
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::with_policy(
            reqwest::Client::new(),
            CapabilityEvidencePolicy::default(),
        )
        .discover(
            &settings,
            CapabilityDiscoveryRequest {
                account_id: "account-1".to_string(),
                models: Vec::new(),
                profiles: Vec::new(),
                include_catalog: false,
                run_probes: true,
                confirm_network: true,
                revalidate_expired: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.probe_requests, 1);
        let plan = report.recertification_plan.unwrap();
        assert_eq!(plan.items[0].profiles, vec![CapabilityProfile::Tools]);
        assert_eq!(
            report.observations[0].outcome,
            CapabilityObservationOutcome::Verified
        );
    }

    #[tokio::test]
    pub(crate) async fn discovery_calls_only_the_configured_catalog_and_redacts_credentials() {
        let app = Router::new().route(
            "/v1/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(
                    headers
                        .get(AUTHORIZATION)
                        .and_then(|value| value.to_str().ok()),
                    Some("Bearer secret")
                );
                Json(json!({
                    "data": [{"id": "discovered-model", "supportsTools": true}]
                }))
            }),
        );
        let base_url = spawn_test_server(app).await;
        let mut configured = account();
        configured.base_url = base_url;
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::new(reqwest::Client::new())
            .discover(
                &settings,
                CapabilityDiscoveryRequest {
                    account_id: "account-1".to_string(),
                    models: Vec::new(),
                    profiles: Vec::new(),
                    include_catalog: true,
                    run_probes: false,
                    confirm_network: true,
                    revalidate_expired: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            report.catalog.as_ref().unwrap().models,
            vec!["discovered-model"]
        );
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains(&settings.accounts[0].base_url));
    }

    #[tokio::test]
    pub(crate) async fn fixed_tools_and_streaming_probes_require_canonical_evidence() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                if body.get("stream").and_then(Value::as_bool) == Some(true) {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from(
                            "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\n",
                        ))
                        .unwrap()
                } else {
                    assert_eq!(body["max_tokens"], PROBE_MAX_TOKENS);
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&json!({
                                "choices": [{
                                    "message": {
                                        "role": "assistant",
                                        "content": null,
                                        "tool_calls": [{
                                            "id": "call-1",
                                            "type": "function",
                                            "function": {
                                                "name": "hub_capability_probe",
                                                "arguments": "{\"value\":\"OK\"}"
                                            }
                                        }]
                                    }
                                }]
                            }))
                            .unwrap(),
                        ))
                        .unwrap()
                }
            }),
        );
        let base_url = spawn_test_server(app).await;
        let mut configured = account();
        configured.base_url = base_url;
        configured.models = vec!["probe-model".to_string()];
        let settings = LLMSettings {
            accounts: vec![configured],
            ..LLMSettings::default()
        };
        let report = ProviderCapabilityRuntime::new(reqwest::Client::new())
            .discover(
                &settings,
                CapabilityDiscoveryRequest {
                    account_id: "account-1".to_string(),
                    models: vec!["probe-model".to_string()],
                    profiles: vec![CapabilityProfile::Tools, CapabilityProfile::Streaming],
                    include_catalog: false,
                    run_probes: true,
                    confirm_network: true,
                    revalidate_expired: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(report.probe_requests, 2);
        assert!(report
            .observations
            .iter()
            .all(|observation| { observation.outcome == CapabilityObservationOutcome::Verified }));
    }

    pub(crate) async fn spawn_test_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }
