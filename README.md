# SuperAgent (hub-rs) / AI-index

SuperAgent (`hub-rs`) is an advanced agentic backend and MCP (Model Context Protocol) server designed to orchestrate LLM workflows, optimize token usage (via RTK), and provide a long-term "Supermemory" for autonomous agents.

## Features

- **LLM Proxy & Token Compression (RTK)**: Intercepts and compresses prompts before sending them to LLMs to save tokens.
- **Supermemory**: Persists user preferences, architecture constraints, and key project context in a local JSON database, making agents smarter over time.
- **Requirement Analysis**: Understands user requirements and calculates the "Blast Radius" of code changes by leveraging a full-project knowledge graph.
- **MCP Server**: Provides a seamless interface for autonomous AI agents (like Antigravity or Claude) to interact with the project context, git history, and supermemory.
- **Shadow Git**: Creates temporary commits/snapshots before destructive operations, allowing safe rollbacks.
- **Cross-Platform**: Written in Rust, extremely fast and lightweight.

## Getting Started

### Installation

```bash
cargo build --release
```

### Usage

Start the MCP server:
```bash
hub-rs mcp-server
```

Configure your LLM provider and quota:
```bash
hub-rs config auth
hub-rs config quota
```

## Architecture & Testing Benchmark

- Architecture overview and current implementation boundaries: [系统架构设计](系统架构设计.md).
- **Core Capability Benchmark & Multi-Requirement Challenge Ledger**: [SUPERAGENT_BENCHMARK_AND_EVALUATION.md](docs/SUPERAGENT_BENCHMARK_AND_EVALUATION.md)
  - Records real-world testing demands, failure modes, anti-hallucination DoD gates, and cognitive evolution trajectories.

This project is built using Rust and `axum` for HTTP serving, integrating seamlessly with `reqwest` for external LLM API calls and `tree-sitter` for codebase analysis.
