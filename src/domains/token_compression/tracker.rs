//! 全局线程安全的 Token 节约追踪器

use super::CompressionResult;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

static GLOBAL_TRACKER: OnceLock<TokenSavingsTracker> = OnceLock::new();

pub struct TokenSavingsTracker {
    total_original_bytes: AtomicUsize,
    total_compressed_bytes: AtomicUsize,
    total_original_tokens: AtomicUsize,
    total_compressed_tokens: AtomicUsize,
    total_invocations: AtomicUsize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSavingsStats {
    pub total_original_bytes: usize,
    pub total_compressed_bytes: usize,
    pub total_original_tokens: usize,
    pub total_compressed_tokens: usize,
    pub total_tokens_saved: usize,
    pub overall_reduction_percent: f64,
    pub total_invocations: usize,
}

impl Default for TokenSavingsTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenSavingsTracker {
    pub fn new() -> Self {
        Self {
            total_original_bytes: AtomicUsize::new(0),
            total_compressed_bytes: AtomicUsize::new(0),
            total_original_tokens: AtomicUsize::new(0),
            total_compressed_tokens: AtomicUsize::new(0),
            total_invocations: AtomicUsize::new(0),
        }
    }

    pub fn global() -> &'static Self {
        GLOBAL_TRACKER.get_or_init(Self::new)
    }

    pub fn record_savings(&self, res: &CompressionResult) {
        self.total_original_bytes
            .fetch_add(res.original_bytes, Ordering::Relaxed);
        self.total_compressed_bytes
            .fetch_add(res.compressed_bytes, Ordering::Relaxed);
        self.total_original_tokens
            .fetch_add(res.original_tokens_est, Ordering::Relaxed);
        self.total_compressed_tokens
            .fetch_add(res.compressed_tokens_est, Ordering::Relaxed);
        self.total_invocations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> TokenSavingsStats {
        let orig_bytes = self.total_original_bytes.load(Ordering::Relaxed);
        let comp_bytes = self.total_compressed_bytes.load(Ordering::Relaxed);
        let orig_tokens = self.total_original_tokens.load(Ordering::Relaxed);
        let comp_tokens = self.total_compressed_tokens.load(Ordering::Relaxed);
        let invocations = self.total_invocations.load(Ordering::Relaxed);
        let saved_tokens = orig_tokens.saturating_sub(comp_tokens);
        let reduction_percent = if orig_bytes > 0 {
            ((orig_bytes.saturating_sub(comp_bytes)) as f64 / orig_bytes as f64) * 100.0
        } else {
            0.0
        };

        TokenSavingsStats {
            total_original_bytes: orig_bytes,
            total_compressed_bytes: comp_bytes,
            total_original_tokens: orig_tokens,
            total_compressed_tokens: comp_tokens,
            total_tokens_saved: saved_tokens,
            overall_reduction_percent: (reduction_percent * 10.0).round() / 10.0,
            total_invocations: invocations,
        }
    }

    pub fn reset(&self) {
        self.total_original_bytes.store(0, Ordering::Relaxed);
        self.total_compressed_bytes.store(0, Ordering::Relaxed);
        self.total_original_tokens.store(0, Ordering::Relaxed);
        self.total_compressed_tokens.store(0, Ordering::Relaxed);
        self.total_invocations.store(0, Ordering::Relaxed);
    }
}
