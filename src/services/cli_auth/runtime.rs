use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use crate::config::llm_settings::LLMAccount;
use crate::config::llm_settings;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Token expired and no refresh token available")]
    TokenExpired,
    #[error("Failed to refresh token: {0}")]
    RefreshFailed(String),
}

pub struct AuthRuntime {
    refresh_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl AuthRuntime {
    pub fn global() -> &'static Self {
        static INSTANCE: std::sync::OnceLock<AuthRuntime> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| AuthRuntime {
            refresh_locks: Mutex::new(HashMap::new()),
        })
    }

    pub async fn get_valid_token(&self, account: &mut LLMAccount, http_client: &reqwest::Client) -> Result<String, AuthError> {
        let needs_refresh = {
            if let Some(expires_at) = account.expires_at {
                chrono::Utc::now().timestamp() + 300 >= expires_at
            } else {
                false
            }
        };

        if !needs_refresh {
            return Ok(account.api_key.clone());
        }

        let lock = {
            let mut locks = self.refresh_locks.lock().await;
            locks.entry(account.id.clone()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
        };

        let _guard = lock.lock().await;
        // Check again after acquiring lock
        if let Some(expires_at) = account.expires_at {
            if chrono::Utc::now().timestamp() + 300 < expires_at {
                return Ok(account.api_key.clone()); 
            }
        }

        if account.refresh_token.is_none() {
            return Err(AuthError::TokenExpired);
        }

        let refreshed = crate::services::cli_auth::refresh_account_token_if_needed(account, http_client).await;
        
        if refreshed {
            // Save it
            let account_id = account.id.clone();
            let new_key = account.api_key.clone();
            let new_refresh = account.refresh_token.clone();
            let new_expires = account.expires_at;
            let _ = llm_settings::update_llm_settings(move |settings| {
                for a in &mut settings.accounts {
                    if a.id == account_id {
                        a.api_key = new_key.clone();
                        a.refresh_token = new_refresh.clone();
                        a.expires_at = new_expires;
                        break;
                    }
                }
                Ok(())
            });
            Ok(account.api_key.clone())
        } else {
            Err(AuthError::RefreshFailed("Failed to refresh token upstream".into()))
        }
    }
}
