content = """# Architecture Review Deep Fixes Phase 2

This plan aims to address the remaining issues found in the latest deep architecture review, focusing on security boundary tightening, agent runtime unification, destructive operations containment, and protocol contract accuracy.

## User Review Required

- **Destructive Operations Module**: A new Destructive Operations Seam will be introduced to unify Git destructive actions (checkout, rollback, apply patch) and file overwrites. We need to decide if this module should prompt for confirmation asynchronously or simply log and restrict paths strictly within the safe workspace limits.
- **Agent Capability Sync**: Sync operations will be atomic. What is the intended behavior for when configuration parsing fails during sync? Fallback to defaults or abort? Currently proposing aborting to avoid data loss.

## Open Questions

- Should src/services/claude_cli.rs be completely removed after migrating claude.rs to the new Agent Runtime Interface, or does it provide utilities still needed by the headless un_and_capture used by context/understand routes?
- For the JSON-RPC error format, MCP protocol expects an error object inside the response with code, message, and data (and no esult field). Should all MCP error responses follow standard JSON-RPC 2.0 error specs exactly?

## Proposed Changes

---

### Security Boundaries & Skills Sync

#### [MODIFY] [skills_sync.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/agent/skills_sync.rs)
- Add strict validation to 
ormalize_skill_dir_name or before it's used to reject . or .. and ensure it's a valid single-level directory name.
- Enforce the same validation on eq.skill_name during uninstallation.

---

### Hardcoded Credentials & Configuration

#### [MODIFY] [claude_settings.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/config/claude_settings.rs)
- Remove hardcoded ANTHROPIC_API_KEY from default_settings(). Default to empty or read from environment variable ANTHROPIC_API_KEY initially.

#### [MODIFY] [claude.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/routes/claude.rs) & [settings.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/routes/settings.rs)
- Ensure API keys are redacted (e.g., "sk-..." replaced with "sk-...***") when returning configurations over HTTP GET requests to prevent local credential theft.

---

### Agent Runtime & Execution Chains

#### [MODIFY] [claude.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/routes/claude.rs)
- Refactor the un handler. Instead of calling claude_cli::run_prompt which manually manages a duplicate TaskManager and discards cancellation, delegate to the unified Agent Runtime (AppServerBackend or ClaudeCodeBackend) so that it shares the global TaskManager, cancellation buses, and AgentMessage bus.

#### [MODIFY] [claude_cli.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/claude_cli.rs)
- In un_and_capture(), add explicit child.kill().await; inside the timeout Err block to prevent zombie child processes and runaway token usage.
- Deprecate/Remove un_prompt if no longer used.

---

### Destructive Operations Module

#### [NEW] [destructive_ops.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/destructive_ops.rs)
- Introduce a centralized module for file overwriting, deletion, and Git rollbacks.
- Require WorkspaceGuard approval before executing any destructive action.
- Add audit logging for all destructive actions.

#### [MODIFY] [git_service.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/git_service.rs)
- Refactor pply_patch, single-file recovery, and git checkout (rollback) to route through the new Destructive Operations Module.

---

### Protocol & Contract Fixes

#### [MODIFY] [proxy/mod.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/routes/proxy/mod.rs)
- Check how /openai/v1/completions handles prompt. Convert prompt (string/array) to messages format before proxying to Chat Completions endpoint.

#### [MODIFY] [openai_responses.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/protocol_converter/openai_responses.rs)
- Implement conversion logic for /openai/v1/responses (Codex) to standard /v1/chat/completions.

#### [MODIFY] [anthropic.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/protocol_converter/anthropic.rs)
- Convert Anthropic's token usage fields in non-streaming responses to OpenAI's standard prompt_tokens and completion_tokens instead of passthrough.

#### [MODIFY] [gemini.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/protocol_converter/gemini.rs)
- Ensure unique IDs for tool calls (e.g., append an index or UUID) to prevent conflicts during parallel tool invocation.
- Map Gemini's finish reason to standard stop correctly.

#### [MODIFY] [mcp_server.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/mcp_server.rs)
- Update error responses to comply with standard JSON-RPC 2.0 error format ({ "jsonrpc": "2.0", "id": ..., "error": { "code": ..., "message": ... } }) instead of wrapping errors inside esult.

---

### Atomic Config & Agent Capability

#### [MODIFY] [sync_utils.rs](file:///d:/mowen/deepcloud/super-agent/hub-rs/src/services/agent/sync_utils.rs)
- Implement atomic file writes for configuration syncing (write to .tmp and rename).
- Keep historical backups (e.g., using timestamps .bak.<timestamp>) instead of overwriting a single .bak file, and abort if JSON/TOML is invalid rather than falling back to empty structures.

#### [MODIFY] Agent Registration (e.g., egistry.rs / ackend.rs)
- Remove hardcoded .live = true and model_selection_supported = true defaults. Make each adapter explicitly declare its capabilities based on probing (e.g., checking if CLI exists).

## Verification Plan

### Automated Tests
- cargo test --all-targets to ensure no existing logic breaks.
- Add a new integration test for JSON-RPC error formats in mcp_server.rs.
- Add unit tests for 
ormalize_skill_dir_name rejecting . and ...

### Manual Verification
- Manually test trying to uninstall a skill named ../parent_dir via API to ensure it gets rejected.
- Manually hit /api/claude/settings and verify the API key is redacted.
- Verify git rollback properly routes through the destructive ops module and validates via WorkspaceGuard.
"""
with open('C:/Users/24778/.gemini/antigravity/brain/8ec610db-fa41-484e-a58d-647a4e591f36/implementation_plan.md', 'w', encoding='utf-8') as f:
    f.write(content)
