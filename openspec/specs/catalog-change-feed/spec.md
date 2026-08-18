# Catalog Change Feed Specification

## Purpose

Provides durable, bounded, and deterministic evidence of changes to the active local Agent catalog without turning refresh into installation or approval authority.

## Requirements

### Requirement: Successful refresh commits one durable catalog transition
The system SHALL retain the prior active-catalog snapshot until a successful explicit refresh has durably committed the replacement snapshot and its bounded typed change batch. Cursor or recommendation state MUST NOT advance before that commit.

#### Scenario: Refresh succeeds
- **WHEN** an explicit catalog refresh validates and activates a new catalog snapshot
- **THEN** the system durably records the corresponding change batch and replacement snapshot before exposing the new refresh result

#### Scenario: Refresh fails
- **WHEN** download, validation, activation, or persistence fails
- **THEN** the last successful snapshot and timestamp remain visible as stale, no change batch is appended, and Retry is available

### Requirement: Catalog changes are deterministic and source-relative
The system SHALL classify normalized source-relative entries as added, updated, removed, or renamed. It SHALL infer rename only for one unambiguous removed/added pair with matching identity hashes; ambiguous matches remain separate remove and add events.

#### Scenario: Unambiguous rename
- **WHEN** exactly one removed entry and one added entry share the required matching hashes
- **THEN** the batch exposes one rename with the old and new normalized relative paths

#### Scenario: Ambiguous matching entries
- **WHEN** multiple added or removed entries could satisfy the same rename match
- **THEN** the batch exposes independent add and remove events and does not guess a rename

### Requirement: Feed reads are bounded and non-mutating
The system SHALL bound snapshots, batches, items, text, and relative paths before persistence and IPC. Listing or viewing the feed MUST NOT install, update, approve, execute, or otherwise mutate an Agent.

#### Scenario: Oversized control-center state
- **WHEN** a candidate snapshot or feed exceeds any configured item, byte, text, or path bound
- **THEN** the refresh fails closed before committing the candidate state
