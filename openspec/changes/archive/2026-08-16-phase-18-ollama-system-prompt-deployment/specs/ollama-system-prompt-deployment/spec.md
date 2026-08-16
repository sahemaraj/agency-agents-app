## Purpose

Let users deploy an exact reviewed Agent prompt as a device-local Ollama model while preserving explicit approval, truthful reconciliation, and recoverable lifecycle guarantees.

## ADDED Requirements

### Requirement: Local Ollama inventory is bounded and passive
The system SHALL query only the fixed loopback Ollama service, SHALL list only already-present non-cloud models as eligible bases, and MUST NOT start Ollama, pull or push a model, or run inference while discovering deployment targets.

#### Scenario: Ollama is available
- **WHEN** the user opens local-model deployment and the loopback service returns installed models
- **THEN** the system shows a sorted, deduplicated list of eligible local base models without changing Ollama or app state

#### Scenario: Ollama is unavailable
- **WHEN** the loopback service is absent, times out, or returns an invalid or oversized response
- **THEN** the system explains that local-model deployment is unavailable and performs no mutation

#### Scenario: Cloud model is present
- **WHEN** Ollama inventory contains a cloud-tagged model
- **THEN** the system excludes that model from the eligible base-model list

### Requirement: Every lifecycle action has an immutable review plan
The system SHALL build a no-write plan for create, update, or remove that identifies the exact Agent source reference, operation, selected local base model, deterministic app-owned target name, device-wide scope, current reconciliation state, exact system-prompt preview, warnings, blockers, rollback availability, and a revision derived from all mutation-relevant inputs.

#### Scenario: User reviews a new deployment
- **WHEN** the user selects an installable Agent and an eligible base model
- **THEN** the system previews the exact system prompt and the deterministic `agency-agents/` target name before offering confirmation

#### Scenario: Reviewed facts change
- **WHEN** the Agent source, base-model inventory, target state, or another mutation-relevant input changes after review
- **THEN** application rejects the stale revision and requires a fresh plan

#### Scenario: Target name belongs to another model
- **WHEN** the deterministic target name already exists without an exact app-owned deployment record
- **THEN** the plan is blocked and the existing model is not overwritten or adopted

### Requirement: Confirmed deployment preserves the exact Agent prompt
The system SHALL create or update only the reviewed app-owned target from the selected base model already present at apply time, SHALL preserve every prompt character accepted by the Agent source contract, and SHALL persist deployment truth only after Ollama confirms success.

#### Scenario: Create succeeds
- **WHEN** the user confirms a current create plan and the selected base model remains local
- **THEN** Ollama receives the exact Agent body as the target model's system prompt and the app records the exact source, base-model, target, and prompt hashes

#### Scenario: Prompt contains Modelfile delimiters
- **WHEN** the Agent body contains literal triple quotes or other Modelfile syntax
- **THEN** the deployed system prompt remains byte-for-byte equivalent to the Agent body rather than being interpreted as model configuration

#### Scenario: Base model disappears
- **WHEN** the selected base model is no longer present immediately before application
- **THEN** the operation fails closed without pulling a replacement or changing the target or ledger

### Requirement: Removal is limited to tracked deployments
The system SHALL remove only a target tied to the exact app-owned deployment record and SHALL require the same revision-bound review used for create and update.

#### Scenario: Tracked model is removed
- **WHEN** the user confirms a current remove plan for an app-owned target
- **THEN** the target is removed, its deployment record is removed, and the action receives a terminal receipt

#### Scenario: Model is not tracked
- **WHEN** a model exists in Ollama without an exact app-owned deployment record
- **THEN** the system refuses removal and leaves the model unchanged

### Requirement: Reconciliation reports runtime truth without destroying known state
The system SHALL compare each deployment record, current Agent source, local base inventory, target existence, and Ollama-reported system prompt to classify it as current, outdated, modified, missing, or source-unavailable. A failed refresh MUST retain the last complete view, disclose staleness, and block lifecycle mutation until a successful retry.

#### Scenario: Source changes after deployment
- **WHEN** the target still contains the recorded prompt but the exact Agent source now has a different prompt hash
- **THEN** reconciliation reports outdated

#### Scenario: Target prompt changes outside the app
- **WHEN** the Ollama target exists but its reported system prompt differs from the recorded prompt hash
- **THEN** reconciliation reports modified and does not overwrite it automatically

#### Scenario: Runtime refresh fails
- **WHEN** reconciliation cannot obtain a complete bounded Ollama view
- **THEN** the last complete deployment view remains visible with an explicit stale-state error and all mutations remain blocked

### Requirement: Lifecycle mutations are recoverable and auditable
The system SHALL preserve an existing managed target before update or removal, SHALL abort before mutation if preservation fails, and SHALL restore both target and ledger when a later step fails. It SHALL emit bounded success or failure receipts without Agent prompt contents.

#### Scenario: Update fails after preservation
- **WHEN** an update fails after the prior managed target was preserved
- **THEN** the prior target and deployment record are restored and the operation is reported as failed

#### Scenario: New target succeeds but state persistence fails
- **WHEN** Ollama creates a previously absent target but the deployment record cannot be committed
- **THEN** the new target is removed and no successful deployment is reported

#### Scenario: Receipt is recorded
- **WHEN** a create, update, or remove reaches a terminal outcome
- **THEN** Activity records the operation, Agent name, target name, outcome, and safe error summary without storing the system prompt, source text, credentials, or private paths

### Requirement: Authority remains local and user-controlled
The system MUST use only Ollama's fixed default loopback API, MUST NOT call generate, chat, pull, push, or remote endpoints, and MUST NOT expose local-model mutations through MCP. Every mutation SHALL originate from an explicit desktop review and confirmation.

#### Scenario: MCP client inspects available tools
- **WHEN** an MCP client enumerates Agency Agents tools
- **THEN** no Ollama create, update, or remove authority is exposed

#### Scenario: Desktop plan has not been confirmed
- **WHEN** a plan is previewed, dismissed, or left unconfirmed
- **THEN** neither Ollama nor persistent deployment state changes
