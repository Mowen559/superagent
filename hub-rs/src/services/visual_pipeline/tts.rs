use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tts::Tts;
use once_cell::sync::Lazy;

#[derive(Debug, Clone, Deserialize)]
pub struct TtsRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TtsResponse {
    pub success: bool,
    pub error: Option<String>,
}

static TTS_ENGINE: Lazy<Mutex<Option<Tts>>> = Lazy::new(|| {
    match Tts::default() {
        Ok(tts) => Mutex::new(Some(tts)),
        Err(e) => {
            eprintln!("Failed to initialize TTS engine: {}", e);
            Mutex::new(None)
        }
    }
});

pub fn speak_text(text: &str) -> Result<(), String> {
    let mut engine_guard = TTS_ENGINE.lock().unwrap();
    if let Some(engine) = engine_guard.as_mut() {
        engine.speak(text, true).map_err(|e| format!("TTS Speak Error: {}", e))?;
        Ok(())
    } else {
        Err("TTS Engine not initialized".to_string())
    }
}
