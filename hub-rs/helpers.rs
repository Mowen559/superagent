// Helper: Standard OpenAI -> Gemini v1beta
fn convert_standard_to_gemini(mut standard: Value, fallback_model: String) -> Value {
    let mut contents = Vec::new();
    let mut system_instruction = None;

    if let Some(messages) = standard.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
            
            if role == "system" {
                let current_sys = system_instruction.unwrap_or_else(|| String::new());
                system_instruction = Some(format!("{}{}\n", current_sys, content));
            } else if role == "tool" {
                let name = msg.get("tool_call_id").and_then(|n| n.as_str()).unwrap_or("");
                let resp_obj: Value = serde_json::from_str(content).unwrap_or_else(|_| json!({}));
                contents.push(json!({
                    "role": "function",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": resp_obj
                        }
                    }]
                }));
            } else {
                let gemini_role = if role == "assistant" { "model" } else { "user" };
                
                let mut parts = Vec::new();
                if !content.is_empty() {
                    parts.push(json!({ "text": content }));
                }

                if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        if let Some(func) = tc.get("function") {
                            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let args_str = func.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                            let input: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
                            
                            let mut function_call = json!({
                                "name": name,
                                "args": input
                            });
                            
                            // Preserve thought_signature if it was stored in the function object
                            if let Some(sig) = func.get("thought_signature") {
                                function_call.as_object_mut().unwrap().insert("thought_signature".to_string(), sig.clone());
                            }

                            parts.push(json!({
                                "functionCall": function_call
                            }));
                        }
                    }
                }
                
                if parts.is_empty() {
                    parts.push(json!({ "text": "" }));
                }

                contents.push(json!({
                    "role": gemini_role,
                    "parts": parts
                }));
            }
        }
    }

    let model = standard.get("model").and_then(|m| m.as_str()).unwrap_or(&fallback_model);
    
    let mut gemini_req = json!({
        "contents": contents,
        "model": model,
        "generationConfig": {
            "temperature": standard.get("temperature").unwrap_or(&json!(0.7)),
        }
    });

    if let Some(sys) = system_instruction {
        gemini_req.as_object_mut().unwrap().insert("systemInstruction".to_string(), json!({
            "parts": [{"text": sys.trim_end()}]
        }));
    }

    if let Some(tools) = standard.get("tools").and_then(|t| t.as_array()) {
        let mut gemini_tools = Vec::new();
        let mut function_declarations = Vec::new();
        
        for tool in tools {
            if let Some(func) = tool.get("function") {
                function_declarations.push(json!({
                    "name": func.get("name").unwrap_or(&json!("")),
                    "description": func.get("description").unwrap_or(&json!("")),
                    "parameters": func.get("parameters").unwrap_or(&json!({}))
                }));
            }
        }
        
        if !function_declarations.is_empty() {
            gemini_tools.push(json!({ "functionDeclarations": function_declarations }));
        }
        
        gemini_req.as_object_mut().unwrap().insert("tools".to_string(), json!(gemini_tools));
    }

    gemini_req
}

// Helper: Gemini v1beta -> Standard OpenAI
fn convert_gemini_to_standard(gemini: Value) -> Result<Value, anyhow::Error> {
    let mut text_content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(candidates) = gemini.get("candidates").and_then(|c| c.as_array()) {
        if let Some(first) = candidates.first() {
            if let Some(parts) = first.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        text_content.push_str(text);
                    } else if let Some(func_call) = part.get("functionCall") {
                        let name = func_call.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let empty_args = json!({});
                        let args = func_call.get("args").unwrap_or(&empty_args);
                        
                        let id = format!("call_{}", name);
                        
                        let mut function_obj = json!({
                            "name": name,
                            "arguments": serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                        });

                        // Propagate thought_signature if present from Gemini response
                        if let Some(sig) = func_call.get("thought_signature") {
                            function_obj.as_object_mut().unwrap().insert("thought_signature".to_string(), sig.clone());
                        }
                        
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": function_obj
                        }));
                    }
                }
            }
        }
    }

    let mut message = json!({
        "role": "assistant"
    });

    if !text_content.is_empty() || tool_calls.is_empty() {
        message["content"] = json!(text_content);
    } else {
        message["content"] = Value::Null;
    }

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    Ok(json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": "gemini",
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if !tool_calls.is_empty() { "tool_calls" } else { "stop" }
        }]
    }))
}
