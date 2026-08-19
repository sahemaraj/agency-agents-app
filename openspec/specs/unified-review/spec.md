# Unified Review Specification

## Purpose

Provides one accessible Activity projection of every existing pending approval shape without creating another decision engine or weakening domain ownership.

## Requirements

### Requirement: Review aggregates every existing approval source
The system SHALL show Agent, Skill, Expert change, Expert run, and Expert activation approval shapes in one Review mode while keeping catalog recommendations separately labelled and counted.

#### Scenario: All sources are ready
- **WHEN** every approval source loads successfully
- **THEN** Review shows the combined pending items with their source type and owning action

#### Scenario: One source fails
- **WHEN** one approval source is unavailable
- **THEN** its error and Retry remain visible, other sources remain usable, and the combined count is marked partial rather than treating the failed source as zero

### Requirement: Owning domains retain action authority
The unified view SHALL delegate inspection, approval, rejection, retry, and deep links to the existing domain workflow. It MUST NOT persist a second approval record or invoke a mutation directly.

#### Scenario: User opens an Agent approval
- **WHEN** the user activates the Agent review item
- **THEN** the exact existing Agent approval surface opens and its original revision-bound authority remains in force

### Requirement: Review interaction is keyboard and focus safe
Mode controls SHALL be visible-focus pressed buttons with polite announcements. Closing a delegated review SHALL restore focus to the exact initiating review control.

#### Scenario: Keyboard user returns from review
- **WHEN** a keyboard user closes an opened approval detail
- **THEN** focus returns to the same Review item trigger and the resulting state is announced

### Requirement: Review includes both Factory human gates
Activity Review SHALL include Factory Runs awaiting plan approval and Factory Runs awaiting final approval as Expert-run-owned approval items. Each item MUST identify the gate, work-order title, project label, current revision, and owning action. Opening it SHALL delegate to the existing Expert/Factory detail authority; Activity MUST NOT store or execute the decision. A stale or superseded item MUST refresh rather than deciding an older revision.

#### Scenario: Factory plan awaits approval
- **WHEN** a Factory Run enters Awaiting plan approval
- **THEN** Activity Review shows one Expert-run-owned plan item that opens the exact current plan revision

#### Scenario: Factory result awaits final approval
- **WHEN** a Factory Run has current validation, review, and delivery evidence and enters Awaiting final approval
- **THEN** Activity Review shows one final-result item that opens the exact current evidence and delivery record

#### Scenario: Factory revision changes while review is open
- **WHEN** the reviewed run revision is no longer current before the user decides
- **THEN** no decision is recorded, refreshed evidence is shown, and focus remains within the owning review workflow
