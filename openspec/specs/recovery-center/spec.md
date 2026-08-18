# Recovery Center Specification

## Purpose

Provides one honest recovery inventory over existing exact history, rollback, backup, and storage controls while preserving each operation's established safety boundary.

## Requirements

### Requirement: Recovery aggregates existing recoverable evidence
The system SHALL list Agent history/rollback, Skill backup/rollback, and verified storage backup/reveal evidence without inventing recoverability when history or a backup is unavailable.

#### Scenario: Some recovery sources fail
- **WHEN** Agent history loads but Skill history inspection fails
- **THEN** Agent recovery remains usable, Skill recovery shows an independent error and Retry, and the aggregate state is partial

### Requirement: Recovery actions retain exact safety contracts
Every exposed rollback or backup action SHALL execute through its existing exact source, revision, destination, ownership, path-containment, journal, and verification boundary.

#### Scenario: Recovery target drifted after review
- **WHEN** destination bytes or source identity no longer match the reviewed recovery action
- **THEN** the action fails closed and asks for refreshed evidence without overwriting current content

### Requirement: Database restore is offline and manual
The running application SHALL NOT replace its live SQLite database. Recovery guidance SHALL allow creation and reveal of a verified backup and SHALL label restore as an offline/manual operation.

#### Scenario: User requests database recovery
- **WHEN** the user opens database recovery guidance
- **THEN** the app offers verified backup creation/reveal and explicit shutdown/manual-restore guidance, with no hot-restore control

### Requirement: Recovery completion is announced and focusable
Async completion and errors SHALL be announced. After creating a backup, focus SHALL move to the exact Reveal action or another deterministic result control.

#### Scenario: Backup completes
- **WHEN** verified backup creation succeeds
- **THEN** the result is announced and keyboard focus moves to the visible Reveal control
