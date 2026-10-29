# Capability Certification and Golden Evals Benchmark Capability

## Purpose
This capability enforces the Capability Certification Matrix (`Certified`, `Experimental`, `Disabled`) and automated Golden Evals benchmark test suites.

## Requirements

### Requirement: Golden Evals 100% Pass DoD Gate
The system SHALL mandate all 5 major Golden Evals benchmarks (Isomorphic Rule, Surgical Patch, Anti-Crawl 403, Action-First, CoT Pruning) to achieve 100% pass score before binary distribution.

#### Scenario: Pre-Release Evaluation Run
- **WHEN** running `node scripts/run_evals.cjs`
- **THEN** all 5 test dimensions must return PASS with zero failures

### Requirement: Anti-Fabrication Physical Verification Gate
The system MUST fail capability certification if an agent outputs a delivery token or sharing command without producing physical verification artifacts.

#### Scenario: Hallucinated Delivery Token
- **WHEN** agent emits a mock sharing code without running generator tools
- **THEN** eval suite detects missing tool receipts and assigns a zero score
