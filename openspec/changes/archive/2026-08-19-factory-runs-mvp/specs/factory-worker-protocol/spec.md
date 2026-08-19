## Purpose

Defines the bounded pull-based MCP contract through which authorized external Claude Code and Codex workers discover, claim, and report Factory stages without receiving approval or application execution authority.

## ADDED Requirements

### Requirement: Work discovery is exact and project-scoped
The Factory worker protocol SHALL list only currently available worker phases for an exact canonical registered project authorized for the connected client. A project path MUST be supplied; the protocol MUST NOT provide a global work queue, private paths from other projects, terminal runs, human approval phases, or stages already held by a valid claim.

#### Scenario: Authorized client lists one project
- **WHEN** a connected client with read permission and exact project allowlisting requests available Factory work for that project
- **THEN** it receives only claimable phases in that project with bounded identity and status metadata

#### Scenario: Client omits or cannot access the project
- **WHEN** the request omits the project, names an unregistered path, or lacks exact allowlisting
- **THEN** the protocol returns no work and reveals no run metadata

### Requirement: A phase has one bounded renewable claim
Claiming SHALL create one opaque claim id and generation bound to the run, current phase, run revision, connected worker identity, claimed time, and expiry. Only one unexpired claim MAY own a phase. An identical idempotent retry MUST return the original claim, while reusing the key with different input MUST fail. A valid submission MAY renew the fixed two-hour lease. Expiry or desktop release SHALL make the same phase claimable without advancing it, and every prior claim generation MUST remain invalid.

#### Scenario: Two workers claim concurrently
- **WHEN** two authorized workers claim the same available phase at the same revision
- **THEN** exactly one claim succeeds and the other receives a busy or stale result

#### Scenario: Claim expires and is reassigned
- **WHEN** no valid renewal arrives before expiry and another worker claims the phase
- **THEN** the new generation owns the phase and the old claim cannot submit or complete it

#### Scenario: Idempotency key conflicts
- **WHEN** a worker reuses an existing claim idempotency key with different run, phase, project, or revision data
- **THEN** the protocol rejects the request and preserves the original claim

### Requirement: Claim contracts are immutable and phase-specific
An authorized claimant SHALL be able to read an immutable bounded contract for only its current claim. The contract MUST include the exact work order, project, Expert/version, selected configuration references, phase, attempt budget, run revision, approved plan/base binding when available, current head binding when applicable, required quality checks, permitted submission shapes, and lease expiry. It MUST NOT include secrets, waiver reasons, raw repository content, private data from another run, or capabilities outside the phase.

#### Scenario: Claimant reads its current contract
- **WHEN** the worker presents the current claim for its exact project and phase
- **THEN** the protocol returns the immutable current-phase contract and no approval or execution capability

#### Scenario: Another worker reads the claim
- **WHEN** a different worker identity or stale claim generation requests the contract
- **THEN** the protocol rejects the request without revealing contract content

### Requirement: Worker submissions are bounded, bound, and idempotent
Artifacts, evidence, blockers, and phase completion MUST include the exact run, project, claim, claim generation, run revision, phase, attempt, and idempotency key. Artifact submissions MUST contain bounded kind, label, safe reference, digest, byte size, summary, and applicable base/head binding rather than raw content. Identical retries SHALL return the original logical result; conflicting reuse SHALL fail. Unknown checks, malformed digests, non-zero command evidence reported as pass, oversized content, stale bindings, terminal runs, or submissions outside the claim's phase MUST be rejected without partial state.

#### Scenario: Worker retries an identical evidence submission
- **WHEN** the same claimant repeats the same fully bound request with the same idempotency key
- **THEN** it receives the original result and the run contains one logical evidence item

#### Scenario: Submitted pass reports command failure
- **WHEN** evidence reports pass with a non-zero exit code
- **THEN** the protocol rejects it and the validation gate remains unchanged

#### Scenario: Worker completes a stale head
- **WHEN** phase completion names a plan, base, head, attempt, or revision different from the current claim contract
- **THEN** the protocol rejects completion and does not advance the run

### Requirement: Worker identity is server-assigned and review is distinct
The protocol SHALL derive client and connection identity from the server transport rather than request payloads. Unknown clients and the generic shared HTTP identity MUST remain unable to claim or mutate Factory work in the MVP. The current build claimant MUST NOT claim or complete Independent review. The UI and receipt MUST describe the resulting guarantee as a distinct worker-session review, not as cryptographic human identity proof.

#### Scenario: Payload attempts to spoof another client
- **WHEN** a claim or submission includes actor fields that differ from the server-assigned identity
- **THEN** the server ignores or rejects the supplied identity and authorizes only from its own connection context

#### Scenario: Generic HTTP client attempts a claim
- **WHEN** the existing shared HTTP MCP transport requests a Factory claim
- **THEN** it remains read-only for Factory work and no claim is created

### Requirement: Factory tools reuse existing authorization and audit boundaries
Every Factory tool SHALL be classified before dispatch through the existing MCP action policy, exact project allowlist, serialized policy lease, and bounded attempt/terminal audit. Discovery and contract reads SHALL require existing read authority. Claims and submissions SHALL require the existing default-denied source authority. The protocol MUST NOT add a second permission family, bypass paranoid mode or client overrides, persist waiver reasons in MCP views, or omit a terminal audit result after an attempted mutation.

#### Scenario: Source mutation permission is disabled
- **WHEN** an otherwise allowlisted client attempts to claim or submit while its existing source action is denied
- **THEN** the operation is rejected and a bounded failed audit result is retained

#### Scenario: Authorized submission completes
- **WHEN** policy, project, claim, revision, and input validation all succeed
- **THEN** the protocol records bounded attempt and terminal audit evidence without prompt, artifact content, token, or waiver-reason leakage

### Requirement: MCP cannot approve or execute Factory work
The Factory worker protocol MUST NOT expose operations that approve plans, accept final results, grant waivers, create branches or worktrees, run commands or tests, invoke models or coding clients, contact pull-request services, merge, deploy, alter project source, or mutate shared improvement targets.

#### Scenario: Worker completes Planning
- **WHEN** a valid worker submits and completes the current Planning phase
- **THEN** the run moves only to desktop plan review and no later phase or repository action begins automatically
