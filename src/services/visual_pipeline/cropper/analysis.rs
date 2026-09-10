use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver}};
use super::text_selection::TextLine;
use crate::services::visual_pipeline::{VisualPipelineRuntime, types::{OcrRequest, TranslationRequest}};

pub struct AnalysisOutput {
    pub lines: Vec<TextLine>,
    pub translated: Option<Vec<TextLine>>,
}
pub struct AnalysisJob {
    pub receiver: Receiver<Result<AnalysisOutput, String>>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for AnalysisJob {
    fn drop(&mut self) { self.cancelled.store(true, Ordering::Relaxed); }
}

pub fn start(
    vp: Arc<VisualPipelineRuntime>, rgba: Vec<u8>, rect: [i32; 4],
    translate: bool, existing: Vec<TextLine>,
) -> Result<AnalysisJob, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = cancelled.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new().name("CropperAnalysis".into()).spawn(move || {
        let result = (|| {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            runtime.block_on(async {
                let lines = if existing.is_empty() {
                    let id = uuid::Uuid::new_v4().to_string();
                    let path = vp.artifact_dir().join(format!("{id}.png"));
                    let image = image::RgbaImage::from_raw((rect[2]-rect[0]) as u32, (rect[3]-rect[1]) as u32, rgba)
                        .ok_or("Invalid OCR image dimensions")?;
                    image.save(&path).map_err(|e| e.to_string())?;
                    let receipt = vp.ocr(OcrRequest { provider_id: None, artifact_id: id }).await;
                    let _ = std::fs::remove_file(&path);
                    let receipt = receipt?;
                    let count = receipt.blocks.len().max(1) as i32;
                    let mut lines = Vec::new();
                    for (index, block) in receipt.blocks.into_iter().enumerate() {
                        if block.text.trim().is_empty() { continue; }
                        let bounds = block.box_.filter(|b| b.len() >= 4 && b.iter().all(|v| v.is_finite()));
                        let r = if let Some(b) = bounds {
                            if b.len() >= 8 {
                                let xs = [b[0], b[2], b[4], b[6]];
                                let ys = [b[1], b[3], b[5], b[7]];
                                let x_min = xs.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                                let x_max = xs.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                                let y_min = ys.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                                let y_max = ys.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                                [rect[0] + x_min.floor() as i32, rect[1] + y_min.floor() as i32,
                                 rect[0] + x_max.ceil() as i32, rect[1] + y_max.ceil() as i32]
                            } else {
                                [rect[0] + b[0].floor() as i32, rect[1] + b[1].floor() as i32,
                                 rect[0] + b[2].ceil() as i32, rect[1] + b[3].ceil() as i32]
                            }
                        } else {
                            let row = (rect[3]-rect[1])/count;
                            [rect[0], rect[1]+index as i32*row, rect[2], rect[1]+(index as i32+1)*row]
                        };
                        lines.push(TextLine { rect: [r[0].max(rect[0]), r[1].max(rect[1]), r[2].min(rect[2]), r[3].min(rect[3])], text: block.text });
                    }
                    if lines.is_empty() && !receipt.text.trim().is_empty() {
                        let count = receipt.text.lines().count().max(1) as i32;
                        for (i, line) in receipt.text.lines().enumerate() {
                            lines.push(TextLine { text: line.into(), rect: [rect[0], rect[1]+i as i32*(rect[3]-rect[1])/count,
                                rect[2], rect[1]+(i as i32+1)*(rect[3]-rect[1])/count] });
                        }
                    }
                    lines.sort_by_key(|line| (line.rect[1], line.rect[0]));
                    lines
                } else { existing };
                if lines.is_empty() { return Err("选区内未识别到文字，请调整选区或 OCR 配置后重试".into()); }
                let mut translated = None;
                if translate {
                    let settings = vp.settings();
                    let mut output = Vec::with_capacity(lines.len());
                    for line in &lines {
                        if cancellation.load(Ordering::Relaxed) { return Err("已取消".into()); }
                        let result = tokio::time::timeout(std::time::Duration::from_secs(60), vp.translate(TranslationRequest {
                            provider_id: None, text: line.text.clone(), source_language: None,
                            target_language: settings.default_target_language.clone(), model: None, system_prompt: None,
                        })).await.map_err(|_| "翻译超时，请重试")??;
                        if result.translated_text.trim().is_empty() { return Err("翻译服务返回空内容".into()); }
                        output.push(TextLine { rect: line.rect, text: result.translated_text });
                    }
                    translated = Some(output);
                }
                Ok(AnalysisOutput { lines, translated })
            })
        })();
        let _ = sender.send(result);
    }).map_err(|e| e.to_string())?;
    Ok(AnalysisJob { receiver, cancelled })
}
