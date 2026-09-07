use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::services::llm_provider::LlmProvider;

use crate::services::visual_pipeline::types::{
    CaptureProviderOutput, MonitorDescriptor, OcrBlock, OcrProvider, OcrProviderOutput,
    ProviderDescriptor, ProviderKind, ScreenshotProvider, TranslationInput, TranslationProvider,
    TranslationProviderOutput, VisualSettings,
};


use super::*;

pub(crate) fn descriptor(
    id: &str,
    kind: ProviderKind,
    display_name: &str,
    version: &str,
    ready: bool,
    status_message: Option<String>,
    configuration_requirements: Vec<String>,
    capabilities: Value,
) -> ProviderDescriptor {
    ProviderDescriptor {
        id: id.to_string(),
        kind,
        display_name: display_name.to_string(),
        source: "builtin".to_string(),
        version: version.to_string(),
        ready,
        status_message,
        configuration_requirements,
        capabilities,
    }
}


