## Purpose

Provides one accessible Activity projection of every existing pending approval shape without creating another decision engine or weakening domain ownership.

## ADDED Requirements

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

