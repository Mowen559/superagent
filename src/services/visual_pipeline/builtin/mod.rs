pub mod common;
pub mod ocr;
pub mod screenshot;
pub mod translation;

#[cfg(feature = "vision-paddleocr")]
pub use ocr::PaddleOcrProvider;
pub use ocr::RapidOcrProvider;

#[cfg(feature = "vision-xcap")]
pub use screenshot::XcapScreenshotProvider;
pub use screenshot::SnowCaptureScreenshotProvider;

pub use translation::{DeepLTranslationProvider, GoogleTranslationProvider, HubLlmTranslationProvider};

#[cfg(test)]
pub(crate) use translation::normalize_language;

#[cfg(test)]
mod tests;
