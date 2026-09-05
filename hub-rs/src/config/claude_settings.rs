use super::app_paths::AppPaths;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Claude CLI settings persisted at `{app_data_root}/settings/claude.json`.
///
/// The file follows the Claude Code `--settings` schema (env / model /
/// permissions / ...), so it is passed to the CLI verbatim and stays fully
/// user-editable. Kept as raw JSON instead of a typed struct because the
/// schema is open-ended.
pub fn settings_path() -> PathBuf {
    let app_paths = AppPaths::new();
    app_paths.data_path(&["settings", "claude.json"])
}

pub fn default_settings() -> Value {
    json!({
        "env": {
            "ANTHROPIC_BASE_URL": "http://127.0.0.1:8045",
            "ANTHROPIC_API_KEY": ""
        },
        "model": "gemini-pro-agent",
        "permissions": {
            "allow": [
                "mcp__codegraph__codegraph_search",
                "mcp__codegraph__codegraph_context",
                "mcp__codegraph__codegraph_callers",
                "mcp__codegraph__codegraph_callees",
                "mcp__codegraph__codegraph_impact",
                "mcp__codegraph__codegraph_node",
                "mcp__codegraph__codegraph_status"
            ]
        }
    })
}

pub fn read_settings() -> Value {
    let path = settings_path();
    if !path.exists() {
        return default_settings();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Tolerate a UTF-8 BOM left by external editors.
            let content = content.trim_start_matches('\u{feff}');
            serde_json::from_str(content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse Claude settings: {}, using defaults", e);
                default_settings()
            })
        }
        Err(e) => {
            tracing::warn!("Failed to read Claude settings: {}, using defaults", e);
            default_settings()
        }
    }
}

/// Ensure the settings file exists on disk. Called once at startup so the
/// configuration is always inspectable, mirroring `llm_settings`.
pub fn ensure_persisted() -> PathBuf {
    let path = settings_path();
    if !path.exists() {
        let app_paths = AppPaths::new();
        app_paths.ensure_data_dir(&["settings"]);
        if let Ok(json) = serde_json::to_string_pretty(&default_settings()) {
            match std::fs::write(&path, json) {
                Ok(_) => tracing::info!("Claude settings initialized at {}", path.display()),
                Err(e) => tracing::warn!("Failed to persist default Claude settings: {}", e),
            }
        }
    }
    path
}

/// Deep-merge a partial update into the current settings and persist.
pub fn write_settings(partial: Value) -> Value {
    let app_paths = AppPaths::new();
    app_paths.ensure_data_dir(&["settings"]);

    let merged = json_merge(read_settings(), partial);
    let path = settings_path();
    if let Ok(json) = serde_json::to_string_pretty(&merged) {
        std::fs::write(&path, json).ok();
    }
    merged
}

fn json_merge(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                if value.is_null() {
                    base_map.remove(&key);
                    continue;
                }
                let entry = base_map.entry(key).or_insert(Value::Null);
                *entry = json_merge(entry.take(), value);
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

// ---------------------------------------------------------------------------
// rtk PreToolUse hook wiring
//
// So the Claude CLI hub launches auto-routes its Bash tool output through rtk
// (token compression), we inject rtk's `PreToolUse` hook directly into hub's
// own `claude.json` (the `--settings` file passed to every `claude -p` run),
// rather than patching the user's global `~/.claude/settings.json`. The entry
// shape mirrors rtk `hooks/init.rs::insert_hook_entry`:
//   hooks.PreToolUse[] = { matcher: "Bash", hooks: [{ type: "command",
//                          command: "<rtk> hook claude" }] }
// ---------------------------------------------------------------------------

/// The command rtk expects Claude to invoke; we pin it to the absolute rtk
/// binary path so it resolves regardless of the child process PATH.
fn rtk_hook_command(rtk_bin: &Path) -> String {
    format!("\"{}\" hook claude", rtk_bin.display())
}

/// True when a settings value already carries an rtk `hook claude` PreToolUse
/// entry (matches any absolute-path variant, so re-runs are idempotent).
fn has_rtk_hook(settings: &Value) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(|p| p.as_array())
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hooks| {
                        hooks.iter().any(|hook| {
                            hook.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c.trim_end().ends_with("hook claude"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Report whether hub's claude.json currently carries the rtk hook.
pub fn rtk_hook_installed() -> bool {
    has_rtk_hook(&read_settings())
}

/// Whether the rtk hook is enabled (default true unless `rtkHook: false`).
pub fn rtk_hook_enabled() -> bool {
    read_settings()
        .get("rtkHook")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Idempotently reconcile the rtk PreToolUse hook in hub's claude.json against
/// the `rtkHook` toggle (default enabled). Returns whether the hook is present
/// after the call. `enable_override` forces the toggle when `Some`.
pub fn ensure_rtk_hook(rtk_bin: &Path, enable_override: Option<bool>) -> bool {
    let app_paths = AppPaths::new();
    app_paths.ensure_data_dir(&["settings"]);
    let mut settings = read_settings();

    let enabled = enable_override
        .or_else(|| settings.get("rtkHook").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    settings["rtkHook"] = json!(enabled);

    if enabled {
        if !has_rtk_hook(&settings) {
            insert_claude_hook(&mut settings, &rtk_hook_command(rtk_bin));
        }
    } else {
        remove_rtk_hook(&mut settings);
    }

    persist_settings(&settings);
    enabled && has_rtk_hook(&settings)
}

/// Insert the rtk PreToolUse/Bash hook, preserving any existing hooks.
fn insert_claude_hook(settings: &mut Value, hook_command: &str) {
    if !settings.is_object() {
        *settings = json!({});
    }
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let pre = hooks
        .as_object_mut()
        .unwrap()
        .entry("PreToolUse")
        .or_insert_with(|| json!([]));
    if let Some(arr) = pre.as_array_mut() {
        arr.push(json!({
            "matcher": "Bash",
            "hooks": [ { "type": "command", "command": hook_command } ]
        }));
    }
}

/// Strip any rtk `hook claude` entries and prune emptied structures.
fn remove_rtk_hook(settings: &mut Value) {
    let Some(pre) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PreToolUse"))
        .and_then(|p| p.as_array_mut())
    else {
        return;
    };
    for entry in pre.iter_mut() {
        if let Some(hooks) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) {
            hooks.retain(|hook| {
                !hook
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| c.trim_end().ends_with("hook claude"))
                    .unwrap_or(false)
            });
        }
    }
    // Drop matcher groups whose hooks are now empty.
    pre.retain(|entry| {
        entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|h| !h.is_empty())
            .unwrap_or(true)
    });
}

fn persist_settings(settings: &Value) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        std::fs::write(settings_path(), json).ok();
    }
}
