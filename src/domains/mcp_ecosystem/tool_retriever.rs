use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub is_core: bool, // Core tools (like shell, fs) are always injected
}

pub struct ToolRetriever {
    registered_tools: Vec<ToolMetadata>,
    tool_embeddings: Vec<Vec<f32>>,
}

impl ToolRetriever {
    pub fn new() -> Self {
        Self {
            registered_tools: Vec::new(),
            tool_embeddings: Vec::new(),
        }
    }

    /// Register external Harness/MCP tools and compute their embeddings.
    pub async fn register_and_embed(&mut self, tools: Vec<ToolMetadata>) -> Result<(), String> {
        let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .map_err(|e| format!("Failed to init embedding model: {}", e))?;

        let descriptions: Vec<String> = tools.iter().map(|t| t.description.clone()).collect();
        
        if !descriptions.is_empty() {
            let embeddings = model.embed(descriptions, None).map_err(|e| e.to_string())?;
            self.tool_embeddings.extend(embeddings);
        }
        self.registered_tools.extend(tools);
        
        Ok(())
    }

    /// Retrieve the top K most relevant tools for a given user prompt.
    pub async fn retrieve_top_k(&self, prompt: &str, k: usize) -> Result<Vec<ToolMetadata>, String> {
        // 1. Always include core tools
        let mut selected: Vec<ToolMetadata> = self.registered_tools.iter()
            .filter(|t| t.is_core)
            .cloned()
            .collect();

        // 2. Filter non-core tools for semantic search
        let non_core: Vec<(usize, &ToolMetadata)> = self.registered_tools.iter()
            .enumerate()
            .filter(|(_, t)| !t.is_core)
            .collect();

        if non_core.is_empty() {
            return Ok(selected);
        }

        // 3. Embed the prompt
        let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .map_err(|e| e.to_string())?;
        let query_embeds = model.embed(vec![prompt], None).map_err(|e| e.to_string())?;
        let query_vec = query_embeds.into_iter().next().unwrap();

        // 4. Compute cosine similarity
        let mut scored_tools: Vec<(&ToolMetadata, f32)> = non_core.into_iter().map(|(idx, tool)| {
            let tool_vec = &self.tool_embeddings[idx];
            let mut dot = 0.0;
            let mut ma = 0.0;
            let mut mb = 0.0;
            for (a, b) in query_vec.iter().zip(tool_vec.iter()) {
                dot += a * b;
                ma += a * a;
                mb += b * b;
            }
            let sim = if ma > 0.0 && mb > 0.0 { dot / (ma.sqrt() * mb.sqrt()) } else { 0.0 };
            (tool, sim)
        }).collect();

        // 5. Sort by descending similarity and pick top K
        scored_tools.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        for (tool, _sim) in scored_tools.into_iter().take(k) {
            selected.push(tool.clone());
        }

        Ok(selected)
    }
}

