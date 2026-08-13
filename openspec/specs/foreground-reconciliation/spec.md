# Foreground Reconciliation Specification

## Purpose

Keep Agent and Skill installation truth current when users return to the application without duplicating scans, changing managed content, or discarding usable state after a failure.

## Requirements

### Requirement: Foreground return reconciles all installation truth
The system SHALL request reconciliation of both Agent and Skill installation state after the application window regains focus.

#### Scenario: Application regains focus
- **WHEN** the application window regains focus after being in the background
- **THEN** the system SHALL request current Agent and Skill installation state

#### Scenario: Application remains active
- **WHEN** the application remains focused without a new focus event
- **THEN** the system SHALL NOT start periodic foreground reconciliation

### Requirement: Foreground requests are debounced and coalesced
The system SHALL debounce foreground signals and SHALL share existing in-flight store reconciliation so rapid or overlapping focus and mount requests do not cause duplicate visible scans.

#### Scenario: Rapid focus signals occur
- **WHEN** multiple focus events occur within the debounce interval
- **THEN** the system SHALL issue one reconciliation request for each installation ledger after the final signal

#### Scenario: Focus overlaps a mount scan
- **WHEN** a foreground reconciliation reaches a store while the same installation scope is already reconciling for application or view mount
- **THEN** both callers SHALL share the existing in-flight operation without a second visible loading cycle

### Requirement: Foreground reconciliation is local and read-only
Foreground reconciliation SHALL inspect only local installation state and SHALL NOT refresh Agent catalogs, refresh Skill sources, perform network access, or mutate catalog, source, ledger, or managed destination content.

#### Scenario: Foreground reconciliation succeeds
- **WHEN** a foreground scan completes successfully
- **THEN** only the in-memory reconciled installation and backup views SHALL be refreshed from local read commands

#### Scenario: Catalog or source refresh would be available
- **WHEN** the application regains focus while catalog or Skill source refresh actions are available
- **THEN** the system SHALL NOT invoke those refresh actions as part of reconciliation

### Requirement: Failed foreground reconciliation preserves usable state
If either installation scan fails, the system SHALL retain that ledger's last-known rows, expose its actionable stale or error state, and keep its existing Retry control available.

#### Scenario: Scan fails after known state exists
- **WHEN** a foreground reconciliation fails after a prior successful reconciliation
- **THEN** the system SHALL keep the prior installation rows visible and mark their truth as out of date

#### Scenario: Scan fails before known state exists
- **WHEN** the first reconciliation fails before any installation state is known
- **THEN** the system SHALL present installation status as unavailable rather than as a confirmed empty library

#### Scenario: Retry succeeds
- **WHEN** the user retries a failed reconciliation and the local scan succeeds
- **THEN** the system SHALL replace the affected ledger with current results and clear its stale or error state
