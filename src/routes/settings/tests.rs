use super::*;

mod tests {
    use super::*;

    #[test]
    fn capability_discovery_requires_local_control_plane_header() {
        let error = require_capability_discovery_header(&HeaderMap::new()).unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-capability-discovery", "confirm".parse().unwrap());
        assert!(require_capability_discovery_header(&headers).is_ok());
    }

    #[test]
    fn settings_responses_redact_account_credentials_and_round_trip_masks() {
        let current = llm_settings::LLMSettings {
            api_key: "top-level-secret".to_string(),
            accounts: vec![llm_settings::LLMAccount {
                id: "account-1".to_string(),
                api_key: "account-secret".to_string(),
                refresh_token: Some("refresh-secret".to_string()),
                ..llm_settings::LLMAccount::default()
            }],
            ..llm_settings::LLMSettings::default()
        };
        let mut redacted = redacted_settings_value(current.clone());
        assert!(!redacted.to_string().contains("account-secret"));
        assert!(!redacted.to_string().contains("refresh-secret"));

        restore_redacted_secrets(&mut redacted, &current);
        assert_eq!(redacted["accounts"][0]["apiKey"], "account-secret");
        assert_eq!(redacted["accounts"][0]["refreshToken"], "refresh-secret");
    }
}
