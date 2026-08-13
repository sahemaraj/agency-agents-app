# Unified Task Search Specification

## Purpose

Lets users describe a task once and find explainable, exact Agent and Skill matches through the existing global command palette while preserving local-first inspection and deployment safety.

## Requirements

### Requirement: Task search shares the global command palette
The system SHALL accept a bounded task description in the existing global command palette and SHALL present matching Agents and Skills without removing matching application commands.

#### Scenario: User describes a task
- **WHEN** the user enters enough task text to request recommendations
- **THEN** the palette shows separate Agent, Skill, and matching command groups in one keyboard-navigable result list

#### Scenario: User enters command text
- **WHEN** the query matches an existing application command
- **THEN** that command remains available alongside any catalog matches

### Requirement: Recommendations are local, deterministic, and exact
The system SHALL rank validated installable Agent and Skill packages using bounded local catalog metadata. The same catalog state and normalized task input MUST produce the same ordered exact references, scores, and structured reasons without model inference, network access, or mutation.

#### Scenario: Catalog contains matching metadata
- **WHEN** multiple validated installable packages match the normalized task tokens
- **THEN** results are ordered by deterministic score and stable exact-reference tie-breakers

#### Scenario: Package is not installable
- **WHEN** a matching Agent or Skill package is rejected or otherwise not installable
- **THEN** the package is excluded from task recommendations

#### Scenario: Search is performed offline
- **WHEN** the user requests task recommendations while network access is unavailable or disabled
- **THEN** recommendation behavior remains available from current local catalog state

### Requirement: Every recommendation explains its match
Each recommended item SHALL display its artifact type, display name, source provenance, and a human-readable explanation derived from its structured match reasons. Raw internal reason tokens SHALL NOT be the only explanation shown to the user.

#### Scenario: Name or description matches
- **WHEN** a result scores through name, description, taxonomy, language, or preferred-source metadata
- **THEN** the palette explains the applicable match categories in user-facing language

#### Scenario: Duplicate display names exist
- **WHEN** two sources contain Agents or Skills with the same display name
- **THEN** each result remains distinguishable by artifact type and source provenance and activates its exact reference

### Requirement: Activation hands off to existing safe workflows
Activating an Agent or Skill recommendation SHALL open that exact item in its existing workspace. The palette SHALL NOT install, update, execute, or otherwise mutate the item.

#### Scenario: Agent recommendation is activated
- **WHEN** the user activates an Agent result
- **THEN** the Agents workspace opens the exact recommended Agent for inspection and its existing deployment actions remain responsible for mutation approval

#### Scenario: Skill recommendation is activated
- **WHEN** the user activates a Skill result
- **THEN** the Skills workspace opens the exact recommended Skill for inspection and its existing trust, plan, and approval controls remain responsible for mutation

### Requirement: Search lifecycle is bounded and accessible
The task search SHALL enforce its input bound, avoid stale asynchronous results, and expose loading, failure, no-match, and result-count changes to assistive technology while preserving Escape, arrow-key, Enter, pointer, and focus behavior of the command palette.

#### Scenario: Query changes during recommendation
- **WHEN** an earlier recommendation request finishes after the user has changed or cleared the query
- **THEN** the stale response is ignored and does not replace results for the current query

#### Scenario: Recommendation lookup fails
- **WHEN** the local recommendation command returns an error
- **THEN** existing application commands remain usable and the palette presents a bounded retryable error without closing

#### Scenario: Input exceeds the bound
- **WHEN** the task description reaches the supported maximum length
- **THEN** additional input is prevented or rejected before unbounded processing and the supported limit is communicated accessibly
