## Purpose

Lets users move one reviewed local workspace configuration between projects or machines without embedding private paths or bypassing existing Agent and Skill safety boundaries.

## ADDED Requirements

### Requirement: Workspace Packs are bounded deterministic portable manifests
The system SHALL export a versioned UTF-8 JSON Workspace Pack containing a user-visible name, exact source-aware Agent and Skill references, tool targets, one logical user or project scope, optional runbook context, and optional declarative instruction and MCP requirements. Export MUST be deterministic for the same selected managed state, MUST NOT include absolute project paths, credentials, file contents, audit history, or machine-specific secrets, and MUST reject data outside the supported bounds.

#### Scenario: User exports a global pack
- **WHEN** the user selects the global managed deployment and exports a Workspace Pack
- **THEN** the file contains the exact managed Agent and Skill references and tool targets in deterministic order with user scope and no project path

#### Scenario: User exports a project pack
- **WHEN** the user selects one registered project and exports its managed deployment
- **THEN** the file contains only that project's exact managed Agent and Skill references with logical project scope and does not reveal the source project's absolute path

### Requirement: Legacy Agentfiles remain inspectable through the safe workflow
The system SHALL accept an existing Agentfile version 1 as an Agent-only legacy input, resolve each slug only when it maps unambiguously to one exact installable Agent, and convert it into the same read-only review workflow. It MUST reject unsupported versions, malformed entries, ambiguity, and exceeded bounds instead of silently skipping them or mutating files.

#### Scenario: User opens a valid legacy Agentfile
- **WHEN** every legacy slug resolves uniquely and all entries are within bounds
- **THEN** the system presents an Agent-only Workspace Pack review without changing any destination

#### Scenario: Legacy input contains an ambiguous or invalid entry
- **WHEN** a slug cannot resolve to exactly one installable Agent or an entry is malformed
- **THEN** import reports a blocker for that entry and performs no mutation

### Requirement: Import produces a complete read-only plan before approval
Opening a Workspace Pack SHALL perform bounded local validation and produce one complete plan for every declared Agent and Skill deployment. The plan MUST show exact artifact identity, tool/runtime, chosen scope binding, exact destination, current state, dependency entries, warnings, blockers, rollback scope, runbook context, instruction requirements, and MCP requirements. Project-scoped packs MUST require an explicit existing project binding before their destinations can be planned. Planning MUST perform no filesystem mutation, network request, instruction edit, MCP configuration, or runbook execution.

#### Scenario: Pack is ready to apply
- **WHEN** every exact reference and target resolves, the project binding is valid when required, and no destination is unsafe
- **THEN** the complete plan identifies every destination and enables one explicit approval action

#### Scenario: Pack has an unresolved requirement or unsafe destination
- **WHEN** a required Agent or Skill is missing, a target is unsupported, a destination is foreign, modified, or otherwise unsafe, or a project binding is absent
- **THEN** the complete plan displays the blocker and approval remains disabled

#### Scenario: Pack declares instructions or MCP requirements
- **WHEN** declarative instruction or MCP requirements are present
- **THEN** the review displays them as not automatically applied or configured and does not claim that they are satisfied

### Requirement: Approval applies only the unchanged reviewed plan
The system MUST perform fresh local source, tool, project, and installation validation immediately before applying a Workspace Pack. Approval MUST be bound to a deterministic plan revision. If exact identities, destinations, dependencies, warnings, blockers, project binding, or existing state differ from the reviewed plan, the system SHALL perform zero pack mutation and require a new review.

#### Scenario: Reviewed plan remains current
- **WHEN** fresh preflight matches the approved revision and contains no blockers
- **THEN** the system begins applying only the planned missing Agent and Skill deployments

#### Scenario: Reviewed plan changed
- **WHEN** fresh preflight produces a different revision or blocker
- **THEN** no pack item is mutated and the user receives the refreshed plan for review

### Requirement: Pack application reuses recoverable exact lifecycle operations
The system SHALL apply Agents and Skills through their existing exact-reference install, backup, ledger, and reconciliation authorities. Existing current deployments MUST remain unchanged. If an item fails, the system MUST stop, remove only Agent and Skill artifacts newly created by that pack run in reverse order, preserve all pre-existing content, and report any rollback failure honestly. The operation MUST be durably recoverable through the existing filesystem-operation journal.

#### Scenario: All planned items install successfully
- **WHEN** every missing planned Agent and Skill installs through its existing lifecycle
- **THEN** the operation completes, both installation ledgers reconcile, and pre-existing current content remains byte-identical

#### Scenario: A later item fails
- **WHEN** an Agent or Skill fails after earlier pack items were newly installed
- **THEN** only those newly installed items are removed in reverse order, pre-existing content is preserved, and the operation reports failure rather than partial success

#### Scenario: App stops during pack application
- **WHEN** the process restarts with a prepared or applied pack filesystem operation
- **THEN** existing recovery completes or rolls back the bound operation idempotently without applying unreviewed pack content

### Requirement: Pack completion is accessible and auditable
The existing Teams file surface SHALL retain the complete plan while applying, announce progress, present every terminal item and exact destination, and preserve keyboard focus. A completed pack mutation SHALL create one bounded local post-action receipt and offer `View Activity` for that exact receipt. Closing or persistence failure MUST NOT cause a mutation retry.

#### Scenario: Pack apply completes
- **WHEN** every planned mutation reaches a terminal outcome
- **THEN** retained results and one exact Activity receipt identify every attempted Agent and Skill destination and the aggregate result

#### Scenario: User follows pack completion evidence
- **WHEN** the user activates `View Activity` from the retained result
- **THEN** the matching receipt opens, reveals, and receives focus through the existing Activity navigation behavior

### Requirement: Workspace Packs do not become a runtime or remote marketplace
Workspace Pack import, export, planning, and application SHALL remain local. The system SHALL NOT execute Agents or runbooks, install or enable MCP servers, edit project instruction files, fetch missing sources, upload packs, create sharing links, emit telemetry, or send native notifications as part of this capability.

#### Scenario: Pack references unavailable external configuration
- **WHEN** a pack declares a missing source, instruction requirement, MCP requirement, or runbook
- **THEN** the system reports the unresolved declaration locally and does not fetch, execute, install, or configure it
