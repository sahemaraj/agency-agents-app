## ADDED Requirements

### Requirement: A terminal Factory Run produces one bounded result receipt
After a Factory Run reaches Accepted, Rework, Rejected, Cancelled, or an attempt-exhausted blocked result, the system SHALL record one bounded local receipt containing the run and work-order identity, project label, terminal outcome, approved plan revision when present, base and head identifiers when present, required-check summary, independent-review or waiver status, safe HTTPS delivery reference when present, retry count, known limitations, and bounded safe failure detail. Evidence and external execution MUST be labelled client-reported. The receipt MUST NOT contain prompt text, raw plans, command output, diffs, repository files, credentials, private absolute paths, or human waiver reasons.

#### Scenario: User accepts a delivered Factory Run
- **WHEN** final desktop acceptance records a complete current delivery
- **THEN** one Activity receipt summarizes the exact accepted revision, evidence provenance, delivery reference, and limitations

#### Scenario: Factory Run is cancelled during external work
- **WHEN** desktop cancellation makes the run terminal
- **THEN** one receipt records cancellation and states that Agency Agents revoked control-plane authority but did not terminate or delete external work

#### Scenario: Factory receipt persistence fails
- **WHEN** the bounded local Activity journal cannot retain the terminal Factory receipt
- **THEN** the Expert Run terminal result remains unchanged and no external work, transition, or decision is repeated

### Requirement: Factory receipts remain inert accessible evidence
Activity SHALL expose Factory receipt detail with textual phase, result, provenance, evidence, review, limitation, and delivery labels without relying on color. Following a Factory completion action SHALL reveal and focus the exact retained receipt or announce that it is no longer retained. Viewing or following a Factory receipt MUST NOT open an external URL automatically, contact a network service, resume a run, reclaim a phase, apply an improvement proposal, or broaden any mutation authority.

#### Scenario: Keyboard user opens Factory receipt detail
- **WHEN** the user activates the receipt disclosure or follows `View Activity`
- **THEN** the exact retained Factory evidence is disclosed and focused with textual status and provenance

#### Scenario: User inspects a delivery reference
- **WHEN** a Factory receipt contains a valid delivery reference
- **THEN** Activity presents it as inert bounded evidence and performs no network request or repository action
