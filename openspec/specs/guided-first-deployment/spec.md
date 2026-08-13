## Purpose

Provide a fast, explicit, and truthful first-run path from catalog selection to a verified Claude Code or Codex team deployment without bypassing installation safety controls.

## Requirements

### Requirement: Catalog selection continues into guided deployment
After a new user successfully selects a catalog, the system SHALL continue directly into a first-deployment flow instead of dismissing into an unguided empty workspace.

#### Scenario: Catalog selection succeeds
- **WHEN** a previously unconfigured user successfully selects any supported catalog source
- **THEN** the guided deployment flow remains in the foreground and advances to deployment preparation

#### Scenario: User defers deployment
- **WHEN** the user selects the secondary finish-later action
- **THEN** the system SHALL close the guide without writing any Agent destination

### Requirement: Supported target detection is truthful
The system SHALL show the actual detected state of Claude Code and Codex and SHALL enable first deployment only for a detected compatible target.

#### Scenario: One supported target is detected
- **WHEN** exactly one of Claude Code or Codex is detected
- **THEN** the system SHALL recommend that target and make the other target visibly unavailable

#### Scenario: Both supported targets are detected
- **WHEN** both Claude Code and Codex are detected
- **THEN** the system SHALL select Claude Code by default while allowing the user to select Codex

#### Scenario: No supported target is detected
- **WHEN** neither Claude Code nor Codex is detected
- **THEN** the system SHALL present an actionable blocked state and SHALL NOT offer deployment approval

### Requirement: Preset recommendation is deterministic and compatible
The system SHALL display exactly one deterministic preset-team recommendation whose members can be resolved from the active catalog.

#### Scenario: Preferred preset is complete
- **WHEN** every Agent in the AI Builders preset can be resolved from the active catalog
- **THEN** the system SHALL recommend AI Builders

#### Scenario: Preferred preset is incomplete
- **WHEN** one or more AI Builders Agents cannot be resolved
- **THEN** the system SHALL recommend the first complete bundled preset in declaration order

#### Scenario: No preset is complete
- **WHEN** no bundled preset can be fully resolved from the active catalog
- **THEN** the system SHALL block approval and explain that the selected catalog has no compatible preset

### Requirement: Scope and mutation plan are reviewed before writing
The system SHALL allow user or registered-project scope and SHALL expose every planned Agent, dependency, destination, warning, and blocker before any deployment write.

#### Scenario: User scope selected
- **WHEN** the user selects user scope and a detected target
- **THEN** the plan SHALL identify the exact user-scoped destination for every Agent and dependency

#### Scenario: Project scope selected
- **WHEN** the user selects project scope
- **THEN** the system SHALL require a registered canonical project and identify every project-scoped destination before approval

#### Scenario: Plan is blocked
- **WHEN** plan generation reports one or more blockers
- **THEN** the system SHALL display every blocker and SHALL disable deployment approval

#### Scenario: User has not approved
- **WHEN** a valid mutation plan is visible but the user has not selected Apply
- **THEN** no destination content or installation ledger entry SHALL be changed

### Requirement: Approved deployment is transactional
The system SHALL apply the reviewed exact-reference preset plan through the existing transactional Agent batch path after one explicit approval.

#### Scenario: Batch deployment succeeds
- **WHEN** the user explicitly approves a valid plan
- **THEN** the system SHALL install every planned Agent and dependency or report failure without partial success

#### Scenario: Batch deployment fails
- **WHEN** any planned Agent write fails
- **THEN** the system SHALL restore captured destination content and the prior installation ledger before reporting failure

### Requirement: Completion is reconciliation-backed
The system SHALL report first-deployment success only after reconciliation confirms every planned Agent at the selected target and scope.

#### Scenario: Reconciliation confirms deployment
- **WHEN** the approved transaction completes and reconciliation classifies every planned exact reference as present at the selected target and scope
- **THEN** the system SHALL show success, all installed destinations, and a starter prompt appropriate to the recommended preset

#### Scenario: Reconciliation fails or cannot confirm deployment
- **WHEN** reconciliation errors or does not confirm every planned exact reference
- **THEN** the system SHALL retain an incomplete or stale state, show the actionable error, and SHALL NOT claim verified success

### Requirement: Normal first deployment is designed for rapid completion
The normal local happy path SHALL require only catalog selection, target and scope confirmation, plan review, and one deployment approval, with no required network request after catalog selection.

#### Scenario: Default happy path
- **WHEN** Claude Code or Codex is already detected, the preferred preset is complete, user scope is accepted, and local writes and reconciliation succeed
- **THEN** the interaction SHALL be designed to complete in under 60 seconds without navigating away from the guide
