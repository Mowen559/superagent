# Code Graph Intelligence and Blast Radius Analysis Capability

## Purpose
This capability governs Tree-sitter AST extraction, SQLite graph relational persistence, and quantitative Blast Radius dependency impact calculation.

## Requirements

### Requirement: Graph Database Concurrent Concurrency Guard
The system SHALL operate the local SQLite graph database (`.codegraph/graph.db`) in WAL mode with connection busy-retry handling.

#### Scenario: Concurrent Graph Queries
- **WHEN** multiple agent threads query AST symbol paths simultaneously
- **THEN** SQLite executes reads concurrently without database locked errors

### Requirement: Critical Blast Radius Boundary Alert
The system MUST emit a critical risk alert whenever a proposed modification touches five or more cross-module system singletons.

#### Scenario: Wide-Impact Code Mutation
- **WHEN** proposed code changes affect core application layout or global audio singletons
- **THEN** impact analyzer marks risk level as `CRITICAL` and injects regression testing requirements
