use super::*;

    use futures::StreamExt;

    use super::*;

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
        let success_key = format!("proxy-stream-success-{}", uuid::Uuid::new_v4());
        health.reset(&success_key);
        assert_eq!(
            health.begin_attempt(&success_key),
            ProviderAdmission::Allowed
        );
        let success_stream =
            futures::stream::iter(vec![Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"))]);
        track_provider_stream(
            Box::pin(success_stream),
            success_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;
        let snapshot = health.snapshot(&success_key).unwrap();
        assert_eq!(snapshot.total_successes, 1);
        health.reset(&success_key);

        let incomplete_key = format!("proxy-stream-incomplete-{}", uuid::Uuid::new_v4());
        health.reset(&incomplete_key);
        assert_eq!(
            health.begin_attempt(&incomplete_key),
            ProviderAdmission::Allowed
        );
        let incomplete_stream = futures::stream::iter(vec![Ok(bytes::Bytes::from_static(
            b"data: {\"choices\":[]}\n\n",
        ))]);
        track_provider_stream(
            Box::pin(incomplete_stream),
            incomplete_key.clone(),
            Instant::now(),
        )
        .collect::<Vec<_>>()
        .await;
        let snapshot = health.snapshot(&incomplete_key).unwrap();
        assert_eq!(snapshot.total_recoverable_failures, 1);
        health.reset(&incomplete_key);
    }
