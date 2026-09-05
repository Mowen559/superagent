use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::services::llm_provider::LlmProvider;

use super::types::{
    CaptureProviderOutput, MonitorDescriptor, OcrBlock, OcrProvider, OcrProviderOutput,
    ProviderDescriptor, ProviderKind, ScreenshotProvider, TranslationInput, TranslationProvider,
    TranslationProviderOutput, VisualSettings,
};

fn descriptor(
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

#[cfg(feature = "vision-xcap")]
pub struct XcapScreenshotProvider;

#[cfg(feature = "vision-xcap")]
#[async_trait]
impl ScreenshotProvider for XcapScreenshotProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        descriptor(
            "xcap",
            ProviderKind::Screenshot,
            "xcap 屏幕截图",
            "0.9.8",
            true,
            None,
            vec![],
            json!({ "monitors": true, "format": "png", "mode": "still", "windowsFallback": "gdi" }),
        )
    }

    async fn monitors(&self) -> Result<Vec<MonitorDescriptor>, String> {
        tokio::task::spawn_blocking(|| {
            let monitors = xcap::Monitor::all()
                .map_err(|error| format!("xcap failed to enumerate monitors: {error}"))?;
            monitors
                .into_iter()
                .map(|monitor| {
                    let id = monitor
                        .id()
                        .map_err(|error| format!("xcap failed to read monitor id: {error}"))?;
                    Ok(MonitorDescriptor {
                        id: id.to_string(),
                        name: monitor
                            .friendly_name()
                            .unwrap_or_else(|_| format!("Monitor {id}")),
                        width: monitor.width().unwrap_or_default(),
                        height: monitor.height().unwrap_or_default(),
                        primary: monitor.is_primary().unwrap_or(false),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .await
        .map_err(|error| format!("xcap monitor worker failed: {error}"))?
    }

    async fn capture(
        &self,
        monitor_id: Option<&str>,
        output_path: &Path,
    ) -> Result<CaptureProviderOutput, String> {
        let requested = monitor_id.map(ToOwned::to_owned);
        let output_path = output_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let monitors = xcap::Monitor::all()
                .map_err(|error| format!("xcap failed to enumerate monitors: {error}"))?;
            let monitor = if let Some(requested) = requested.as_deref() {
                monitors
                    .into_iter()
                    .find(|monitor| {
                        monitor
                            .id()
                            .ok()
                            .is_some_and(|id| id.to_string() == requested)
                    })
                    .ok_or_else(|| format!("monitor not found: {requested}"))?
            } else {
                let primary = monitors
                    .iter()
                    .position(|monitor| monitor.is_primary().unwrap_or(false))
                    .unwrap_or(0);
                monitors
                    .into_iter()
                    .nth(primary)
                    .ok_or_else(|| "xcap found no monitor".to_string())?
            };
            let monitor_x = monitor.x().unwrap_or_default();
            let monitor_y = monitor.y().unwrap_or_default();
            let monitor_width = monitor
                .width()
                .map_err(|error| format!("xcap failed to read monitor width: {error}"))?;
            let monitor_height = monitor
                .height()
                .map_err(|error| format!("xcap failed to read monitor height: {error}"))?;
            let image = match monitor.capture_image() {
                Ok(image) => image,
                #[cfg(target_os = "windows")]
                Err(xcap_error) => windows_gdi_capture(
                    monitor_x,
                    monitor_y,
                    monitor_width,
                    monitor_height,
                )
                .map_err(|fallback_error| {
                    format!(
                        "xcap capture failed: {xcap_error}; Windows GDI fallback failed: {fallback_error}"
                    )
                })?,
                #[cfg(not(target_os = "windows"))]
                Err(error) => return Err(format!("xcap capture failed: {error}")),
            };
            let scale = monitor.scale_factor().unwrap_or(1.0);
            let logical_width = (image.width() as f32 / scale).round() as u32;
            let logical_height = (image.height() as f32 / scale).round() as u32;
            {
                use std::fs::File;
                use std::io::BufWriter;
                use image::codecs::png::{PngEncoder, CompressionType, FilterType};
                use image::ImageEncoder;
                let file = File::create(&output_path).map_err(|error| format!("failed to create file: {error}"))?;
                let mut w = BufWriter::new(file);
                let encoder = PngEncoder::new_with_quality(w, CompressionType::Fast, FilterType::NoFilter);
                encoder.write_image(image.as_raw(), image.width(), image.height(), image::ColorType::Rgba8.into())
                    .map_err(|error| format!("failed to encode screenshot: {error}"))?;
            }
            Ok(CaptureProviderOutput { width: logical_width, height: logical_height })
        })
        .await
        .map_err(|error| format!("xcap capture worker failed: {error}"))?
    }
}

#[cfg(feature = "vision-snowcapture")]
pub struct SnowCaptureScreenshotProvider;

#[cfg(feature = "vision-snowcapture")]
#[async_trait]
impl ScreenshotProvider for SnowCaptureScreenshotProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        descriptor(
            "snow-capture",
            ProviderKind::Screenshot,
            "Snow Capture (WGC)",
            "0.1.0",
            true,
            None,
            vec![],
            json!({ "monitors": true, "format": "png", "mode": "still" }),
        )
    }

    async fn monitors(&self) -> Result<Vec<MonitorDescriptor>, String> {
        tokio::task::spawn_blocking(|| {
            let xcap_monitors = xcap::Monitor::all().unwrap_or_default();
            let monitors = snow_capture::monitor::enumerate_monitors()
                .map_err(|error| format!("snow-capture failed to enumerate monitors: {error}"))?;
            monitors
                .into_iter()
                .map(|monitor| {
                    let mut width = 0;
                    let mut height = 0;
                    
                    let name = monitor.name().to_string();
                    if let Some(xcap_mon) = xcap_monitors.iter().find(|m| m.name().unwrap_or_default() == name) {
                        width = xcap_mon.width().unwrap_or_default();
                        height = xcap_mon.height().unwrap_or_default();
                    } else if let Some(xcap_mon) = xcap_monitors.iter().find(|m| m.is_primary().unwrap_or(false) == monitor.is_primary()) {
                        width = xcap_mon.width().unwrap_or_default();
                        height = xcap_mon.height().unwrap_or_default();
                    }

                    Ok(MonitorDescriptor {
                        id: monitor.stable_id(),
                        name,
                        width,
                        height,
                        primary: monitor.is_primary(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .await
        .map_err(|error| format!("snow-capture monitor worker failed: {error}"))?
    }

    async fn capture(
        &self,
        monitor_id: Option<&str>,
        output_path: &Path,
    ) -> Result<CaptureProviderOutput, String> {
        let requested = monitor_id.map(ToOwned::to_owned);
        let output_path = output_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let target = if let Some(requested) = requested.as_deref() {
                let monitors = snow_capture::monitor::enumerate_monitors()
                    .map_err(|error| format!("snow-capture failed to enumerate monitors: {error}"))?;
                let monitor = monitors
                    .into_iter()
                    .find(|m| m.stable_id() == requested)
                    .ok_or_else(|| format!("monitor not found: {requested}"))?;
                snow_capture::CaptureTarget::Monitor(monitor)
            } else {
                snow_capture::CaptureTarget::PrimaryMonitor
            };

            let frame = snow_capture::capture_once(&target)
                .map_err(|error| format!("snow-capture failed: {error}"))?;
            
            let physical_width = frame.width();
            let physical_height = frame.height();
            let mut logical_width = physical_width;
            let mut logical_height = physical_height;
            
            // Try to override with logical dimensions from xcap
            if let Ok(xcap_monitors) = xcap::Monitor::all() {
                let target_monitor = if let Some(requested_id) = requested.as_deref() {
                    let mut found = None;
                    if let Ok(monitors) = snow_capture::monitor::enumerate_monitors() {
                        if let Some(target_mon) = monitors.iter().find(|m| m.stable_id() == requested_id) {
                            let name = target_mon.name().to_string();
                            found = xcap_monitors.iter().find(|m| m.name().unwrap_or_default() == name)
                                .or_else(|| xcap_monitors.iter().find(|m| m.is_primary().unwrap_or(false) == target_mon.is_primary()));
                        }
                    }
                    found
                } else {
                    xcap_monitors.iter().find(|m| m.is_primary().unwrap_or(false))
                };

                if let Some(xcap_mon) = target_monitor {
                    let scale = xcap_mon.scale_factor().unwrap_or(1.0);
                    let pw = xcap_mon.width().unwrap_or(logical_width);
                    let ph = xcap_mon.height().unwrap_or(logical_height);
                    logical_width = (pw as f32 / scale).round() as u32;
                    logical_height = (ph as f32 / scale).round() as u32;
                }
            }
            
            let rgba = frame.as_rgba_bytes();

            {
                use std::fs::File;
                use std::io::BufWriter;
                use image::codecs::png::{PngEncoder, CompressionType, FilterType};
                use image::ImageEncoder;
                let file = File::create(&output_path).map_err(|error| format!("failed to create file: {error}"))?;
                let mut w = BufWriter::new(file);
                let encoder = PngEncoder::new_with_quality(w, CompressionType::Fast, FilterType::NoFilter);
                encoder.write_image(&rgba, physical_width, physical_height, image::ColorType::Rgba8.into())
                    .map_err(|error| format!("failed to encode screenshot: {error}"))?;
            }
            Ok(CaptureProviderOutput { width: logical_width, height: logical_height })
        })
        .await
        .map_err(|error| format!("snow-capture capture worker failed: {error}"))?
    }
}

#[cfg(all(feature = "vision-xcap", target_os = "windows"))]
fn windows_gdi_capture(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<xcap::image::RgbaImage, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
        SRCCOPY,
    };

    if width == 0 || height == 0 {
        return Err("monitor dimensions are empty".to_string());
    }
    let width_i32 = i32::try_from(width).map_err(|_| "monitor width is too large".to_string())?;
    let height_i32 =
        i32::try_from(height).map_err(|_| "monitor height is too large".to_string())?;
    let byte_count = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(height as usize))
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "monitor image size overflow".to_string())?;

    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return Err("GetDC returned a null screen device context".to_string());
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err("CreateCompatibleDC failed".to_string());
        }
        let bitmap = CreateCompatibleBitmap(screen_dc, width_i32, height_i32);
        if bitmap.is_null() {
            DeleteDC(memory_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err("CreateCompatibleBitmap failed".to_string());
        }
        let previous = SelectObject(memory_dc, bitmap);
        let result = (|| {
            if BitBlt(
                memory_dc,
                0,
                0,
                width_i32,
                height_i32,
                screen_dc,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            ) == 0
            {
                return Err("BitBlt failed".to_string());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
            info.bmiHeader.biWidth = width_i32;
            info.bmiHeader.biHeight = -height_i32;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = vec![0_u8; byte_count];
            if GetDIBits(
                memory_dc,
                bitmap,
                0,
                height,
                pixels.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            ) == 0
            {
                return Err("GetDIBits failed".to_string());
            }
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = u8::MAX;
            }
            xcap::image::RgbaImage::from_raw(width, height, pixels)
                .ok_or_else(|| "failed to create RGBA image".to_string())
        })();
        SelectObject(memory_dc, previous);
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        result
    }
}

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

fn configured_file(value: &str) -> bool {
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
fn model_signature(detection: &Path, recognition: &Path, keys: &Path) -> Result<String, String> {
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

pub struct HubLlmTranslationProvider {
    client: reqwest::Client,
}

impl HubLlmTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for HubLlmTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        let ready = LlmProvider::new(self.client.clone()).is_available();
        descriptor(
            "hub-llm",
            ProviderKind::Translation,
            "Hub 模型网关（Gemini / GPT 等）",
            env!("CARGO_PKG_VERSION"),
            ready,
            (!ready).then(|| "请先在模型管理中配置可用账户".to_string()),
            vec!["Hub model account".to_string()],
            json!({ "models": ["gemini", "gpt", "openai-compatible"], "network": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
    ) -> Result<TranslationProviderOutput, String> {
        let source = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("auto-detect");
        let message = json!({
            "role": "user",
            "content": format!(
                "Translate the following text from {source} to {}.\n\n{}",
                input.target_language,
                input.text
            )
        });

        let system_prompt = input.system_prompt.clone().unwrap_or_else(|| {
            "You are a pure translation engine. You MUST output ONLY the translated text without any explanation, markdown formatting, or surrounding quotes. Preserve the original line breaks.".to_string()
        });

        let request_body = json!({
            "model": input.model.clone().unwrap_or_else(|| "gpt-4o".to_string()),
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                message
            ],
            "temperature": 0.0
        });

        let proxy_url = "http://127.0.0.1:3001/v1/chat/completions";
        let response = self.client.post(proxy_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Failed to call internal proxy: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(format!("Proxy error {}: {}", status, err_text));
        }

        let resp_json: serde_json::Value = response.json().await
            .map_err(|e| format!("Failed to parse proxy response: {}", e))?;

        let mut translated_text = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
            
        // Clean up markdown block if the model ignores the instruction
        if translated_text.starts_with("```") {
            if let Some(start_idx) = translated_text.find('\n') {
                translated_text = translated_text[start_idx..].trim_start().to_string();
            }
            if translated_text.ends_with("```") {
                translated_text = translated_text[..translated_text.len()-3].trim_end().to_string();
            }
        }
        
        Ok(TranslationProviderOutput {
            translated_text,
            detected_source_language: None,
        })
    }
}

pub struct DeepLTranslationProvider {
    client: reqwest::Client,
}

impl DeepLTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for DeepLTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        let ready = std::env::var("DEEPL_API_KEY")
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        descriptor(
            "deepl",
            ProviderKind::Translation,
            "DeepL API",
            "v2",
            ready,
            (!ready).then(|| "缺少环境变量 DEEPL_API_KEY".to_string()),
            vec!["DEEPL_API_KEY".to_string()],
            json!({ "network": true, "sourceLanguageAutoDetect": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
    ) -> Result<TranslationProviderOutput, String> {
        let key = std::env::var("DEEPL_API_KEY")
            .map_err(|_| "DEEPL_API_KEY is not configured".to_string())?;
        let default_url = if key.ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate"
        } else {
            "https://api.deepl.com/v2/translate"
        };
        let endpoint = std::env::var("DEEPL_TRANSLATE_URL").unwrap_or_else(|_| default_url.into());
        let mut body = json!({
            "text": [input.text],
            "target_lang": normalize_language(&input.target_language, true)
        });
        if let Some(source) = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["source_lang"] = json!(normalize_language(source, true));
        }
        let response = self
            .client
            .post(endpoint)
            .header("Authorization", format!("DeepL-Auth-Key {key}"))
            .json(&body)
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
            .map_err(|error| format!("DeepL request failed: {error}"))?;
        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("DeepL returned invalid JSON: {error}"))?;
        if !status.is_success() {
            return Err(format!("DeepL returned HTTP {status}"));
        }
        let translation = payload
            .get("translations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| "DeepL response has no translation".to_string())?;
        Ok(TranslationProviderOutput {
            translated_text: translation
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            detected_source_language: translation
                .get("detected_source_language")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

pub struct GoogleTranslationProvider {
    client: reqwest::Client,
}

impl GoogleTranslationProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TranslationProvider for GoogleTranslationProvider {
    fn descriptor(&self, _settings: &VisualSettings) -> ProviderDescriptor {
        let ready = std::env::var("GOOGLE_TRANSLATE_API_KEY")
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        descriptor(
            "google-translate",
            ProviderKind::Translation,
            "Google Cloud Translation",
            "v2",
            ready,
            (!ready).then(|| "缺少环境变量 GOOGLE_TRANSLATE_API_KEY".to_string()),
            vec!["GOOGLE_TRANSLATE_API_KEY".to_string()],
            json!({ "network": true, "sourceLanguageAutoDetect": true }),
        )
    }

    async fn translate(
        &self,
        input: &TranslationInput,
    ) -> Result<TranslationProviderOutput, String> {
        let key = std::env::var("GOOGLE_TRANSLATE_API_KEY")
            .map_err(|_| "GOOGLE_TRANSLATE_API_KEY is not configured".to_string())?;
        let endpoint = std::env::var("GOOGLE_TRANSLATE_URL")
            .unwrap_or_else(|_| "https://translation.googleapis.com/language/translate/v2".into());
        let mut body = json!({
            "q": input.text,
            "target": normalize_language(&input.target_language, false),
            "format": "text"
        });
        if let Some(source) = input
            .source_language
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["source"] = json!(normalize_language(source, false));
        }
        let response = self
            .client
            .post(endpoint)
            .header("X-Goog-Api-Key", key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
            .map_err(|error| format!("Google Translation request failed: {error}"))?;
        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("Google Translation returned invalid JSON: {error}"))?;
        if !status.is_success() {
            return Err(format!("Google Translation returned HTTP {status}"));
        }
        let translation = payload
            .pointer("/data/translations/0")
            .ok_or_else(|| "Google Translation response has no translation".to_string())?;
        Ok(TranslationProviderOutput {
            translated_text: translation
                .get("translatedText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            detected_source_language: translation
                .get("detectedSourceLanguage")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

fn normalize_language(language: &str, uppercase: bool) -> String {
    let normalized = language.trim().replace('_', "-");
    if uppercase {
        normalized.to_ascii_uppercase()
    } else {
        normalized.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_are_normalized_per_adapter() {
        assert_eq!(normalize_language("zh_CN", true), "ZH-CN");
        assert_eq!(normalize_language("EN-US", false), "en-us");
    }
}
