## Purpose

Lets users opt into quiet native alerts when bounded local reconciliation discovers newly actionable Agent or Skill drift while Agency Agents is backgrounded.

## ADDED Requirements

### Requirement: Native drift notifications are explicit opt-in
The system SHALL keep native drift notifications disabled by default. It MUST request operating-system permission only from an explicit user action, persist the preference only when permission is granted, and expose denial or failure without repeatedly prompting.

#### Scenario: User enables notifications
- **WHEN** the user enables drift notifications and the operating system grants permission
- **THEN** the preference is persisted and background drift checks become eligible

#### Scenario: Permission is denied
- **WHEN** the user enables drift notifications and permission is denied or unavailable
- **THEN** the preference remains disabled, the failure is visible, and no notification is sent

### Requirement: Background checking reuses bounded local reconciliation
While opted in, the system SHALL periodically reuse the existing bounded local Agent and Skill reconciliation only while the app is backgrounded. It MUST NOT perform a network request, catalog refresh, filesystem mutation, source update, telemetry emission, or Agent or Skill execution.

#### Scenario: App remains in the foreground
- **WHEN** the periodic interval elapses while the app is visible
- **THEN** no background drift scan or native notification occurs

#### Scenario: One reconciliation authority fails
- **WHEN** either the Agent or Skill scan fails during a background check
- **THEN** no drift notification is sent and the last complete baseline remains unchanged

### Requirement: Notifications report only newly actionable tracked drift
The system SHALL establish a complete initial baseline without notifying. After later successful background scans, it SHALL emit at most one bounded notification for tracked installations newly entering outdated, modified, or missing state. It MUST deduplicate by exact logical installation identity, omit filesystem paths and source content, and update the baseline after each complete scan so unchanged drift is not repeated.

#### Scenario: Initial drift already exists
- **WHEN** the first complete baseline contains actionable drift
- **THEN** no native notification is sent for that existing drift

#### Scenario: New drift appears
- **WHEN** a later complete background scan finds one or more tracked installations newly actionable relative to the previous complete baseline
- **THEN** one notification reports bounded Agent and Skill counts without private paths or content

#### Scenario: Drift remains unchanged
- **WHEN** a later complete scan contains the same actionable identities as the previous complete baseline
- **THEN** no duplicate notification is sent

#### Scenario: Resolved drift returns
- **WHEN** an actionable identity disappears from one complete baseline and reappears in a later complete background scan
- **THEN** it is treated as newly actionable and may trigger one notification

### Requirement: Notification activation opens an existing review surface
Activating a drift notification SHALL bring Agency Agents to an existing relevant review surface without starting repair or any other mutation. Agent drift SHALL open the Agent attention lens; Skill-only drift SHALL open the Skills workspace.

#### Scenario: User activates an Agent drift notification
- **WHEN** the most recent notification includes Agent drift and the user activates it
- **THEN** the app opens the existing Agent attention lens and performs no repair automatically

#### Scenario: User activates a Skill-only drift notification
- **WHEN** the most recent notification includes only Skill drift and the user activates it
- **THEN** the app opens the existing Skills workspace and performs no repair automatically

