## Purpose

Provide safe local management of bounded app-owned instruction snippets inside registered projects while preserving user-authored content and requiring exact review before every mutation.

## Requirements

### Requirement: Inspection is bounded to known files in registered projects
The system SHALL inspect only `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and `.github/copilot-instructions.md` beneath an exact canonical registered project. Inspection SHALL be local and read-only, reject links and non-regular files, enforce byte and UTF-8 bounds, and classify each target as absent, existing-unmanaged, managed, or blocked.

#### Scenario: Existing files are inspected without mutation
- **WHEN** a user opens instruction management for a registered project
- **THEN** the system reports each known target, existing app-owned snippets, adoption state, and blockers without changing project or app state

#### Scenario: Unsafe target is blocked
- **WHEN** a supported target or one of its existing path components is a link, special file, oversized file, or invalid UTF-8
- **THEN** the system marks that target blocked and does not read outside the project or offer approval

### Requirement: Managed snippets preserve unowned content
The system SHALL create, replace, or remove only one bounded app-owned named snippet at a time. It SHALL preserve every unowned byte, reject malformed or nested ownership markers, reject duplicate snippet identifiers, and treat the first managed insertion into an existing file as explicit adoption rather than ownership of the file.

#### Scenario: Adopt an existing instruction file
- **WHEN** a user plans a new valid snippet for an existing unmanaged target
- **THEN** the proposed content contains the original bytes unchanged plus one visibly delimited app-owned block and labels the operation as adoption

#### Scenario: Remove a managed snippet
- **WHEN** a user plans removal of an existing app-owned snippet
- **THEN** the proposal removes only that owned block and restores the surrounding unowned bytes exactly

### Requirement: Every mutation has a complete deterministic plan
The system SHALL return the exact target, operation, current content, proposed content, warnings, blockers, backup expectation, and a deterministic revision derived from all apply-relevant inputs and current bytes. Planning SHALL perform no write, network request, instruction execution, or external configuration change.

#### Scenario: User reviews an exact diff
- **WHEN** a valid create, replace, or remove request is planned
- **THEN** the UI presents a line diff of the complete current and proposed file plus adoption, backup, warning, and blocker information before approval

#### Scenario: No-op is not approvable
- **WHEN** the proposed bytes equal the current bytes
- **THEN** the plan identifies a no-op and approval remains unavailable

### Requirement: Apply is explicit and revision-bound
The system SHALL require explicit confirmation and the exact plan revision. Immediately before writing it SHALL repeat registration, path, link, file, marker, bound, and byte checks; any drift or blocker SHALL cause zero project-file writes and return a refreshed plan.

#### Scenario: Stale approval causes zero writes
- **WHEN** the target bytes or project registration change after review
- **THEN** apply returns a refreshed blocked or changed plan and leaves the target and backups untouched

#### Scenario: Fresh approval applies one target
- **WHEN** the revision is current, the plan has a real change and no blockers, and the user confirms
- **THEN** the system changes only the reviewed target and returns its exact destination, outcome, and backup path when applicable

### Requirement: Existing content is backed up and recovery is honest
Before replacing or deleting any existing target bytes, the system SHALL write and verify a private exact backup. The project mutation SHALL use atomic publication and the existing durable filesystem-operation journal, with idempotent startup recovery that restores the pre-operation state or retains an explicit recovery error.

#### Scenario: Existing file receives a verified backup
- **WHEN** an approved plan changes an existing target
- **THEN** the system preserves its exact previous bytes in app-owned backup storage before atomic replacement and reports the backup path

#### Scenario: Interrupted apply is recovered
- **WHEN** startup encounters an incomplete instruction operation
- **THEN** recovery safely restores the previous file or removes a newly created file, commits the recovered journal once, and never modifies unrelated content

### Requirement: Project UI owns the review workflow
The existing Projects detail surface SHALL provide target inspection, snippet composition, removal, complete diff review, explicit approval, progress, retained terminal result, accessible announcements, and focus restoration without adding a route or executing instruction content.

#### Scenario: Blocked plan stays inspectable
- **WHEN** inspection or planning reports a blocker
- **THEN** the UI keeps the evidence visible, disables apply, announces the blocker, and returns focus to the initiating control when the flow closes

#### Scenario: Terminal result is recorded locally
- **WHEN** apply succeeds or fails after an attempt
- **THEN** the UI retains the exact destination and redacted bounded detail and adds one existing-format local Activity entry without retrying the filesystem mutation if Activity persistence fails

### Requirement: Instruction text remains passive and secret-free
The system MUST treat all instruction text as inert data and MUST reject ownership-marker injection, control characters, traversal, and obvious credential material. It SHALL NOT run tools, install dependencies, configure MCP, emit telemetry, or notify externally as part of instruction management.

#### Scenario: Unsafe snippet is rejected before planning
- **WHEN** a snippet contains an ownership marker, control character, traversal identity, or obvious credential material
- **THEN** the system returns a validation blocker and performs no write or execution
