# Canonical LLM Gateway and Stream Optimization Capability

## Purpose
This capability defines the Canonical format mediation layer in Rust, consolidating provider adapters, RTK compression routines, and SSE keep-alive heartbeats.

## Requirements

### Requirement: Single Canonical Adapter Protocol
The system SHALL translate all outbound model requests through a single CanonicalRequest structure with exactly one concrete adapter per provider.

#### Scenario: Provider Stream Chunk Decoding
- **WHEN** incoming streaming chunks arrive from Anthropic or OpenAI endpoints
- **THEN** adapter translates chunks into uniform `thought_chunk` and `text_chunk` SSE events

### Requirement: Low-Latency SSE Heartbeat
The system MUST emit periodic keep-alive comments when deep reasoning loops produce no visible text tokens for more than 3 seconds.

#### Scenario: Extended Slow Brain Reasoning
- **WHEN** slow brain inference takes extended compute time without token emission
- **THEN** gateway emits `: keep-alive\n\n` comments every 2 seconds preventing client disconnects
