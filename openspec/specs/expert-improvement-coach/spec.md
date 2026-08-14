# expert-improvement-coach Specification

## Purpose
Provides local, threshold-gated Expert performance summaries and deterministic improvement suggestions from existing comparable terminal runs.
## Requirements
### Requirement: Analytics use only comparable quality verdicts
The system SHALL include only accepted, rework, and rejected runs whose Expert id, Expert version, contract version, and ordered quality-check definitions match the selected Expert. Cancelled and non-terminal runs MUST NOT contribute.

#### Scenario: Historical contract differs
- **WHEN** a terminal run uses another Expert version or quality contract
- **THEN** it is excluded from the selected Expert's performance cohort

#### Scenario: Run has no quality verdict
- **WHEN** a run is in progress, awaiting review, or cancelled
- **THEN** it is excluded from performance metrics and the minimum sample count

### Requirement: Coaching requires five comparable runs
The system SHALL show aggregate metrics and improvement suggestions only after at least five comparable terminal runs exist. Before then, it SHALL show the current comparable-run count and the five-run requirement without estimating a trend.

#### Scenario: Cohort is below threshold
- **WHEN** fewer than five comparable terminal runs exist
- **THEN** no acceptance rate, issue rate, or improvement suggestion is shown

#### Scenario: Cohort reaches threshold
- **WHEN** at least five comparable terminal runs exist
- **THEN** the existing Expert detail surface shows bounded aggregate metrics and evidence-derived signals

### Requirement: Metrics and suggestions are deterministic and local
The system SHALL derive acceptance, rework/rejection, waiver, and per-check latest-evidence issue rates from already-loaded local run data. Suggestions MUST identify recurring observed signals without claiming causation. The feature MUST NOT invoke a model, contact a network service, emit telemetry, mutate an Expert, or add new persistence.

#### Scenario: A check repeatedly fails or lacks evidence
- **WHEN** at least two comparable runs and at least 40% of the cohort have a latest fail, skipped, or missing result for the same check
- **THEN** the system suggests reviewing that named check's instructions or tooling and reports the observed count

#### Scenario: Required checks are repeatedly waived
- **WHEN** at least two comparable runs and at least 40% of the cohort waive the same required check
- **THEN** the system suggests clarifying that named check or improving its evidence path and reports the observed count

#### Scenario: No recurring issue meets the threshold
- **WHEN** five or more comparable runs exist and no verdict or check signal meets the recurring threshold
- **THEN** the system states that no recurring improvement signal was detected

