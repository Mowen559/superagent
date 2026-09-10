use std::sync::Arc;

use super::adapter::{
    CliConfigAdapter, ProbeResult,
    ResolvedSyncRequest, SyncError,
};
use super::transaction::ConfigTransactionManager;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncExecutionResult {
    pub status: String,
    pub cli: CliMetadata,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub model_source: String,
    pub effective_provider: Option<String>,
    pub effective_base_url: Option<String>,
    pub validation: ProbeResult,
    pub restart_required: bool,
    pub warnings: Vec<String>,
    pub transaction_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliMetadata {
    pub kind: String,
    pub version: String,
}

pub struct CliSyncCoordinator {
    adapter: Arc<dyn CliConfigAdapter>,
}

impl CliSyncCoordinator {
    pub fn new(adapter: Arc<dyn CliConfigAdapter>) -> Self {
        Self { adapter }
    }

    pub async fn plan_sync(
        &self,
        request: &ResolvedSyncRequest,
        hub_base_url: &str,
    ) -> Result<super::adapter::ConfigMutationPlan, SyncError> {
        let install = self.adapter.detect().await?;
        let current_config = self.adapter.inspect(&install).await?;

        if let Some(expected_rev) = &request.expected_revision {
            if *expected_rev != current_config.revision {
                return Err(SyncError::ConfigConflict);
            }
        }

        self.adapter.plan(&install, request, hub_base_url).await
    }

    pub async fn status_sync(&self) -> Result<super::adapter::EffectiveCliConfig, SyncError> {
        let install = self.adapter.detect().await?;
        self.adapter.inspect(&install).await
    }

    pub async fn execute_sync(
        &self,
        request: &ResolvedSyncRequest,
        hub_base_url: &str,
    ) -> Result<SyncExecutionResult, SyncError> {
        // 1. DETECT
        let install = self.adapter.detect().await?;

        // 2. CAPABILITIES & INSPECT
        let _caps = self.adapter.capabilities(&install).await?;
        let current_config = self.adapter.inspect(&install).await?;

        // 3. CONFLICT CHECK
        if let Some(expected_rev) = &request.expected_revision {
            if *expected_rev != current_config.revision {
                return Err(SyncError::ConfigConflict);
            }
        }

        // 4. PLAN
        let plan = self.adapter.plan(&install, request, hub_base_url).await?;

        // 5. VALIDATE
        self.adapter.validate_rendered(&plan)?;

        // Setup Transaction
        let mut tx = ConfigTransactionManager::new();
        let files: Vec<String> = plan.files_to_write.keys().cloned().collect();

        // 6. LOCK
        if let Err(e) = tx.lock(&files) {
            return Err(e);
        }


        // 7. SNAPSHOT
        if let Err(e) = tx.snapshot(&files) {
            return Err(e);
        }

        // Re-inspect to verify version under lock
        let locked_config = self.adapter.inspect(&install).await?;
        if let Some(expected_rev) = &request.expected_revision {
            if *expected_rev != locked_config.revision {
                return Err(SyncError::ConfigConflict);
            }
        }


        // 7. WRITE
        if let Err(write_e) = tx.commit(&plan.files_to_write) {
            if let Err(rb_e) = tx.rollback(Some(&plan.files_to_write)) {
                return Err(SyncError::RollbackFailed(format!("Original error: {}; Rollback error: {}", write_e, rb_e)));
            }
            return Err(write_e);
        }

        // 8. READ BACK
        let new_config = match self.adapter.inspect(&install).await {
            Ok(c) => c,
            Err(e) => {
                if let Err(rb_e) = tx.rollback(Some(&plan.files_to_write)) {
                    return Err(SyncError::RollbackFailed(format!("Readback error: {}; Rollback error: {}", e, rb_e)));
                }
                return Err(SyncError::ReadBackMismatch);
            }
        };

        // Validate read back matches what we expected loosely
        // Real implementation should probably be strict here depending on the adapter.

        // 9. PROBE
        let probe_result = match self.adapter.probe(&install, &new_config).await {
            Ok(p) => p,
            Err(e) => {
                if let Err(rb_e) = tx.rollback(Some(&plan.files_to_write)) {
                    return Err(SyncError::RollbackFailed(format!("Probe error: {}; Rollback error: {}", e, rb_e)));
                }
                return Err(e);
            }
        };

        // Save history before returning
        let transaction_id = format!("{}", chrono::Utc::now().timestamp_millis());
        if let Err(e) = tx.save_history(&self.adapter.kind().info().slug, &transaction_id) {
            // Warn but do not fail the sync if history save fails
            // In a real app we'd log this properly
            eprintln!("Failed to save history: {:?}", e);
        }

        // 10. COMMITTED
        Ok(SyncExecutionResult {
            status: "APPLIED".to_string(),
            cli: CliMetadata {
                kind: self.adapter.kind().info().slug.to_string(),
                version: install.version.clone(),
            },
            requested_model: request.model.default_model.clone(),
            effective_model: new_config.effective_model,
            model_source: new_config.model_source,
            effective_provider: new_config.effective_provider,
            effective_base_url: new_config.effective_base_url,
            validation: probe_result,
            restart_required: plan.requires_restart,
            warnings: vec![],
            transaction_id,
        })
    }
}
