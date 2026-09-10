use std::collections::HashMap;
use std::process::Stdio;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use crate::services::agent::proc::agent_command;
use crate::services::agent::models::Model;

pub struct AcpDiscoveryProvider {
    pub default_bin: String,
    pub client_name: String,
    pub extra_env: Vec<(&'static str, &'static str)>,
    pub acp_args: Vec<String>,
    pub select_auth_method:
        Option<Box<dyn Fn(Value, &[(String, String)]) -> Result<String, String> + Send + Sync>>,
}

pub async fn discover_acp_models(
    executable_path: Option<&str>,
    provider: AcpDiscoveryProvider,
) -> Result<Vec<Model>, String> {
    let bin = executable_path.unwrap_or(&provider.default_bin);
    let mut cmd = agent_command(bin);
    cmd.args(&provider.acp_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut envs: Vec<(String, String)> = std::env::vars().collect();
    for (k, v) in provider.extra_env {
        cmd.env(k, v);
        envs.push((k.to_string(), v.to_string()));
    }

    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut scanner = BufReader::new(stdout).lines();

    let mut next_id = 1;

    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": next_id,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": { "name": provider.client_name, "version": "0.1.0" },
            "clientCapabilities": {}
        }
    });
    next_id += 1;

    async fn request_acp(
        stdin: &mut tokio::process::ChildStdin,
        scanner: &mut tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
        msg: Value,
        expected_id: i64,
    ) -> Result<Value, String> {
        let mut data = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        data.push(b'\n');
        stdin.write_all(&data).await.map_err(|e| e.to_string())?;

        while let Ok(Some(line)) = scanner.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(env) = serde_json::from_str::<Value>(trimmed) {
                if env.get("id").and_then(|i| i.as_i64()) == Some(expected_id) {
                    if let Some(err) = env.get("error") {
                        if !err.is_null() {
                            return Err(err.to_string());
                        }
                    }
                    if let Some(res) = env.get("result") {
                        return Ok(res.clone());
                    }
                    return Err("No result".to_string());
                }
            }
        }
        Err("EOF".to_string())
    }

    let init_result = request_acp(&mut stdin, &mut scanner, init_msg, next_id - 1).await?;

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("tempdir: {}", e))?;

    if let Some(select) = provider.select_auth_method {
        let method_id = select(init_result, &envs)?;
        let auth_msg = json!({
            "jsonrpc": "2.0",
            "id": next_id,
            "method": "authenticate",
            "params": {
                "methodId": method_id,
                "_meta": { "headless": true }
            }
        });
        next_id += 1;
        request_acp(&mut stdin, &mut scanner, auth_msg, next_id - 1).await?;
    }

    let session_msg = json!({
        "jsonrpc": "2.0",
        "id": next_id,
        "method": "session/new",
        "params": {
            "cwd": tmp_dir.path().to_string_lossy(),
            "mcpServers": []
        }
    });
    let session_result = request_acp(&mut stdin, &mut scanner, session_msg, next_id).await?;

    let _ = child.kill().await;

    Ok(parse_acp_session_new_models(&session_result))
}

fn parse_acp_session_new_models(raw: &Value) -> Vec<Model> {
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let extract_current = |v: &Value| -> String {
        v.get("currentModelId")
            .and_then(|s| s.as_str())
            .or_else(|| v.get("current_model_id").and_then(|s| s.as_str()))
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let extract_models = |available: &Value,
                          current: &str,
                          out: &mut Vec<Model>,
                          seen: &mut std::collections::HashSet<String>| {
        if let Some(arr) = available.as_array() {
            for m in arr {
                let id = m
                    .get("modelId")
                    .and_then(|s| s.as_str())
                    .or_else(|| m.get("model_id").and_then(|s| s.as_str()))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() || seen.contains(&id) {
                    continue;
                }
                seen.insert(id.clone());

                let name = m
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let label = if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
                    id.clone()
                } else {
                    name
                };

                let provider = id.find(':').map(|idx| id[..idx].to_string());

                out.push(Model {
                    id: id.clone(),
                    label,
                    provider,
                    default: Some(id == current),
                    service_tiers: vec![],
                    thinking: None,
                });
            }
        }
    };

    if let Some(models_obj) = raw.get("models") {
        let current = extract_current(models_obj);
        if let Some(available) = models_obj
            .get("availableModels")
            .or_else(|| models_obj.get("available_models"))
        {
            extract_models(available, &current, &mut models, &mut seen);
        }
    }

    if models.is_empty() {
        if let Some(opts) = raw
            .get("configOptions")
            .or_else(|| raw.get("config_options"))
        {
            if let Some(arr) = opts.as_array() {
                for opt in arr {
                    let id = opt.get("id").and_then(|s| s.as_str()).unwrap_or("").trim();
                    let category = opt
                        .get("category")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .trim();
                    if !id.eq_ignore_ascii_case("model") && !category.eq_ignore_ascii_case("model")
                    {
                        continue;
                    }
                    let current = extract_current(opt);
                    if let Some(options) = opt.get("options") {
                        if let Some(o_arr) = options.as_array() {
                            for choice in o_arr {
                                let val = choice
                                    .get("value")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                                if val.is_empty() || seen.contains(&val) {
                                    continue;
                                }
                                seen.insert(val.clone());
                                let name = choice
                                    .get("name")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                                let label =
                                    if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
                                        val.clone()
                                    } else {
                                        name
                                    };

                                let provider = val.find(':').map(|idx| val[..idx].to_string());

                                models.push(Model {
                                    id: val.clone(),
                                    label,
                                    provider,
                                    default: Some(val == current),
                                    service_tiers: vec![],
                                    thinking: None,
                                });
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    models
}
