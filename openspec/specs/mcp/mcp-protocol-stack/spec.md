# Model Context Protocol Stack and Sandbox Capability

## Purpose
This capability specifies SuperAgent's dual MCP role as both a compliant MCP Server exposing local codebase graphs and an MCP Client hosting external tool servers.

## Requirements

### Requirement: Native Core Tool Name Collision Protection
The system SHALL prevent external MCP servers from overwriting top-10 native engineering tools (`view_file`, `write_to_file`, `run_command`).

#### Scenario: Tool Name Conflict
- **WHEN** an external MCP server registers a tool named `view_file`
- **THEN** registry automatically prepends the plugin namespace (`mcp_ext_view_file`) preserving the native tool

### Requirement: External Plugin Subprocess Timeout Protection
The system MUST enforce a 30-second execution timeout on external stdio MCP plugin tool invocations.

#### Scenario: Stalled MCP Tool Call
- **WHEN** an external tool hangs without responding for 30 seconds
- **THEN** manager terminates the child subprocess and returns a standard JSON-RPC timeout error
