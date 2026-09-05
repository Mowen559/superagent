---
name: hub-cli
description: "Master capability for using the hub CLI to orchestrate AI agents, analyze projects, and manage tasks."
---

# Hub CLI Skill

You are an AI assistant equipped with the `hub` CLI tool. `hub` (formerly hub-rs) is a powerful, locally-run AI orchestrator and project analyzer written in Rust.

## Core Capabilities

The `hub` CLI empowers you to execute advanced analysis, manage AI agents, handle MCP (Model Context Protocol) servers, and automate tasks. Once installed globally, it can be called using the `hub` command.

### Installation

If `hub` is not yet available in the PATH, you can install it automatically using the executable:
```bash
hub install
```
*(This automatically copies the binary to a global location and registers it in the system PATH.)*

## Commands and Usage

### 1. Project Analysis & Understanding
Hub provides high-level understanding and explosion radius analysis based on the AST and graph data.
*(Note: Use these commands to gain deep context about the codebase you are working on, it automatically detects the project's primary language for accurate recommendations.)*
- **Explain Project**: `hub analyze project` (or via API)
- **Impact/Explosion Radius Analysis**: Use this to understand what parts of the system are affected by a planned change.

### 2. Task Management (`hub task`)
Use `hub task` to manage background and foreground tasks.
```bash
hub task list
hub task create --name "refactor-auth"
```

### 3. Agent Management (`hub agent`)
You can orchestrate sub-agents and view existing agent configurations.
```bash
hub agent list
hub agent run <agent-id>
```

### 4. Skill Operations (`hub skill`)
Manage and discover skills that agents can use.
```bash
hub skill list
hub skill add <skill-path>
```

### 5. MCP Operations (`hub mcp` & `hub mcp-server`)
Integrate with Model Context Protocol to fetch contextual data or provide contextual capabilities.
```bash
hub mcp list
hub mcp-server start
```

### 6. Configuration & Models (`hub config`)
Manage LLM models, API keys, and supplier configurations directly from the terminal.
```bash
hub config set-model --provider openai --model gpt-4o
hub config view
```

## Best Practices for AI Agents

1. **Auto-Language Detection Context**: When you ask `hub` to analyze the project, it automatically runs `detect_primary_language()` and instructs its internal LLM using language-specific idioms (e.g., Python vs Rust ecosystem conventions). Trust the output it provides for language-specific tasks.
2. **Global Availability**: Always verify if `hub` is in your PATH. If the user just built it, run `./target/release/hub install` to make it globally available for your sub-agents.
3. **HTTP parity**: Everything that can be done via the `hub` CLI also has HTTP endpoints exposed when `hub service` is running. Use the service mode if you need a persistent daemon for multi-agent workflows.

## When to use this skill
- Use `hub` whenever you need to delegate a complex workflow, analyze large repositories beyond basic grep, or manage autonomous sub-agent processes.
- Use it to manage the user's local AI environment configs (like LLM keys and limits).
