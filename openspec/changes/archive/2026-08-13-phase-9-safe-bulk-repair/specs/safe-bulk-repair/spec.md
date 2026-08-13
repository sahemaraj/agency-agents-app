## Purpose

Provides one reviewed repair workflow for recoverable Agent and Skill installations while protecting divergent or unavailable content from automatic mutation.

## ADDED Requirements

### Requirement: Repair candidates use reconciled Agent and Skill truth
The system SHALL combine tracked Agent and Skill installations in the `outdated` or `missing` state into one repair selection. It SHALL present `outdated` items as updates and `missing` items as reinstalls, and SHALL select all eligible items by default only after both installation ledgers have completed successful reconciliation.

#### Scenario: Outdated and missing installs are offered together
- **WHEN** successful reconciliation reports tracked outdated or missing Agent and Skill installations
- **THEN** the repair workflow lists every such exact installation and labels its operation as update or reinstall according to its state

#### Scenario: Repair truth is unavailable
- **WHEN** either ledger has not reconciled successfully, is reconciling, or has a current reconciliation error
- **THEN** the system SHALL not permit repair review or execution and SHALL identify the ledger that requires a retry

### Requirement: Unsafe states require manual review
The system SHALL exclude modified, foreign, disabled, and source-unavailable Agent and Skill installations from automatic selection and mutation. Each excluded installation SHALL remain visible in a manual-review section with a reason specific to its state.

#### Scenario: Divergent content is excluded
- **WHEN** reconciliation classifies an installation as modified or foreign
- **THEN** the installation is not selectable for automatic repair and the system explains that its content requires manual review

#### Scenario: Disabled or unavailable content is excluded
- **WHEN** reconciliation classifies an installation as disabled or source-unavailable
- **THEN** the installation is not selectable for automatic repair and the system explains the state-specific action required before repair

### Requirement: Complete repair plan precedes approval
The system SHALL build a read-only mutation plan for every selected installation before enabling approval. The review SHALL identify the exact Agent or Skill reference, destination, update or reinstall intent, dependencies or packages, warnings, blockers, and rollback or backup availability. Relevant Agent content differences SHALL be available from the review. Any plan failure or blocker SHALL prevent approval until the affected item is removed or the issue is resolved and the review is rebuilt.

#### Scenario: Selected set is fully planned
- **WHEN** planning succeeds for every selected installation without blockers
- **THEN** the system presents the complete combined review and enables one explicit repair approval action

#### Scenario: A plan cannot be approved
- **WHEN** any selected installation has a planning error or blocker
- **THEN** the system presents that error or blocker and keeps the approval action disabled

#### Scenario: Agent difference is inspected
- **WHEN** the user requests the difference for a planned Agent installation
- **THEN** the system shows the canonical and destination difference without mutating the installation

### Requirement: Approval applies only to the reviewed plan
The system MUST perform a fresh read-only reconciliation and preflight immediately before mutation. If eligibility, exact identity, destination, warnings, blockers, package set, or revision differs from the reviewed plan, the system SHALL perform no repair from that approval and SHALL return the user to an updated review.

#### Scenario: Reviewed truth remains current
- **WHEN** the user approves and the fresh preflight matches the complete reviewed plan
- **THEN** the system begins the approved repairs

#### Scenario: Reviewed truth changed
- **WHEN** fresh reconciliation or preflight differs from the plan the user approved
- **THEN** no selected installation is mutated and the system requires review and approval of the changed plan

### Requirement: Repairs use existing recoverable lifecycle paths
The system SHALL execute each approved installation through its existing exact-reference update lifecycle, including existing backup, transactional write, reconciliation, and Activity behavior. A missing managed installation SHALL be restored through that lifecycle while retaining its exact recorded identity and destination.

#### Scenario: Managed missing installation is restored
- **WHEN** a missing tracked Agent or Skill installation is in the approved set
- **THEN** the existing exact update lifecycle recreates its recorded destination and reconciles the resulting state

#### Scenario: Existing content is backed up
- **WHEN** an approved repair replaces content for which the existing lifecycle supports backup
- **THEN** the repair uses that existing backup and rollback behavior

### Requirement: Individual failure does not abort the remaining set
The system SHALL attempt approved repairs independently and continue with remaining installations after an individual failure. It SHALL show a terminal result for every selected installation, persist the existing exact per-item outcome in Activity, add a bounded aggregate summary, and reconcile both ledgers after the operation.

#### Scenario: One repair fails
- **WHEN** an approved item fails while later items remain
- **THEN** the failed item records its exact error and the system continues attempting the remaining approved items

#### Scenario: Repair run completes
- **WHEN** every approved item has reached success or failure
- **THEN** the workflow shows all per-item outcomes, records a success/failure summary, and displays the final reconciled installation truth

### Requirement: Existing repair entry points reflect both artifact types
The existing Agents landing action and sidebar badge SHALL count all currently eligible Agent and Skill repairs without adding another navigation destination.

#### Scenario: Skill-only repairs are available
- **WHEN** no Agent is repairable but one or more Skills are repairable
- **THEN** the existing repair badge and action remain visible with the combined eligible count
