# Post-Action Receipts Specification

## Purpose

Preserves a durable, privacy-safe account of every destination attempted by a completed bulk Agent or Skill mutation and lets users return to that exact evidence from the completion surface.

## Requirements

### Requirement: Every completed bulk mutation produces one exact receipt
After a bulk Agent or Skill mutation reaches terminal outcomes, the system SHALL record one receipt containing the operation, aggregate success and failure counts, and one item for every attempted mutation. Each item MUST identify its artifact kind, name, and terminal success or failure. Every successful item MUST include the exact destination changed. A failed item MUST include its exact planned destination when the existing mutation path knew it before execution; otherwise it MUST state that no destination was returned and no changed destination is claimed. Failures MUST include bounded safe detail.

#### Scenario: All destinations succeed
- **WHEN** a bulk mutation completes successfully for every attempted destination
- **THEN** one receipt records every exact changed destination as successful and its aggregate counts match the item outcomes

#### Scenario: Some destinations fail
- **WHEN** a bulk mutation completes with a mix of successful and failed destinations
- **THEN** one receipt records the exact terminal outcome for every attempted item and every changed destination without discarding later work after a failure

### Requirement: Receipts reuse the bounded local Activity journal
Receipts SHALL remain local and persist through the existing bounded Activity journal. Existing journal entries without receipt items MUST continue to load and render. Receipt content MUST be normalized at the journal boundary so secrets, credentials, control characters, and unbounded detail are not persisted.

#### Scenario: App restarts after a bulk action
- **WHEN** the Activity journal is hydrated after a receipt was persisted
- **THEN** the receipt retains its aggregate outcome and destination items within the existing journal retention policy

#### Scenario: Older entry has no receipt
- **WHEN** the journal hydrates an entry created before post-action receipts existed
- **THEN** the entry remains readable with its existing summary and no receipt-detail control

#### Scenario: Failure detail contains sensitive content
- **WHEN** a terminal error contains a token, credential, private key, control character, or excessive text
- **THEN** the persisted and rendered receipt contains only the bounded redacted representation

### Requirement: Activity exposes receipt detail accessibly
The Activity surface SHALL show that a journal row has receipt details and SHALL let keyboard and assistive-technology users disclose the per-item results. The disclosed content MUST expose the operation summary, artifact identity, terminal outcome, and exact changed or known planned destination without relying on color alone. A failed item with no returned destination MUST state that limitation instead of inventing a path.

#### Scenario: User opens a receipt row
- **WHEN** the user activates the receipt-detail control in Activity
- **THEN** the system discloses every recorded item, its textual terminal outcome, and any exact changed or known planned destination while preserving the journal's chronological grouping

#### Scenario: Receipt contains failures
- **WHEN** disclosed receipt items include failures
- **THEN** each failed item is identified textually and exposes its bounded safe detail

### Requirement: Completion surfaces link to the exact receipt
Each user-visible bulk completion toast or retained result surface SHALL offer a localized `View Activity` action when its receipt was recorded. Activating the action MUST open Activity, focus and reveal the exact receipt, and remain safe when the receipt is no longer retained.

#### Scenario: User follows a completion action
- **WHEN** the user activates `View Activity` from a completed bulk action
- **THEN** Activity opens with the matching receipt disclosed, scrolled into view, and keyboard focus moved to its receipt control

#### Scenario: Receipt is no longer retained
- **WHEN** the user follows a stale receipt reference after the journal retention policy removed it
- **THEN** Activity opens without an incorrect row being focused and announces that the receipt is no longer available

### Requirement: Receipts do not broaden mutation authority
Receipt creation and navigation SHALL occur only after existing mutation paths resolve and SHALL NOT alter planning, approval, reconciliation, backup, rollback, authorization, or execution behavior. Receipts SHALL NOT trigger network access, telemetry, cloud persistence, native notifications, or additional filesystem mutation.

#### Scenario: Receipt persistence fails
- **WHEN** the local Activity mirror cannot persist a completed receipt
- **THEN** the underlying mutation outcome remains unchanged and the system does not repeat or roll back the mutation solely to recreate the receipt

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
