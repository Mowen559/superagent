use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::types::{
    CaptureProviderOutput, MonitorDescriptor, OcrProvider, OcrProviderOutput, ProviderDescriptor,
    ProviderKind, ScreenshotProvider, TranslationInput, TranslationProvider,
    TranslationProviderOutput, VisualSettings,
};

const MAX_PLUGIN_MANIFESTS: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PLUGIN_OUTPUT_BYTES: usize = 1024 * 1024;

fn default_schema_version() -> u32 {
    1
}

fn default_timeout_seconds() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ExternalProviderManifest {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!("unsupported schemaVersion {}", self.schema_version));
        }
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
        {
            return Err("provider id must match [A-Za-z0-9_-]{1,64}".to_string());
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > 120 {
            return Err("displayName must contain 1..120 characters".to_string());
        }
        let executable = Path::new(self.executable.trim());
        if !executable.is_absolute() || !executable.is_file() {
            return Err("executable must be an existing absolute local file".to_string());
        }
        if self.args.len() > 32 || self.args.iter().any(|argument| argument.len() > 512) {
            return Err("plugin args exceed bounded limits".to_string());
        }
        if !(1..=300).contains(&self.timeout_seconds) {
            return Err("timeoutSeconds must be between 1 and 300".to_string());
        }
        Ok(())
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id.clone(),
            kind: self.kind,
            display_name: self.display_name.clone(),
            source: "external-command".to_string(),
            version: format!("manifest-v{}", self.schema_version),
            ready: true,
            status_message: None,
            configuration_requirements: vec![],
            capabilities: json!({ "jsonStdio": true, "timeoutSeconds": self.timeout_seconds }),
        }
    }

    async fn invoke(&self, request: Value) -> Result<Value, String> {
        let mut child = Command::new(&self.executable)
            .args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("failed to start provider {}: {error}", self.id))?;
        let request = serde_json::to_vec(&request)
            .map_err(|error| format!("failed to encode provider request: {error}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "provider stdin is unavailable".to_string())?;
        stdin
            .write_all(&request)
            .await
            .map_err(|error| format!("failed to write provider stdin: {error}"))?;
        drop(stdin);

        let output = tokio::time::timeout(
            Duration::from_secs(self.timeout_seconds),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| format!("provider {} timed out", self.id))?
        .map_err(|error| format!("provider {} failed: {error}", self.id))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "provider {} exited with {}: {}",
                self.id,
                output.status,
                stderr.chars().take(500).collect::<String>()
            ));
        }
        if output.stdout.len() > MAX_PLUGIN_OUTPUT_BYTES {
            return Err(format!("provider {} output exceeded 1 MiB", self.id));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("provider {} returned invalid JSON: {error}", self.id))
    }
}

pub struct ExternalScreenshotProvider(pub ExternalProviderManifest);

#[async_trait]
impl ScreenshotProvider for ExternalScreenshotProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        self.0.descriptor()
    }

    async fn monitors(&self) -> Result<Vec<MonitorDescriptor>, String> {
        let value = self.0.invoke(json!({ "operation": "monitors" })).await?;
        serde_json::from_value(value.get("monitors").cloned().unwrap_or(Value::Null))
            .map_err(|error| format!("invalid external monitor response: {error}"))
    }

    async fn capture(
        &self,
        monitor_id: Option<&str>,
        output_path: &Path,
    ) -> Result<CaptureProviderOutput, String> {
        let value = self
            .0
            .invoke(json!({
                "operation": "capture",
                "monitorId": monitor_id,
                "outputPath": output_path
            }))
            .await?;
        if !output_path.is_file() {
            return Err("external screenshot provider did not create outputPath".to_string());
        }
        serde_json::from_value(value)
            .map_err(|error| format!("invalid external capture response: {error}"))
    }
}

pub struct ExternalOcrProvider(pub ExternalProviderManifest);

#[async_trait]
impl OcrProvider for ExternalOcrProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        self.0.descriptor()
    }

    async fn recognize(
        &self,
        image_path: &Path,
        _settings: &VisualSettings,
    ) -> Result<OcrProviderOutput, String> {
        let value = self
            .0
            .invoke(json!({ "operation": "recognize", "imagePath": image_path }))
            .await?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid external OCR response: {error}"))
    }
}

pub struct ExternalTranslationProvider(pub ExternalProviderManifest);

#[async_trait]
impl TranslationProvider for ExternalTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        self.0.descriptor()
    }

    async fn translate(
        &self,
        input: &TranslationInput,
        _settings: &VisualSettings,
    ) -> Result<TranslationProviderOutput, String> {
        let value = self
            .0
            .invoke(json!({
                "operation": "translate",
                "text": input.text,
                "sourceLanguage": input.source_language,
                "targetLanguage": input.target_language,
                "model": input.model
            }))
            .await?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid external translation response: {error}"))
    }
}

pub struct ExternalProviders {
    pub screenshots: Vec<ExternalScreenshotProvider>,
    pub ocr: Vec<ExternalOcrProvider>,
    pub translations: Vec<ExternalTranslationProvider>,
    pub warnings: Vec<String>,
}

pub fn load(directory: &Path) -> ExternalProviders {
    let mut providers = ExternalProviders {
        screenshots: vec![],
        ocr: vec![],
        translations: vec![],
        warnings: vec![],
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return providers;
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    for path in paths.into_iter().take(MAX_PLUGIN_MANIFESTS) {
        let result = (|| -> Result<ExternalProviderManifest, String> {
            let metadata = std::fs::metadata(&path)
                .map_err(|error| format!("failed to inspect manifest: {error}"))?;
            if metadata.len() > MAX_MANIFEST_BYTES {
                return Err("manifest exceeds 64 KiB".to_string());
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|error| format!("failed to read manifest: {error}"))?;
            let manifest: ExternalProviderManifest = serde_json::from_str(&content)
                .map_err(|error| format!("invalid manifest JSON: {error}"))?;
            manifest.validate()?;
            Ok(manifest)
        })();
        match result {
            Ok(manifest) if !manifest.enabled => {}
            Ok(manifest) => match manifest.kind {
                ProviderKind::Screenshot => providers
                    .screenshots
                    .push(ExternalScreenshotProvider(manifest)),
                ProviderKind::Ocr => providers.ocr.push(ExternalOcrProvider(manifest)),
                ProviderKind::Translation => providers
                    .translations
                    .push(ExternalTranslationProvider(manifest)),
            },
            Err(error) => providers
                .warnings
                .push(format!("{}: {error}", path.display())),
        }
    }
    providers
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn manifest_rejects_non_file_executable() {
        let manifest = ExternalProviderManifest {
            schema_version: 1,
            id: "scap".to_string(),
            kind: ProviderKind::Screenshot,
            display_name: "scap".to_string(),
            executable: "definitely-missing-provider".to_string(),
            args: vec![],
            timeout_seconds: 30,
            enabled: true,
        };
        assert!(manifest.validate().is_err());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn external_translation_uses_json_stdio_contract() {
        let directory = tempdir().unwrap();
        let script = directory.path().join("translation-provider.ps1");
        std::fs::write(
            &script,
            r#"$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
@{ translatedText = "plugin:$($payload.text)"; detectedSourceLanguage = "en" } | ConvertTo-Json -Compress
"#,
        )
        .unwrap();
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let powershell = Path::new(&system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let manifest = ExternalProviderManifest {
            schema_version: 1,
            id: "test-translation".to_string(),
            kind: ProviderKind::Translation,
            display_name: "Test translation".to_string(),
            executable: powershell.to_string_lossy().to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-File".to_string(),
                script.to_string_lossy().to_string(),
            ],
            timeout_seconds: 10,
            enabled: true,
        };
        manifest.validate().unwrap();
        let provider = ExternalTranslationProvider(manifest);
        let output = provider
            .translate(&TranslationInput {
                text: "hello".to_string(),
                source_language: None,
                target_language: "zh-CN".to_string(),
                model: None,
            })
            .await
            .unwrap();
        assert_eq!(output.translated_text, "plugin:hello");
        assert_eq!(output.detected_source_language.as_deref(), Some("en"));
    }
}
