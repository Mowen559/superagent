use super::*;
use super::types::*;

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
