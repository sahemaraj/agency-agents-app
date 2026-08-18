# Project Readiness Specification

## Purpose

Provides an explainable, source-aware project readiness report derived from explicit intended state and independent local evidence rather than inferred installation guesses.

## Requirements

### Requirement: Readiness requires an explicit exact baseline
The system SHALL store a bounded per-project baseline containing exact source-aware Agent and Skill references plus bounded instruction, MCP, and tool requirements. It MUST report Not configured when no baseline exists and MUST preserve opaque requirements as unverifiable rather than treating them as satisfied.

#### Scenario: Project has no baseline
- **WHEN** readiness is requested for a registered project without a stored baseline
- **THEN** the report is Not configured and identifies baseline creation as the next action

### Requirement: Readiness is derived from independent evidence
The system SHALL inspect Agent, aggregate roster, Skill, instruction, MCP, and tool evidence independently. Overall precedence SHALL be Not configured; otherwise Unavailable if required inspection failed; otherwise Needs attention if any required row fails or is unverifiable; otherwise Ready. Empty groups SHALL be Not required.

#### Scenario: One evidence source is unavailable
- **WHEN** a required MCP inspection fails while other evidence succeeds
- **THEN** the MCP group and overall report are Unavailable while successful evidence remains visible

#### Scenario: Every requirement is proven
- **WHEN** every non-empty requirement group has fresh exact evidence in its Ready state
- **THEN** the overall project report is Ready

### Requirement: Readiness evaluation is local and read-only
The system SHALL use local bounded reconciliation and inspection only. Evaluation MUST NOT write destinations, repair drift, refresh the network catalog, approve requests, or execute a tool CLI.

#### Scenario: Readiness finds missing artifacts
- **WHEN** reconciliation proves a required Agent, roster member, Skill, instruction, MCP server, or tool is missing
- **THEN** the report shows Needs attention and offers only a handoff to an existing reviewed workflow
