use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Result, Context};

/// 混合检索引擎所需的文本特征抽象接口
pub trait EmbeddingService: Send + Sync {
    /// 计算单段文本的语义向量
    fn embed_text(&self, text: &str) -> Result<Vec<f32>>;
    
    /// 批量计算多段文本的语义向量（可大幅优化速度）
    fn embed_batch(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>>;
}

/// 本地极速版语义向量生成器（底层调用纯 Rust ONNX）
pub struct FastEmbedService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl FastEmbedService {
    /// 初始化本地 Embedding 模型（第一次运行时可能会下载模型文件约22MB）
    pub fn new() -> Result<Self> {
        // 使用轻量级的多语言或代码专用模型，例如 BGE 或是 AllMiniLML6V2
        let mut options = InitOptions::new(EmbeddingModel::AllMiniLML6V2);
        options.show_download_progress = true;
        let model = TextEmbedding::try_new(options).context("Failed to initialize FastEmbed model")?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

impl EmbeddingService for FastEmbedService {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        // Since TextEmbedding is NOT thread-safe for mutation/inference (some internal buffers),
        // we wrap it in a Mutex or we need a synchronous blocking call.
        // For our AI Agent CLI, a blocking call inside a tokio thread is acceptable for brief tasks.
        let mut model_lock = self.model.blocking_lock();
        let documents = vec![text];
        let embeddings = model_lock.embed(documents, None).context("Failed to generate embedding")?;
        
        if let Some(first) = embeddings.into_iter().next() {
            Ok(first)
        } else {
            anyhow::bail!("Embedding engine returned empty result")
        }
    }

    fn embed_batch(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>> {
        let mut model_lock = self.model.blocking_lock();
        let embeddings = model_lock.embed(texts, None).context("Failed to generate batch embeddings")?;
        Ok(embeddings)
    }
}

lazy_static::lazy_static! {
    static ref GLOBAL_EMBEDDING_SERVICE: Arc<std::sync::Mutex<Option<Arc<FastEmbedService>>>> = Arc::new(std::sync::Mutex::new(None));
}

/// 获取全局唯一的 EmbeddingService 实例 (同步)
pub fn get_embedding_service() -> Result<Arc<FastEmbedService>> {
    let mut lock = GLOBAL_EMBEDDING_SERVICE.lock().unwrap();
    if let Some(svc) = lock.as_ref() {
        return Ok(Arc::clone(svc));
    }
    
    // 如果没有初始化，则在此处初始化（可能耗时，例如下载模型）
    let svc = Arc::new(FastEmbedService::new()?);
    *lock = Some(Arc::clone(&svc));
    
    Ok(svc)
}

/// 计算两个向量的余弦相似度 (Cosine Similarity)
pub fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    if v1.len() != v2.len() || v1.is_empty() {
        return 0.0;
    }
    let mut dot_product = 0.0;
    let mut norm_v1 = 0.0;
    let mut norm_v2 = 0.0;
    
    for i in 0..v1.len() {
        dot_product += v1[i] * v2[i];
        norm_v1 += v1[i] * v1[i];
        norm_v2 += v2[i] * v2[i];
    }
    
    if norm_v1 == 0.0 || norm_v2 == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_v1.sqrt() * norm_v2.sqrt())
}
