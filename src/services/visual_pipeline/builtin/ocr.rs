use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domains::model_gateway::providers::LlmProvider;

use crate::services::visual_pipeline::types::{
    CaptureProviderOutput, MonitorDescriptor, OcrBlock, OcrProvider, OcrProviderOutput,
    ProviderDescriptor, ProviderKind, ScreenshotProvider, TranslationInput, TranslationProvider,
    TranslationProviderOutput, VisualSettings,
};


use super::*;
use super::common::*;

#[cfg(feature = "vision-paddleocr")]
pub struct PaddleOcrProvider {
    engine: std::sync::Arc<
        std::sync::Mutex<Option<(String, std::sync::Arc<paddleocr_rs_onnx::OcrEngine>)>>,
    >,
}

#[cfg(feature = "vision-paddleocr")]
impl PaddleOcrProvider {
    pub fn new() -> Self {
        Self {
            engine: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

pub(crate) fn configured_file(value: &str) -> bool {
    !value.trim().is_empty() && Path::new(value.trim()).is_file()
}

#[cfg(feature = "vision-paddleocr")]
#[async_trait]
impl OcrProvider for PaddleOcrProvider {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor {
        let paddle = &settings.paddle_ocr;
        let ready = configured_file(&paddle.detection_model_path)
            && configured_file(&paddle.recognition_model_path)
            && configured_file(&paddle.keys_path);
        descriptor(
            "paddleocr-rs",
            ProviderKind::Ocr,
            "PaddleOCR-rs ONNX",
            "0.2.7",
            ready,
            (!ready).then(|| "请配置 detection、recognition ONNX 模型和 keys 文件".to_string()),
            vec![
                "paddleOcr.detectionModelPath".to_string(),
                "paddleOcr.recognitionModelPath".to_string(),
                "paddleOcr.keysPath".to_string(),
            ],
            json!({ "local": true, "offline": true, "orderedText": true }),
        )
    }

    async fn recognize(
        &self,
        image_path: &Path,
        settings: &VisualSettings,
    ) -> Result<OcrProviderOutput, String> {
        let detection = PathBuf::from(settings.paddle_ocr.detection_model_path.trim());
        let recognition = PathBuf::from(settings.paddle_ocr.recognition_model_path.trim());
        let keys = PathBuf::from(settings.paddle_ocr.keys_path.trim());
        if !detection.is_file() || !recognition.is_file() || !keys.is_file() {
            return Err(
                "PaddleOCR-rs is not ready: configure existing detection, recognition and keys files"
                    .to_string(),
            );
        }
        let image_path = image_path.to_path_buf();
        let engine_cache = self.engine.clone();
        tokio::task::spawn_blocking(move || {
            let signature = model_signature(&detection, &recognition, &keys)?;
            let cached = engine_cache
                .lock()
                .map_err(|_| "PaddleOCR engine cache is unavailable".to_string())?
                .as_ref()
                .filter(|(cached_signature, _)| cached_signature == &signature)
                .map(|(_, engine)| engine.clone());
            let engine = if let Some(engine) = cached {
                engine
            } else {
                let detection_bytes = std::fs::read(&detection)
                    .map_err(|error| format!("failed to read detection model: {error}"))?;
                let recognition_bytes = std::fs::read(&recognition)
                    .map_err(|error| format!("failed to read recognition model: {error}"))?;
                let keys_bytes = std::fs::read(&keys)
                    .map_err(|error| format!("failed to read PaddleOCR keys: {error}"))?;
                let engine = std::sync::Arc::new(
                    paddleocr_rs_onnx::OcrEngine::new(
                        &detection_bytes,
                        &recognition_bytes,
                        &keys_bytes,
                    )
                    .map_err(|error| format!("failed to initialize PaddleOCR-rs: {error}"))?,
                );
                *engine_cache
                    .lock()
                    .map_err(|_| "PaddleOCR engine cache is unavailable".to_string())? =
                    Some((signature, engine.clone()));
                engine
            };
            let image = image::open(&image_path)
                .map_err(|error| format!("failed to decode visual artifact: {error}"))?;
            let recognized = engine
                .recognize_all(&image, paddleocr_rs_onnx::OrderBy::Horizontal)
                .map_err(|error| format!("PaddleOCR-rs recognition failed: {error}"))?;
            let blocks = recognized
                .into_iter()
                .filter_map(|block| {
                    let text = block.text.trim().to_string();
                    (!text.is_empty()).then_some(OcrBlock {
                        text,
                        confidence: Some(block.confidence),
                        box_: Some(vec![block.x, block.y, block.x + block.width, block.y + block.height]),
                    })
                })
                .collect::<Vec<_>>();
            let text = blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(OcrProviderOutput { text, blocks })
        })
        .await
        .map_err(|error| format!("PaddleOCR worker failed: {error}"))?
    }
}

#[cfg(feature = "vision-paddleocr")]
pub(crate) fn model_signature(detection: &Path, recognition: &Path, keys: &Path) -> Result<String, String> {
    [detection, recognition, keys]
        .iter()
        .map(|path| {
            let metadata = std::fs::metadata(path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_millis())
                .unwrap_or_default();
            Ok(format!("{}:{}:{modified}", path.display(), metadata.len()))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|parts| parts.join("|"))
}

#[cfg(feature = "vision-rapidocr")]
pub struct RapidOcrProvider {
    engine: std::sync::Arc<
        std::sync::Mutex<Option<(String, rapid_ocr_rs::RapidOcrEngine)>>,
    >,
}

#[cfg(feature = "vision-rapidocr")]
impl RapidOcrProvider {
    pub fn new() -> Self {
        Self {
            engine: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

#[cfg(feature = "vision-rapidocr")]
#[async_trait]
impl OcrProvider for RapidOcrProvider {
    fn descriptor(&self, settings: &VisualSettings) -> ProviderDescriptor {
        let paddle = &settings.paddle_ocr;
        let configured = configured_file(&paddle.detection_model_path)
            && configured_file(&paddle.recognition_model_path)
            && configured_file(&paddle.keys_path);
        descriptor(
            "rapid-ocr",
            ProviderKind::Ocr,
            "Rapid OCR (ONNX)",
            "0.1.0",
            true, // Always ready, downloads automatically if not configured
            None,
            vec![
                "paddleOcr.detectionModelPath".to_string(),
                "paddleOcr.recognitionModelPath".to_string(),
                "paddleOcr.keysPath".to_string(),
            ],
            json!({ "local": true, "offline": true, "orderedText": true }),
        )
    }

    async fn recognize(
        &self,
        image_path: &Path,
        settings: &VisualSettings,
    ) -> Result<OcrProviderOutput, String> {
        let detection = PathBuf::from(settings.paddle_ocr.detection_model_path.trim());
        let recognition = PathBuf::from(settings.paddle_ocr.recognition_model_path.trim());
        let keys = PathBuf::from(settings.paddle_ocr.keys_path.trim());
        let configured = detection.is_file() && recognition.is_file() && keys.is_file();
        
        let image_path = image_path.to_path_buf();
        let engine_cache = self.engine.clone();
        
        tokio::task::spawn_blocking(move || {
            let signature = || -> Result<String, String> {
                if configured {
                    [detection.as_path(), recognition.as_path(), keys.as_path()]
                        .iter()
                        .map(|path| {
                            let metadata = std::fs::metadata(path)
                                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
                            let modified = metadata
                                .modified()
                                .ok()
                                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|value| value.as_millis())
                                .unwrap_or_default();
                            Ok(format!("{}:{}:{modified}", path.display(), metadata.len()))
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(|parts| parts.join("|"))
                } else {
                    Ok("default_auto_download".to_string())
                }
            }()?;
            
            let mut guard = engine_cache
                .lock()
                .map_err(|_| "RapidOCR engine cache is unavailable".to_string())?;

            if guard.as_ref().map(|(s, _)| s) != Some(&signature) {
                let mut config = rapid_ocr_rs::EngineConfig::default();
                config.global.use_cls = false; // Usually Paddle OCR models don't need cls unless provided
                
                let engine = if configured {
                    let keys_text = std::fs::read_to_string(&keys)
                        .map_err(|error| format!("failed to read RapidOCR keys: {error}"))?;
                    
                    let sources = rapid_ocr_rs::PipelineSources {
                        det: Some(rapid_ocr_rs::ModelSource::File(&detection)),
                        cls: None,
                        rec: Some(rapid_ocr_rs::ModelSource::File(&recognition)),
                        rec_dictionary: Some(rapid_ocr_rs::DictionarySource::Memory {
                            name: "keys",
                            text: &keys_text,
                        }),
                    };
                    rapid_ocr_rs::RapidOcrEngine::new_with_sources(config, sources)
                        .map_err(|error| format!("failed to initialize RapidOCR with sources: {:?}", error))?
                } else {
                    rapid_ocr_rs::RapidOcrEngine::new(config)
                        .map_err(|error| format!("failed to initialize RapidOCR (auto download): {:?}", error))?
                };
                
                *guard = Some((signature, engine));
            }
            
            let engine = &mut guard.as_mut().unwrap().1;
            
            let input = rapid_ocr_rs::OcrInput::Path(image_path);
            let result = engine.run(input, rapid_ocr_rs::RunOptions::default())
                .map_err(|error| format!("RapidOCR recognition failed: {:?}", error))?;

            let blocks = match result {
                rapid_ocr_rs::OcrResult::Full(full) => {
                    full.lines.into_iter().zip(full.boxes.into_iter()).filter_map(|(line, bx)| {
                        let text = line.text.trim().to_string();
                        if text.is_empty() {
                            return None;
                        }
                        
                        let x_min = bx.iter().map(|p| p[0]).fold(f32::INFINITY, |a, b| a.min(b));
                        let y_min = bx.iter().map(|p| p[1]).fold(f32::INFINITY, |a, b| a.min(b));
                        let x_max = bx.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, |a, b| a.max(b));
                        let y_max = bx.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, |a, b| a.max(b));
                        
                        Some(OcrBlock {
                            text,
                            confidence: Some(line.score),
                            box_: Some(vec![x_min, y_min, x_max, y_max]),
                        })
                    }).collect::<Vec<_>>()
                }
                rapid_ocr_rs::OcrResult::Rec(rec) => {
                    rec.lines.into_iter().filter_map(|line| {
                        let text = line.text.trim().to_string();
                        (!text.is_empty()).then_some(OcrBlock {
                            text,
                            confidence: Some(line.score),
                            box_: None,
                        })
                    }).collect::<Vec<_>>()
                }
                _ => vec![],
            };

            let text = blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
            
            Ok(OcrProviderOutput { text, blocks })
        })
        .await
        .map_err(|error| format!("RapidOCR worker failed: {error}"))?
    }
}


