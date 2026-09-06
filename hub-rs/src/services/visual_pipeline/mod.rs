mod builtin;
#[cfg(feature = "win_native_cropper")]
pub mod native_cropper;
#[cfg(feature = "win_native_cropper")]
pub mod cropper;
pub mod pin_window;
pub mod record_manager;
pub mod ffmpeg_engine;
mod external;
mod settings;
pub mod types;
pub mod tts;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::config::app_paths::AppPaths;

#[cfg(feature = "vision-paddleocr")]
use self::builtin::PaddleOcrProvider;
#[cfg(feature = "vision-xcap")]
use self::builtin::XcapScreenshotProvider;
use self::builtin::{
    DeepLTranslationProvider, GoogleTranslationProvider, HubLlmTranslationProvider,
};
use self::types::{
    CaptureRequest, OcrProvider, OcrReceipt, OcrRequest, PipelineRequest, ProviderCatalog,
    ScreenshotProvider, StageReceipt, TranslationInput, TranslationProvider, TranslationReceipt,
    TranslationRequest, VisualArtifact, VisualPipelineReceipt, VisualSettings,
};

const MAX_ARTIFACTS: usize = 64;
const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;
const ARTIFACT_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
const MAX_TRANSLATION_CHARS: usize = 20_000;

struct ProviderRegistry {
    screenshots: HashMap<String, Arc<dyn ScreenshotProvider>>,
    ocr: HashMap<String, Arc<dyn OcrProvider>>,
    translations: HashMap<String, Arc<dyn TranslationProvider>>,
    warnings: Vec<String>,
}

impl ProviderRegistry {
    fn build(plugin_dir: &Path, client: reqwest::Client) -> Self {
        #[cfg(feature = "vision-snowcapture")]
        use self::builtin::SnowCaptureScreenshotProvider;
        #[cfg(feature = "vision-rapidocr")]
        use self::builtin::RapidOcrProvider;

        let mut registry = Self {
            screenshots: HashMap::new(),
            ocr: HashMap::new(),
            translations: HashMap::new(),
            warnings: vec![],
        };
        #[cfg(feature = "vision-xcap")]
        registry
            .screenshots
            .insert("xcap".to_string(), Arc::new(XcapScreenshotProvider));
        #[cfg(feature = "vision-snowcapture")]
        registry.screenshots.insert(
            "snow-capture".to_string(),
            Arc::new(SnowCaptureScreenshotProvider),
        );
        #[cfg(feature = "vision-paddleocr")]
        registry.ocr.insert(
            "paddleocr-rs".to_string(),
            Arc::new(PaddleOcrProvider::new()),
        );
        #[cfg(feature = "vision-rapidocr")]
        registry.ocr.insert(
            "rapid-ocr".to_string(),
            Arc::new(RapidOcrProvider::new()),
        );
        registry.translations.insert(
            "hub-llm".to_string(),
            Arc::new(HubLlmTranslationProvider::new(client.clone())),
        );
        registry.translations.insert(
            "deepl".to_string(),
            Arc::new(DeepLTranslationProvider::new(client.clone())),
        );
        registry.translations.insert(
            "google-translate".to_string(),
            Arc::new(GoogleTranslationProvider::new(client)),
        );

        let external = external::load(plugin_dir);
        registry.warnings.extend(external.warnings);
        for provider in external.screenshots {
            let id = provider.0.id.clone();
            if registry.screenshots.contains_key(&id) {
                registry
                    .warnings
                    .push(format!("ignored duplicate screenshot provider id: {id}"));
            } else {
                registry.screenshots.insert(id, Arc::new(provider));
            }
        }
        for provider in external.ocr {
            let id = provider.0.id.clone();
            if registry.ocr.contains_key(&id) {
                registry
                    .warnings
                    .push(format!("ignored duplicate OCR provider id: {id}"));
            } else {
                registry.ocr.insert(id, Arc::new(provider));
            }
        }
        for provider in external.translations {
            let id = provider.0.id.clone();
            if registry.translations.contains_key(&id) {
                registry
                    .warnings
                    .push(format!("ignored duplicate translation provider id: {id}"));
            } else {
                registry.translations.insert(id, Arc::new(provider));
            }
        }
        registry
    }
}

pub struct VisualPipelineRuntime {
    settings_path: PathBuf,
    artifact_dir: PathBuf,
    plugin_dir: PathBuf,
    client: reqwest::Client,
    settings: RwLock<VisualSettings>,
    registry: RwLock<ProviderRegistry>,
}

impl VisualPipelineRuntime {
    pub fn new(app_paths: &AppPaths, client: reqwest::Client) -> Self {
        let visual_dir = app_paths.ensure_data_dir(&["vision"]);
        let artifact_dir = app_paths.ensure_data_dir(&["vision", "artifacts"]);
        let plugin_dir = app_paths.ensure_data_dir(&["vision", "plugins"]);
        let settings_path = visual_dir.join("settings.json");
        let current_settings = settings::read(&settings_path);
        if !settings_path.exists() {
            if let Err(error) = settings::write_atomic(&settings_path, &current_settings) {
                tracing::warn!("failed to initialize visual settings: {error}");
            }
        }
        let registry = ProviderRegistry::build(&plugin_dir, client.clone());
        Self {
            settings_path,
            artifact_dir,
            plugin_dir,
            client,
            settings: RwLock::new(current_settings),
            registry: RwLock::new(registry),
        }
    }

    pub fn settings(&self) -> VisualSettings {
        self.settings
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn providers(&self) -> ProviderCatalog {
        let settings = self.settings();
        let registry = self
            .registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut screenshot = registry
            .screenshots
            .values()
            .map(|provider| provider.descriptor(&settings))
            .collect::<Vec<_>>();
        let mut ocr = registry
            .ocr
            .values()
            .map(|provider| provider.descriptor(&settings))
            .collect::<Vec<_>>();
        let mut translation = registry
            .translations
            .values()
            .map(|provider| provider.descriptor(&settings))
            .collect::<Vec<_>>();
        screenshot.sort_by(|left, right| left.id.cmp(&right.id));
        ocr.sort_by(|left, right| left.id.cmp(&right.id));
        translation.sort_by(|left, right| left.id.cmp(&right.id));
        ProviderCatalog {
            screenshot,
            ocr,
            translation,
            load_warnings: registry.warnings.clone(),
        }
    }

    pub fn update_settings(&self, mut next: VisualSettings) -> Result<VisualSettings, String> {
        next.schema_version = 1;
        normalize_settings(&mut next)?;
        {
            let registry = self
                .registry
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !registry.screenshots.contains_key(&next.screenshot_provider) {
                return Err(format!(
                    "unknown screenshot provider: {}",
                    next.screenshot_provider
                ));
            }
            if !registry.ocr.contains_key(&next.ocr_provider) {
                return Err(format!("unknown OCR provider: {}", next.ocr_provider));
            }
            if !registry
                .translations
                .contains_key(&next.translation_provider)
            {
                return Err(format!(
                    "unknown translation provider: {}",
                    next.translation_provider
                ));
            }
        }
        settings::write_atomic(&self.settings_path, &next)?;
        *self
            .settings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next.clone();
        Ok(next)
    }

    pub fn reload_providers(&self) -> ProviderCatalog {
        *self
            .registry
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            ProviderRegistry::build(&self.plugin_dir, self.client.clone());
        self.providers()
    }

    pub async fn monitors(
        &self,
        provider_id: Option<&str>,
    ) -> Result<Vec<types::MonitorDescriptor>, String> {
        let settings = self.settings();
        let id = provider_id.unwrap_or(&settings.screenshot_provider);
        let provider = self.screenshot_provider(id)?;
        ensure_ready(&provider.descriptor(&settings))?;
        provider.monitors().await
    }

    pub async fn interactive_capture(self: std::sync::Arc<Self>) -> Result<Option<(VisualArtifact, String, i32, i32, i32, i32)>, String> {
        self.cleanup_artifacts();
        let vp = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("SnowCropperUI".into())
            .spawn(move || {
                let res = crate::services::visual_pipeline::native_cropper::run_interactive_capture(vp);
                let _ = tx.send(res);
            })
            .map_err(|e| format!("Failed to spawn cropper thread: {e}"))?;

        let result = rx.await.map_err(|e| format!("Cropper thread dropped: {e}"))??;
        
        let (bytes, action_str, x, y, crop_w, crop_h) = match result {
            Some(res) => res,
            None => return Ok(None),
        };
            let id = uuid::Uuid::new_v4().to_string();
            let output_path = self.artifact_dir.join(format!("{id}.png"));
            tokio::fs::write(&output_path, &bytes).await
                .map_err(|e| format!("Failed to write interactive screenshot: {}", e))?;
                
            let img = image::load_from_memory(&bytes).unwrap();
            let width = img.width();
            let height = img.height();

            Ok(Some((VisualArtifact {
                id: id.clone(),
                provider_id: "native".to_string(),
                mime_type: "image/png".to_string(),
                width,
                height,
                created_at: chrono::Utc::now().to_rfc3339(),
                url: format!("/api/vision/artifacts/{id}"),
            }, action_str, x, y, crop_w, crop_h))
        )
    }

    pub async fn capture(&self, request: CaptureRequest) -> Result<VisualArtifact, String> {
        let settings = self.settings();
        let provider_id = request
            .provider_id
            .as_deref()
            .unwrap_or(&settings.screenshot_provider)
            .to_string();
        let provider = self.screenshot_provider(&provider_id)?;
        ensure_ready(&provider.descriptor(&settings))?;
        self.cleanup_artifacts();
        let id = uuid::Uuid::new_v4().to_string();
        let output_path = self.artifact_dir.join(format!("{id}.png"));
        let output = provider
            .capture(request.monitor_id.as_deref(), &output_path)
            .await?;
        let metadata = std::fs::metadata(&output_path)
            .map_err(|error| format!("screenshot artifact was not created: {error}"))?;
        if metadata.len() == 0 || metadata.len() > MAX_ARTIFACT_BYTES {
            let _ = std::fs::remove_file(&output_path);
            return Err("screenshot artifact is empty or exceeds 32 MiB".to_string());
        }
        Ok(VisualArtifact {
            id: id.clone(),
            provider_id,
            mime_type: "image/png".to_string(),
            width: output.width,
            height: output.height,
            created_at: chrono::Utc::now().to_rfc3339(),
            url: format!("/api/vision/artifacts/{id}"),
        })
    }

    pub async fn ocr(&self, request: OcrRequest) -> Result<OcrReceipt, String> {
        let settings = self.settings();
        let provider_id = request
            .provider_id
            .as_deref()
            .unwrap_or(&settings.ocr_provider)
            .to_string();
        let provider = self.ocr_provider(&provider_id)?;
        ensure_ready(&provider.descriptor(&settings))?;
        let artifact_path = self.artifact_path(&request.artifact_id)?;
        let started = Instant::now();
        let output = provider.recognize(&artifact_path, &settings).await?;
        Ok(OcrReceipt {
            provider_id,
            artifact_id: request.artifact_id,
            text: output.text,
            blocks: output.blocks,
            duration_ms: elapsed_ms(started),
        })
    }

    pub async fn translate(
        &self,
        request: TranslationRequest,
    ) -> Result<TranslationReceipt, String> {
        validate_translation(
            &request.text,
            &request.target_language,
            request.source_language.as_deref(),
            request.model.as_deref(),
        )?;
        let settings = self.settings();
        let provider_id = request
            .provider_id
            .as_deref()
            .unwrap_or(&settings.translation_provider)
            .to_string();
        let provider = self.translation_provider(&provider_id)?;
        ensure_ready(&provider.descriptor(&settings))?;
        let input = TranslationInput {
            text: request.text,
            source_language: request.source_language,
            target_language: request.target_language,
            model: request.model.filter(|s| !s.trim().is_empty()).or_else(|| {
                let m = settings.default_translation_model.trim();
                if m.is_empty() { None } else { Some(m.to_string()) }
            }),
            system_prompt: request.system_prompt.filter(|s| !s.trim().is_empty()).or_else(|| {
                let p = settings.default_system_prompt.trim();
                if p.is_empty() { None } else { Some(p.to_string()) }
            }),
        };
        let started = Instant::now();
        let output = provider.translate(&input).await?;
        Ok(TranslationReceipt {
            provider_id,
            translated_text: output.translated_text,
            detected_source_language: output.detected_source_language,
            duration_ms: elapsed_ms(started),
        })
    }

    pub async fn pipeline(
        &self,
        request: PipelineRequest,
    ) -> Result<VisualPipelineReceipt, String> {
        let started = Instant::now();
        let artifact = self
            .capture(CaptureRequest {
                provider_id: request.screenshot_provider_id,
                monitor_id: request.monitor_id,
            })
            .await
            .map_err(|error| format!("screenshot stage failed: {error}"))?;
        let screenshot_ms = elapsed_ms(started);

        let ocr_started = Instant::now();
        let ocr = self
            .ocr(OcrRequest {
                provider_id: request.ocr_provider_id,
                artifact_id: artifact.id.clone(),
            })
            .await
            .map_err(|error| format!("ocr stage failed: {error}"))?;
        let ocr_stage_ms = elapsed_ms(ocr_started);

        let settings = self.settings();
        let translate_started = Instant::now();
        let translation = self
            .translate(TranslationRequest {
                provider_id: request.translation_provider_id,
                text: ocr.text.clone(),
                source_language: request.source_language,
                target_language: request
                    .target_language
                    .unwrap_or(settings.default_target_language),
                model: request.model,
                system_prompt: request.system_prompt,
            })
            .await
            .map_err(|error| format!("translation stage failed: {error}"))?;
        let translate_stage_ms = elapsed_ms(translate_started);

        Ok(VisualPipelineReceipt {
            schema_version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: elapsed_ms(started),
            stages: vec![
                StageReceipt {
                    stage: "screenshot".to_string(),
                    provider_id: artifact.provider_id.clone(),
                    status: "completed".to_string(),
                    duration_ms: screenshot_ms,
                },
                StageReceipt {
                    stage: "ocr".to_string(),
                    provider_id: ocr.provider_id.clone(),
                    status: "completed".to_string(),
                    duration_ms: ocr_stage_ms,
                },
                StageReceipt {
                    stage: "translation".to_string(),
                    provider_id: translation.provider_id.clone(),
                    status: "completed".to_string(),
                    duration_ms: translate_stage_ms,
                },
            ],
            artifact,
            ocr,
            translation,
            warnings: vec![],
        })
    }

    pub fn artifact_dir(&self) -> &std::path::Path {
        &self.artifact_dir
    }

    pub fn artifact_path(&self, artifact_id: &str) -> Result<PathBuf, String> {
        let id = uuid::Uuid::parse_str(artifact_id)
            .map_err(|_| "artifact id must be a UUID".to_string())?
            .to_string();
        let path = self.artifact_dir.join(format!("{id}.png"));
        if !path.is_file() {
            return Err("visual artifact not found".to_string());
        }
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("failed to inspect visual artifact: {error}"))?;
        if metadata.len() == 0 || metadata.len() > MAX_ARTIFACT_BYTES {
            return Err("visual artifact is empty or exceeds 32 MiB".to_string());
        }
        if metadata
            .modified()
            .ok()
            .and_then(|value| value.elapsed().ok())
            .is_some_and(|age| age > ARTIFACT_TTL)
        {
            let _ = std::fs::remove_file(&path);
            return Err("visual artifact expired".to_string());
        }
        Ok(path)
    }

    pub fn delete_artifact(&self, artifact_id: &str) -> Result<(), String> {
        let path = self.artifact_path(artifact_id)?;
        std::fs::remove_file(&path)
            .map_err(|error| format!("failed to delete visual artifact: {error}"))
    }

    fn screenshot_provider(&self, id: &str) -> Result<Arc<dyn ScreenshotProvider>, String> {
        self.registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .screenshots
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown screenshot provider: {id}"))
    }

    fn ocr_provider(&self, id: &str) -> Result<Arc<dyn OcrProvider>, String> {
        self.registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ocr
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown OCR provider: {id}"))
    }

    fn translation_provider(&self, id: &str) -> Result<Arc<dyn TranslationProvider>, String> {
        self.registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .translations
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown translation provider: {id}"))
    }

    fn cleanup_artifacts(&self) {
        let Ok(entries) = std::fs::read_dir(&self.artifact_dir) else {
            return;
        };
        let mut artifacts = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let stem = path.file_stem()?.to_str()?;
                if path.extension()?.to_str()? != "png" || uuid::Uuid::parse_str(stem).is_err() {
                    return None;
                }
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, path))
            })
            .collect::<Vec<_>>();
        artifacts.retain(|(modified, path)| {
            let expired = modified.elapsed().is_ok_and(|age| age > ARTIFACT_TTL);
            if expired {
                if let Err(error) = std::fs::remove_file(path) {
                    tracing::warn!(
                        "failed to remove expired visual artifact {}: {error}",
                        path.display()
                    );
                }
            }
            !expired
        });
        artifacts.sort_by_key(|(modified, _)| *modified);
        let remove_count = artifacts.len().saturating_sub(MAX_ARTIFACTS - 1);
        for (_, path) in artifacts.into_iter().take(remove_count) {
            if let Err(error) = std::fs::remove_file(&path) {
                tracing::warn!(
                    "failed to remove old visual artifact {}: {error}",
                    path.display()
                );
            }
        }
    }
}

fn ensure_ready(descriptor: &types::ProviderDescriptor) -> Result<(), String> {
    if descriptor.ready {
        Ok(())
    } else {
        Err(descriptor
            .status_message
            .clone()
            .unwrap_or_else(|| format!("provider {} is not ready", descriptor.id)))
    }
}

fn normalize_settings(settings: &mut VisualSettings) -> Result<(), String> {
    settings.screenshot_provider = settings.screenshot_provider.trim().to_string();
    settings.ocr_provider = settings.ocr_provider.trim().to_string();
    settings.translation_provider = settings.translation_provider.trim().to_string();
    settings.default_target_language = settings.default_target_language.trim().to_string();
    settings.default_record_audio = settings.default_record_audio.trim().to_lowercase();
    if !["none", "mic", "system", "both"].contains(&settings.default_record_audio.as_str()) {
        settings.default_record_audio = "system".to_string();
    }
    if settings.default_target_language.is_empty() || settings.default_target_language.len() > 32 {
        return Err("defaultTargetLanguage must contain 1..32 characters".to_string());
    }
    for (name, path) in [
        (
            "detectionModelPath",
            &mut settings.paddle_ocr.detection_model_path,
        ),
        (
            "recognitionModelPath",
            &mut settings.paddle_ocr.recognition_model_path,
        ),
        ("keysPath", &mut settings.paddle_ocr.keys_path),
    ] {
        *path = path.trim().to_string();
        if path.len() > 2048 {
            return Err(format!("{name} exceeds 2048 characters"));
        }
    }
    Ok(())
}

fn validate_translation(
    text: &str,
    target_language: &str,
    source_language: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("translation text cannot be empty".to_string());
    }
    if text.chars().count() > MAX_TRANSLATION_CHARS {
        return Err(format!(
            "translation text exceeds {MAX_TRANSLATION_CHARS} characters"
        ));
    }
    if target_language.trim().is_empty() || target_language.len() > 32 {
        return Err("targetLanguage must contain 1..32 characters".to_string());
    }
    if source_language.is_some_and(|value| value.len() > 32) {
        return Err("sourceLanguage exceeds 32 characters".to_string());
    }
    if model.is_some_and(|value| value.len() > 256) {
        return Err("model exceeds 256 characters".to_string());
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_runtime() -> VisualPipelineRuntime {
        let directory = tempdir().unwrap().keep();
        let app_paths = AppPaths {
            bundle_root: directory.clone(),
            app_data_root: directory,
        };
        VisualPipelineRuntime::new(&app_paths, reqwest::Client::new())
    }

    #[test]
    fn catalog_exposes_three_independent_provider_kinds() {
        let runtime = test_runtime();
        let catalog = runtime.providers();
        assert!(!catalog.translation.is_empty());
        #[cfg(feature = "vision-xcap")]
        assert!(catalog
            .screenshot
            .iter()
            .any(|provider| provider.id == "xcap"));
        #[cfg(feature = "vision-paddleocr")]
        assert!(catalog
            .ocr
            .iter()
            .any(|provider| provider.id == "paddleocr-rs"));
    }

    #[test]
    fn artifact_ids_cannot_escape_store() {
        let runtime = test_runtime();
        assert!(runtime.artifact_path("../../settings/llm.json").is_err());
    }

    #[test]
    fn provider_kind_mismatch_is_rejected() {
        let runtime = test_runtime();
        let mut settings = runtime.settings();
        settings.screenshot_provider = "hub-llm".to_string();
        assert!(runtime.update_settings(settings).is_err());
    }

    #[test]
    fn translation_input_is_bounded() {
        assert!(validate_translation("", "zh-CN", None, None).is_err());
        assert!(
            validate_translation(&"x".repeat(MAX_TRANSLATION_CHARS + 1), "zh-CN", None, None)
                .is_err()
        );
    }

    #[test]
    fn provider_reload_discovers_an_external_manifest_without_executing_it() {
        let runtime = test_runtime();
        let executable = runtime.plugin_dir.join("fake-provider.exe");
        std::fs::write(&executable, b"not executed").unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": "future-ocr",
            "kind": "ocr",
            "displayName": "Future OCR",
            "executable": executable,
            "timeoutSeconds": 5,
            "enabled": true
        });
        std::fs::write(
            runtime.plugin_dir.join("future-ocr.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let catalog = runtime.reload_providers();
        assert!(catalog.ocr.iter().any(|provider| {
            provider.id == "future-ocr" && provider.source == "external-command"
        }));
    }

    #[test]
    fn visual_artifact_can_be_explicitly_deleted() {
        let runtime = test_runtime();
        let id = uuid::Uuid::new_v4().to_string();
        let path = runtime.artifact_dir.join(format!("{id}.png"));
        std::fs::write(&path, b"test").unwrap();
        runtime.delete_artifact(&id).unwrap();
        assert!(!path.exists());
    }
}
