## ADDED Requirements

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
