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
use super::common::*;

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
pub(crate) fn windows_gdi_capture(
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


