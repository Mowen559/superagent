use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Screenshot,
    Ocr,
    Translation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub source: String,
    pub version: String,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default)]
    pub configuration_requirements: Vec<String>,
    #[serde(default)]
    pub capabilities: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub screenshot: Vec<ProviderDescriptor>,
    pub ocr: Vec<ProviderDescriptor>,
    pub translation: Vec<ProviderDescriptor>,
    pub load_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaddleOcrSettings {
    #[serde(default)]
    pub detection_model_path: String,
    #[serde(default)]
    pub recognition_model_path: String,
    #[serde(default)]
    pub keys_path: String,
}

impl Default for PaddleOcrSettings {
    fn default() -> Self {
        Self {
            detection_model_path: std::env::var("HUB_PADDLEOCR_DETECTION_MODEL")
                .unwrap_or_default(),
            recognition_model_path: std::env::var("HUB_PADDLEOCR_RECOGNITION_MODEL")
                .unwrap_or_default(),
            keys_path: std::env::var("HUB_PADDLEOCR_KEYS").unwrap_or_default(),
        }
    }
}

fn settings_schema_version() -> u32 {
    1
}

fn default_screenshot_provider() -> String {
    "snow-capture".to_string()
}

fn default_ocr_provider() -> String {
    "rapid-ocr".to_string()
}

fn default_translation_provider() -> String {
    "hub-llm".to_string()
}

fn default_target_language() -> String {
    "zh-CN".to_string()
}

fn default_translation_model() -> String {
    "".to_string()
}

fn default_system_prompt() -> String {
    "".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualSettings {
    #[serde(default = "settings_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_screenshot_provider")]
    pub screenshot_provider: String,
    #[serde(default = "default_ocr_provider")]
    pub ocr_provider: String,
    #[serde(default = "default_translation_provider")]
    pub translation_provider: String,
    #[serde(default)]
    pub paddle_ocr: PaddleOcrSettings,
    #[serde(default = "default_target_language")]
    pub default_target_language: String,
    #[serde(default = "default_translation_model")]
    pub default_translation_model: String,
    #[serde(default = "default_system_prompt")]
    pub default_system_prompt: String,
}

impl Default for VisualSettings {
    fn default() -> Self {
        Self {
            schema_version: settings_schema_version(),
            screenshot_provider: default_screenshot_provider(),
            ocr_provider: default_ocr_provider(),
            translation_provider: default_translation_provider(),
            paddle_ocr: PaddleOcrSettings::default(),
            default_target_language: default_target_language(),
            default_translation_model: default_translation_model(),
            default_system_prompt: default_system_prompt(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescriptor {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub provider_id: Option<String>,
    pub monitor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualArtifact {
    pub id: String,
    pub provider_id: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProviderOutput {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBlock {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "box")]
    pub box_: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrProviderOutput {
    pub text: String,
    #[serde(default)]
    pub blocks: Vec<OcrBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub provider_id: Option<String>,
    pub artifact_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrReceipt {
    pub provider_id: String,
    pub artifact_id: String,
    pub text: String,
    pub blocks: Vec<OcrBlock>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationInput {
    pub text: String,
    pub source_language: Option<String>,
    pub target_language: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub provider_id: Option<String>,
    pub text: String,
    pub source_language: Option<String>,
    pub target_language: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationProviderOutput {
    pub translated_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_source_language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationReceipt {
    pub provider_id: String,
    pub translated_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_source_language: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRequest {
    pub screenshot_provider_id: Option<String>,
    pub ocr_provider_id: Option<String>,
    pub translation_provider_id: Option<String>,
    pub monitor_id: Option<String>,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageReceipt {
    pub stage: String,
    pub provider_id: String,
    pub status: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualPipelineReceipt {
    pub schema_version: u32,
    pub id: String,
    pub created_at: String,
    pub duration_ms: u64,
    pub artifact: VisualArtifact,
    pub ocr: OcrReceipt,
    pub translation: TranslationReceipt,
    pub stages: Vec<StageReceipt>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait ScreenshotProvider: Send + Sync {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor;
    async fn monitors(&self) -> Result<Vec<MonitorDescriptor>, String>;
    async fn capture(
        &self,
        monitor_id: Option<&str>,
        output_path: &Path,
    ) -> Result<CaptureProviderOutput, String>;
}

#[async_trait]
pub trait OcrProvider: Send + Sync {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor;
    async fn recognize(
        &self,
        image_path: &Path,
        settings: &VisualSettings,
    ) -> Result<OcrProviderOutput, String>;
}

#[async_trait]
pub trait TranslationProvider: Send + Sync {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor;
    async fn translate(
        &self,
        input: &TranslationInput,
    ) -> Result<TranslationProviderOutput, String>;
}
