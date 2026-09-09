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
