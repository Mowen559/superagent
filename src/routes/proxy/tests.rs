use super::*;
use std::time::Instant;
use futures::StreamExt;
use crate::domains::model_gateway::routing::ProviderAttemptOutcome;
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};

    #[test]
    fn detects_stream_success_and_error_terminals() {
        assert_eq!(
            canonical_stream_terminal_outcome(&bytes::Bytes::from_static(b"data: [DONE]\n\n")),
            Some(ProviderAttemptOutcome::Succeeded)
        );
        assert_eq!(
            canonical_stream_terminal_outcome(&bytes::Bytes::from_static(
                b"data: {\"error\":{\"message\":\"failed\"}}\n\n"
            )),
            Some(ProviderAttemptOutcome::Transient)
        );
    }

    #[tokio::test]
    async fn stream_tracker_releases_health_on_success_and_incomplete_end() {
        let health = ProviderHealthRuntime::global();
        let acc_key = format!("proxy-stream-acc-{}", uuid::Uuid::new_v4());
        let mod_key = format!("proxy-stream-mod-{}", uuid::Uuid::new_v4());
        health.reset(&acc_key);
        health.reset(&mod_key);
        assert_eq!(health.begin_attempt(&acc_key), ProviderAdmission::Allowed);
        assert_eq!(health.begin_attempt(&mod_key), ProviderAdmission::Allowed);

        let success_stream =
            futures::stream::iter(vec![Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"))]);
        track_provider_stream(
            Box::pin(success_stream),
            acc_key.clone(),
            mod_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;

        let acc_snapshot = health.snapshot(&acc_key).unwrap();
        assert_eq!(acc_snapshot.total_successes, 1);
        let mod_snapshot = health.snapshot(&mod_key).unwrap();
        assert_eq!(mod_snapshot.total_successes, 1);
        health.reset(&acc_key);
        health.reset(&mod_key);

        let acc_inc_key = format!("proxy-stream-acc-inc-{}", uuid::Uuid::new_v4());
        let mod_inc_key = format!("proxy-stream-mod-inc-{}", uuid::Uuid::new_v4());
        health.reset(&acc_inc_key);
        health.reset(&mod_inc_key);
        assert_eq!(health.begin_attempt(&acc_inc_key), ProviderAdmission::Allowed);
        assert_eq!(health.begin_attempt(&mod_inc_key), ProviderAdmission::Allowed);

        let incomplete_stream = futures::stream::iter(vec![Ok(bytes::Bytes::from_static(
            b"data: {\"choices\":[]}\n\n",
        ))]);
        track_provider_stream(
            Box::pin(incomplete_stream),
            acc_inc_key.clone(),
            mod_inc_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;

        // 模型级被记录为 recoverable failure
        let mod_inc_snapshot = health.snapshot(&mod_inc_key).unwrap();
        assert_eq!(mod_inc_snapshot.total_recoverable_failures, 1);
        // 账号级未被连坐失败
        let acc_inc_snapshot = health.snapshot(&acc_inc_key).unwrap();
        assert_eq!(acc_inc_snapshot.total_recoverable_failures, 0);

        health.reset(&acc_inc_key);
        health.reset(&mod_inc_key);
    }

    #[test]
    fn test_usage_from_value_extracts_reasoning_tokens() {
        use super::audit::usage_from_value;
        use serde_json::json;

        // OpenAI o1 / DeepSeek format
        let usage_json = json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "completion_tokens_details": {
                "reasoning_tokens": 50
            },
            "prompt_tokens_details": {
                "cached_tokens": 30
            }
        });

        let extracted = usage_from_value(Some(&usage_json));
        assert_eq!(extracted.prompt_tokens, 100);
        assert_eq!(extracted.completion_tokens, 80);
        assert_eq!(extracted.reasoning_tokens, 50);
        assert_eq!(extracted.cached_tokens, 30);

        // Kimi / Direct reasoning_tokens format
        let usage_direct = json!({
            "prompt_tokens": 40,
            "completion_tokens": 60,
            "reasoning_tokens": 25
        });
        let extracted_direct = usage_from_value(Some(&usage_direct));
        assert_eq!(extracted_direct.reasoning_tokens, 25);
    }

    #[tokio::test]
    async fn test_stream_audit_ttft_recording_single_event() {
        use super::audit::StreamAuditState;

        let mut audit = StreamAuditState::new(
            "test-acc".to_string(),
            "deepseek-chat".to_string(),
            "OpenAI".to_string(),
            "POST".to_string(),
            "/v1/chat/completions".to_string(),
            200,
            Instant::now(),
            "{}".to_string(),
            vec![],
        );

        assert!(audit.ttft_ms.is_none());
        audit.record_ttft();
        assert!(audit.ttft_ms.is_some());
        let first_ttft = audit.ttft_ms.unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        audit.record_ttft();
        assert_eq!(audit.ttft_ms, Some(first_ttft));
    }
