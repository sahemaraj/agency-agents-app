# Doctor Health Check Specification

## Purpose

Provides one truthful, privacy-safe view of Agency Agents health so users can diagnose local configuration and state problems and reach the existing safe recovery controls.

## Requirements

### Requirement: Doctor produces one evidence-based local report
The system SHALL produce one on-demand report covering storage, settings, catalog, Agent sources, Skill sources, installation reconciliation, deployment-tool detection, MCP client registration, and cached update configuration or state. Each check MUST be classified as Healthy, Needs attention, or Unavailable from current local evidence.

#### Scenario: All authorities are readable
- **WHEN** the user runs Doctor and each covered local authority returns usable evidence
- **THEN** Doctor presents every check in deterministic category and check order with its classification, concise evidence, and timestamp

#### Scenario: One authority fails
- **WHEN** one covered authority cannot be read or evaluated
- **THEN** Doctor marks only the affected check or checks Unavailable, retains successful checks, and does not classify the overall report as fully healthy

#### Scenario: Evidence shows a recoverable problem
- **WHEN** a covered authority reports corrupt, stale, conflicting, missing, rejected, or otherwise actionable state
- **THEN** Doctor classifies the applicable check as Needs attention and explains the observed evidence without claiming a repair occurred

### Requirement: Doctor remains read-only and offline
Running or refreshing Doctor SHALL use bounded local inspection only. It SHALL NOT mutate application or project state, initiate installation or execution, refresh a network source, check a remote update service, write telemetry, or request credentials or Keychain access that could prompt the user.

#### Scenario: User refreshes Doctor
- **WHEN** the user requests a fresh report
- **THEN** the system re-runs the bounded local checks without invoking mutation or network commands

#### Scenario: Remote state is not locally known
- **WHEN** a health conclusion would require a network request or credential prompt
- **THEN** Doctor reports the check as Unavailable or reports only the cached configuration or state and identifies that limitation

### Requirement: Every non-healthy result has a safe next action
Each Needs attention result SHALL provide a safe manual next action. Each Unavailable result SHALL provide either a retry action or a navigation target when one exists. Actions MUST hand off to existing controls and SHALL NOT perform the repair directly.

#### Scenario: Existing recovery surface applies
- **WHEN** a result maps to an existing Settings, Catalog, Tools, MCP, update, source, or reconciliation control
- **THEN** activating its action opens that exact existing surface with the relevant context

#### Scenario: No app action can resolve the condition
- **WHEN** a result requires an external or manual environment change
- **THEN** Doctor displays bounded manual guidance without offering a misleading automatic action

### Requirement: Doctor report export is deterministic and privacy-safe
The system SHALL let the user copy a deterministic text report containing classifications, bounded evidence, and safe guidance. The visible and copied report MUST exclude credentials, tokens, secrets, command credentials, user-identifying home-directory prefixes, and unbounded raw error or path content.

#### Scenario: Evidence contains sensitive values
- **WHEN** source evidence or an error contains a credential, bearer value, secret pattern, authenticated URL, or private absolute path
- **THEN** the rendered and copied report replaces it with a stable redacted or home-relative representation

#### Scenario: User copies the report
- **WHEN** the user activates Copy Report after a report completes
- **THEN** the clipboard receives the same deterministic redacted report represented by the visible checks and the interface announces success or failure

### Requirement: Doctor is accessible and honest during its lifecycle
The Doctor surface SHALL expose loading, completion, failure, counts, classifications, and copy outcomes to assistive technology. A prior report MAY remain visible during refresh but MUST be identified as prior evidence until the refresh completes.

#### Scenario: Report is running
- **WHEN** Doctor inspection is in progress
- **THEN** the interface announces that checks are running, identifies stale prior results if displayed, and prevents overlapping refreshes

#### Scenario: Report completes
- **WHEN** Doctor finishes with any combination of classifications
- **THEN** the interface announces counts for Healthy, Needs attention, and Unavailable and supports keyboard access to every available action

#### Scenario: Report command fails globally
- **WHEN** the report cannot be constructed at all
- **THEN** the interface preserves any prior report as stale evidence, presents a bounded retryable error, and does not show an all-healthy state
