use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use fastembed::EmbeddingModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    pub user_context_limit: Option<u32>,
    pub max_tool_output_chars: usize,
    pub embedding_model: String,
    pub retrieval_threshold: f32,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            user_context_limit: None,
            max_tool_output_chars: 1200,
            embedding_model: "AllMiniLML6V2".to_string(),
            retrieval_threshold: 0.6,
        }
    }
}

impl AgentSettings {
    pub fn get_path() -> PathBuf {
        let mut dir = dirs::data_dir().unwrap_or_else(|| std::env::current_dir().unwrap());
        dir.push(".gemini");
        dir.push("antigravity");
        dir.push("config");
        std::fs::create_dir_all(&dir).unwrap_or(());
        dir.push("agent_settings.json");
        dir
    }

    pub fn load() -> Self {
        if let Ok(data) = fs::read_to_string(Self::get_path()) {
            if let Ok(settings) = serde_json::from_str(&data) {
                return settings;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let data = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(Self::get_path(), data).map_err(|e| e.to_string())
    }

    /// Converts the string setting into the actual fastembed Enum
    pub fn get_embedding_model(&self) -> EmbeddingModel {
        match self.embedding_model.as_str() {
            "AllMiniLML6V2" => EmbeddingModel::AllMiniLML6V2,
            "BGEBaseENV15" => EmbeddingModel::BGEBaseENV15,
            "NomicEmbedTextV15" => EmbeddingModel::NomicEmbedTextV15,
            _ => EmbeddingModel::AllMiniLML6V2, // Fallback
        }
    }
}

