use serde_json::{json, Value};
use crate::domains::token_compression::{compress, FilterLevel};
use crate::services::brain::tool_execution_gateway::ToolExecutionRequest;

/// Intelligently compresses tool results to fit within context limits.
/// Returns (compressed_result, was_modified).
pub fn intelligent_compress(
    request: &ToolExecutionRequest,
    mut result: Value,
    max_chars: usize,
) -> (Value, bool) {
    let serialized = serde_json::to_string(&result).unwrap_or_default();
    let original_len = serialized.chars().count();
    if original_len <= max_chars {
        return (result, false);
    }

    let mut modified = false;

    // 1. Domain-Specific RTK Compression
    if request.name == "view_file" {
        if let Some(path) = request.arguments.get("AbsolutePath").and_then(|v| v.as_str()) {
            if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
                // Preserves code bodies by default with Minimal mode; Aggressive mode is only used when explicitly requested for architecture summary.
                let level = if request.arguments.get("SummaryMode").and_then(|v| v.as_bool()).unwrap_or(false)
                    || request.arguments.get("ArchitectureOnly").and_then(|v| v.as_bool()).unwrap_or(false) {
                    FilterLevel::Aggressive
                } else {
                    FilterLevel::Minimal
                };
                let filter_result = compress(content, path, level);
                result["content"] = json!(filter_result.content);
                result["rtk_compression"] = json!(level.to_string());
                result["reduction_percent"] = json!(filter_result.reduction_percent);
                result["tokens_saved"] = json!(filter_result.original_tokens_est.saturating_sub(filter_result.compressed_tokens_est));
                modified = true;
            }
        }
    } else if request.name == "run_command" {
        if let Some(content) = result.get("stdout").and_then(|v| v.as_str()) {
            let filter_result = crate::domains::token_compression::compress_terminal_output(content, 200);
            result["stdout"] = json!(filter_result.content);
            result["rtk_compression"] = json!("SmartLog");
            result["reduction_percent"] = json!(filter_result.reduction_percent);
            result["tokens_saved"] = json!(filter_result.original_tokens_est.saturating_sub(filter_result.compressed_tokens_est));
            modified = true;
        }
    }

    // 2. Re-evaluate size
    let new_serialized = serde_json::to_string(&result).unwrap_or_default();
    if new_serialized.chars().count() <= max_chars {
        return (result, modified);
    }

    // 3. Fallback: Graceful Truncation
    let head_chars = max_chars.saturating_mul(2) / 3;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let chars: Vec<char> = serialized.chars().collect();
    
    let head: String = chars[..head_chars].iter().collect();
    let tail: String = chars[chars.len().saturating_sub(tail_chars)..].iter().collect();
    
    let final_result = json!({
        "output": format!("{}\n… [tool result truncated] …\n{}", head, tail),
        "truncated": true,
        "original_chars": original_len,
        "retrieval_tool": "read_tool_result",
    });

    (final_result, true)
}
