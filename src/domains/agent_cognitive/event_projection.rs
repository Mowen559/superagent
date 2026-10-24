use serde_json::Value;
use std::collections::HashMap;
use crate::services::brain::events::SessionEvent;

/// Helper: Formats a compact fingerprint for a folded tool result
pub fn format_folded_tool_result(tool_name: &str, result: &Value) -> String {
    let raw_str = if let Some(s) = result.as_str() {
        s.to_string()
    } else {
        result.to_string()
    };
    let char_count = raw_str.len();
    let line_count = raw_str.lines().count();

    // Check if result indicates success or error
    let status_hint = if let Some(status) = result.get("status").and_then(|v| v.as_str()) {
        format!("status: {}", status)
    } else if let Some(code) = result.get("exit_code").and_then(|v| v.as_i64()) {
        format!("exit_code: {}", code)
    } else if raw_str.to_lowercase().contains("error") || raw_str.to_lowercase().contains("fail") {
        "with error warnings".to_string()
    } else {
        "success".to_string()
    };

    // Extract first line or brief excerpt
    let first_line = raw_str
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('{') && !l.starts_with('"'))
        .unwrap_or("");
    let brief_hint = if !first_line.is_empty() && first_line.len() <= 80 {
        format!("; summary: \"{}\"", first_line)
    } else {
        String::new()
    };

    format!(
        "[Tool Result Folded]: {} completed ({}, {} lines/{} chars folded to conserve context{})",
        tool_name, status_hint, line_count, char_count, brief_hint
    )
}

/// Helper: Prunes historical CoT (<think>...</think>) reasoning blocks
pub fn prune_cot_thinking(text: &str) -> String {
    if let (Some(start_tag), Some(end_tag)) = (text.find("<think>"), text.find("</think>")) {
        if start_tag < end_tag {
            let before = &text[..start_tag];
            let after = &text[end_tag + "</think>".len()..];
            let pruned = format!("{}{}", before.trim(), after.trim());
            let final_content = pruned.trim();
            if final_content.is_empty() {
                return "[Historical reasoning completed]".to_string();
            } else {
                return format!("[Reasoning: completed]\n{}", final_content);
            }
        }
    }

    if text.starts_with("<think>") {
        return "[Historical reasoning completed]".to_string();
    }

    if text.len() > 1200 {
        let prefix: String = text.chars().take(250).collect();
        return format!("{}...\n[Historical reasoning folded ({} chars)]", prefix.trim(), text.len());
    }

    text.to_string()
}

/// Event Sourcing 投影器：具备时间衰减指纹化折叠与思考链修剪的高阶投影
pub fn project_events_with_micro_compaction(
    events: &[SessionEvent],
    active_turns: usize,
) -> Vec<Value> {
    if events.is_empty() {
        return Vec::new();
    }

    // 1. 建立 ToolCall ID 到 Tool 名字与参数的快速查找映射
    let mut call_names: HashMap<String, String> = HashMap::new();
    // 2. 为每个事件分配逻辑轮次 (logical turn)
    let mut event_turns = vec![0_usize; events.len()];
    let mut current_turn = 0_usize;
    let mut in_tool_batch = false;

    for (i, evt) in events.iter().enumerate() {
        match evt {
            SessionEvent::UserInput(_) | SessionEvent::UserInputWithImages { .. } => {
                if i > 0 {
                    current_turn += 1;
                }
                in_tool_batch = false;
                event_turns[i] = current_turn;
            }
            SessionEvent::ModelThought(_) => {
                if i > 0 && !in_tool_batch {
                    current_turn += 1;
                }
                in_tool_batch = false;
                event_turns[i] = current_turn;
            }
            SessionEvent::ToolCall { id, name, .. } => {
                call_names.insert(id.clone(), name.clone());
                if !in_tool_batch && i > 0 {
                    current_turn += 1;
                    in_tool_batch = true;
                }
                event_turns[i] = current_turn;
            }
            SessionEvent::ToolResult { id: _, .. } => {
                // ToolResult 属于触发它的同一个逻辑轮次
                event_turns[i] = current_turn;
            }
            _ => {
                event_turns[i] = current_turn;
            }
        }
    }

    let max_turn = current_turn;
    let active_turn_threshold = max_turn.saturating_sub(active_turns.saturating_sub(1));

    let mut projected = Vec::with_capacity(events.len());

    for (i, evt) in events.iter().enumerate() {
        let is_active = event_turns[i] >= active_turn_threshold;

        if is_active {
            // 活跃窗口：100% 原始投射
            projected.push(evt.to_llm_message());
        } else {
            // 历史窗口：微剪枝与指纹化折叠
            match evt {
                SessionEvent::ToolResult { id, result } => {
                    let raw_str = if let Some(s) = result.as_str() {
                        s.to_string()
                    } else {
                        result.to_string()
                    };

                    if raw_str.len() <= 300 {
                        // 小型返回值直接保留
                        projected.push(evt.to_llm_message());
                    } else {
                        // 动态指纹化折叠
                        let tool_name = call_names.get(id).map(|s| s.as_str()).unwrap_or("tool");
                        let folded_content = format_folded_tool_result(tool_name, result);
                        projected.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": folded_content
                        }));
                    }
                }
                SessionEvent::ModelThought(thought) => {
                    let pruned = prune_cot_thinking(thought);
                    projected.push(serde_json::json!({
                        "role": "assistant",
                        "content": pruned
                    }));
                }
                _ => {
                    projected.push(evt.to_llm_message());
                }
            }
        }
    }

    projected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_micro_compaction_folds_older_large_tool_results_and_prunes_cot() {
        let large_output = "a".repeat(1500);
        let events = vec![
            // Turn 0
            SessionEvent::UserInput("Fix issue in server".to_string()),
            // Turn 1 (Historical)
            SessionEvent::ModelThought("<think>Let me deeply analyze the problem first...</think>I will inspect the file".to_string()),
            SessionEvent::ToolCall {
                id: "call-1".to_string(),
                name: "view_file".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call-1".to_string(),
                result: serde_json::json!({ "content": large_output }),
            },
            // Turn 2 (Historical if active_turns = 2)
            SessionEvent::ModelThought("Now checking the logs".to_string()),
            SessionEvent::ToolCall {
                id: "call-2".to_string(),
                name: "run_command".to_string(),
                arguments: serde_json::json!({ "cmd": "cargo check" }),
            },
            SessionEvent::ToolResult {
                id: "call-2".to_string(),
                result: serde_json::json!({ "status": "ok", "short": "pass" }), // <= 300 chars, not folded
            },
            // Turn 3 (Active)
            SessionEvent::ModelThought("Modifying the code".to_string()),
            SessionEvent::ToolCall {
                id: "call-3".to_string(),
                name: "replace_in_file".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
            SessionEvent::ToolResult {
                id: "call-3".to_string(),
                result: serde_json::json!({ "content": large_output }), // Active, so NOT folded!
            },
            // Turn 4 (Active)
            SessionEvent::ModelThought("All tasks completed!".to_string()),
        ];

        let projected = project_events_with_micro_compaction(&events, 2);
        assert_eq!(projected.len(), events.len());

        // 1. Turn 1 Thought should have <think> pruned
        let turn1_thought = projected[1]["content"].as_str().unwrap();
        assert!(!turn1_thought.contains("<think>"));
        assert!(!turn1_thought.contains("deeply analyze"));
        assert!(turn1_thought.contains("[Reasoning: completed]"));
        assert!(turn1_thought.contains("I will inspect the file"));

        // 2. Turn 1 ToolResult (1500 chars) should be folded to fingerprint summary
        let turn1_result = &projected[3];
        assert_eq!(turn1_result["role"], "tool");
        assert_eq!(turn1_result["tool_call_id"], "call-1");
        let turn1_res_content = turn1_result["content"].as_str().unwrap();
        assert!(turn1_res_content.contains("[Tool Result Folded]: view_file completed"));
        assert!(turn1_res_content.contains("folded to conserve context"));
        assert!(turn1_res_content.len() < 250);

        // 3. Turn 2 ToolResult (short) should remain verbatim
        let turn2_result = &projected[6];
        assert_eq!(turn2_result["tool_call_id"], "call-2");
        assert!(turn2_result["content"].as_str().unwrap().contains("pass"));

        // 4. Turn 3 ToolResult (large, but in active window) must NOT be folded
        let turn3_result = &projected[9];
        assert_eq!(turn3_result["tool_call_id"], "call-3");
        assert!(turn3_result["content"].as_str().unwrap().contains(&large_output));
    }
}
