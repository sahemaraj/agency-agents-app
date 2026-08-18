## Purpose

Provides explicit project-level catalog subscriptions and bounded local recommendations while preserving existing review, approval, and mutation boundaries.

## ADDED Requirements

### Requirement: Catalog subscriptions are explicit and bounded
The system SHALL create a subscription only after an explicit user opt-in for a registered project with a readiness baseline. It SHALL persist at most one subscription per project and reject state beyond the control-center bounds.

#### Scenario: User opts in
- **WHEN** the user enables catalog recommendations for an eligible project
- **THEN** one durable subscription is stored without generating a mutation or advancing its cursor prematurely

### Requirement: Recommendation cursors advance only after durable evaluation
The system SHALL derive recommendations only from a successfully committed catalog feed and exact project baseline. It SHALL durably persist the surfaced recommendation set before advancing the subscription cursor.

#### Scenario: Evaluation is interrupted
- **WHEN** recommendation derivation or persistence fails after a feed refresh
- **THEN** the prior cursor remains unchanged and the system can retry without silently losing changes

### Requirement: Recommendations reuse existing reviewed plans
Opening a recommendation SHALL re-resolve current exact references and enter the owning existing plan/review UI. A recommendation MUST NOT approve, install, update, execute, or bypass roster selection constraints.

#### Scenario: Recommended reference is stale
- **WHEN** the referenced source revision no longer resolves exactly at open time
- **THEN** the recommendation is Unavailable and no mutation plan is approved or applied

#### Scenario: Recommendation targets an aggregate roster tool
- **WHEN** a recommendation cannot provide a valid multi-Agent project roster selection
- **THEN** the target is excluded or explicitly blocked before opening an empty or incompatible installer

### Requirement: Recommendation lifecycle is durable
The system SHALL support bounded dismissal and deterministic supersession. A newer recommendation for the same logical requirement SHALL supersede the older item without reviving dismissed stale work.

#### Scenario: User dismisses a recommendation
- **WHEN** the user dismisses a surfaced recommendation
- **THEN** its durable identity is remembered within bounds and the same stale item is not immediately resurfaced

