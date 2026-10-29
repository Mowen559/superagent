# Deep Hub Runtime and Durable Run Ledger Capability

## Purpose
This capability defines the unified execution runtime for SuperAgent (hub-rs), consolidating tasks, worker handles, cancellation tokens, and persistent session ledgers into a single source of truth.

## Requirements

### Requirement: Unified Session Ledger SSOT
The system SHALL maintain all active and completed task states exclusively inside the thread-safe `SessionLedger` to eliminate parallel dead copies.

#### Scenario: Task State Transition
- **WHEN** a task transitions from `Submitted` to `Running`
- **THEN** runtime updates the ledger monotonically and broadcasts lifecycle events across the unified SSE channel

### Requirement: Cascading Cancellation Propagation
The system MUST propagate cancellation tokens through all long-running asynchronous worker loops to immediately abort operations upon user request.

#### Scenario: User Cancellation Signal
- **WHEN** user triggers a task abort
- **THEN** CancellationToken cancels child tasks and shuts down active subprocesses within 200ms
