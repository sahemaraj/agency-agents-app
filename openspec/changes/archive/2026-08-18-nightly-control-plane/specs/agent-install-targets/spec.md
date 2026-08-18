## Purpose

Defines exact, recoverable lifecycle behavior for Kimi, OpenClaw, Aider, Windsurf, and Antigravity while preserving file-only installation and existing review boundaries.

## ADDED Requirements

### Requirement: Multi-artifact targets have one exact lifecycle
For Kimi and OpenClaw, the system SHALL derive every artifact path from fixed registry templates, preflight every target, journal exact prior bytes, write and verify the complete artifact set, and commit ledger truth last. Reconcile, history, rollback, disable, enable, update, uninstall, and recovery SHALL cover every artifact.

#### Scenario: Secondary artifact is missing or modified
- **WHEN** any tracked Kimi or OpenClaw secondary artifact is deleted or its bytes change
- **THEN** reconciliation reports the aggregate installation as Missing or Modified according to the documented precedence

#### Scenario: One artifact write fails
- **WHEN** any write or verification step fails during a multi-artifact mutation
- **THEN** every prior artifact and ledger row is restored and success is not reported

### Requirement: OpenClaw installation never executes OpenClaw
The system SHALL install and verify OpenClaw workspace files only. It MUST NOT execute an OpenClaw command for installation, activation, restart, registration, detection, or version probing, and SHALL report registration/restart as an external required step.

#### Scenario: User opens or installs OpenClaw
- **WHEN** OpenClaw is listed, planned, installed, reconciled, or viewed
- **THEN** no `openclaw` executable is launched and successful file installation remains distinct from external activation

### Requirement: Aider and Windsurf are exact project rosters
The system SHALL represent Aider and Windsurf as project-scoped aggregate roster records containing deterministic ordered exact Agent references and the aggregate artifact manifest. It MUST NOT encode a roster as one per-Agent install and MUST refuse foreign aggregate files.

#### Scenario: User reviews a roster
- **WHEN** at least two exact installable Agents and one registered project are selected
- **THEN** the plan shows the aggregate destination, complete ordered membership, blockers, rollback scope, and a revision before apply

#### Scenario: Generic per-Agent workflow receives a roster target
- **WHEN** a Workspace Pack, recommendation, or generic per-Agent planner cannot express a valid aggregate roster selection
- **THEN** the target is routed to the roster planner or explicitly blocked before an incompatible apply path is reachable

### Requirement: Project inventory and removal preserve roster lifecycle truth
Tracked project rosters SHALL contribute to project inventory and removal decisions. Remove-only SHALL atomically discard associated Agent and roster tracking while leaving project bytes untouched; Remove-and-uninstall SHALL remove verified roster artifacts and ledger truth before unregistering. A project MUST NOT be unregistered while a tracked roster would remain unreachable.

#### Scenario: User chooses remove only with a tracked roster
- **WHEN** the project still owns an Aider or Windsurf roster record
- **THEN** the app leaves aggregate bytes untouched, atomically removes the project roster tracking with the other project tracking, and unregisters only after no tracked roster remains

#### Scenario: User chooses remove and uninstall
- **WHEN** all tracked Agent, Skill, and roster artifacts remain safely removable
- **THEN** the app removes them through their owning lifecycle and unregisters the project only after verified completion

### Requirement: Roster mutations are durable and recoverable
Roster lifecycle operations SHALL enforce registered-project authority, normalized no-link/reparse paths, exact source identity, record/member/artifact bounds, revision binding, prepared journal recovery, byte verification, and rollback on failure.

#### Scenario: Roster recovery finds an unregistered or retargeted project
- **WHEN** startup recovery cannot prove the original registered project and destination authority
- **THEN** it refuses mutation, preserves recoverable evidence, and does not claim completion

### Requirement: Antigravity remains behaviorally unchanged and proven
Antigravity SHALL continue using its existing exact renderer, destinations, plan/apply, reconciliation, and uninstall paths. Target expansion MUST NOT introduce a second Antigravity renderer or lifecycle.

#### Scenario: Antigravity round trip
- **WHEN** an exact Agent is planned, installed, reconciled, and uninstalled for Antigravity
- **THEN** production output matches the existing upstream parity contract and no residual tracked artifact remains
