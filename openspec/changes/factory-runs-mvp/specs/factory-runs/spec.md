## Purpose

Lets a user govern one bounded software work order from ready-project preflight through an approved plan, externally executed build, evidence-backed review, and merge-ready delivery without turning Agency Agents into a coding runtime.

## ADDED Requirements

### Requirement: A Factory Run is an exact bounded Expert Run contract
The system SHALL represent a Factory Run as an optional workflow attached to one existing Expert Run. Its immutable work-order snapshot MUST contain a bounded ticket reference, title, objective, acceptance criteria, non-goals, exact registered project, exact Expert id and version, selected Playbook or runbook, optional Workspace Pack revision, quality contract, risk classification, and fresh readiness evidence. Existing Expert Runs without a Factory workflow MUST continue to load and behave unchanged.

#### Scenario: User creates a Factory Run for a ready project
- **WHEN** the selected registered project is Ready, the exact Expert activation plan has no blockers, and the work order is within all limits
- **THEN** the system creates one Factory-enabled Expert Run with the reviewed work order and preflight snapshot

#### Scenario: Work order or preflight is invalid
- **WHEN** required work-order content is empty or oversized, the project is not registered, readiness is not Ready, or Expert activation is blocked
- **THEN** the system creates no Factory Run and identifies the blocking evidence without changing the project

#### Scenario: Existing non-Factory run is loaded
- **WHEN** a stored Expert Run has no Factory workflow
- **THEN** the system preserves its existing state, evidence, review, and coaching behavior

### Requirement: Factory Runs follow one fixed revision-bound lifecycle
The system SHALL use the fixed lifecycle Preflight, Planning, Awaiting plan approval, Build, Validation, Independent review, Delivery, Awaiting final approval, and Completed. Cancellation SHALL be terminal, while a blocker SHALL pause the current phase without erasing it. Every state-changing request MUST name the expected run revision; a stale request MUST perform no transition. Validation failure or review rework SHALL return to Build with a new attempt, and automated build/review cycles MUST stop after three attempts until a human resolves the run.

#### Scenario: Expected revision advances one phase
- **WHEN** the owning actor completes the current phase with its current revision and all phase gates are satisfied
- **THEN** the system atomically advances once and returns a new revision

#### Scenario: Concurrent or stale transition arrives
- **WHEN** two actors submit the same expected revision or a request names an older revision
- **THEN** exactly one valid transition can succeed and every stale request performs no change

#### Scenario: Rework reaches the attempt limit
- **WHEN** validation or independent review requests another build after three build/review attempts
- **THEN** the run pauses for human intervention and does not begin another automated attempt

### Requirement: Build starts only from an approved exact plan
Planning SHALL produce bounded cited plan content, a client-reported base commit identifier, declared checks, risks, and known limitations. The system SHALL compute the plan revision from the complete canonical work contract and submitted plan. Only a desktop user MAY approve or reject that exact revision. Approval MUST bind the base commit and plan revision used by every later phase; rejection SHALL return to Planning without ending the run.

#### Scenario: User approves the current plan
- **WHEN** a desktop user reviews the complete current plan revision and approves it
- **THEN** Build becomes available with the exact approved plan and base commit

#### Scenario: Plan content changes after review
- **WHEN** the work order, project, Expert version, plan content, declared checks, policy, or base commit differs from the approved revision
- **THEN** Build remains unavailable until the new complete revision receives desktop approval

#### Scenario: MCP client attempts plan approval
- **WHEN** an external worker calls any Factory MCP operation while the run awaits plan approval
- **THEN** it can read its permitted status but cannot approve, reject, or bypass the human gate

### Requirement: Evidence is attempt-bound and honestly client-reported
Every Factory evidence item MUST bind the run, phase, attempt, claim generation, work-contract revision, approved plan revision, base commit, head commit when applicable, quality-check id, result, command label, exit code, bounded summary, artifact metadata, provenance, idempotency key, and submission time. Provenance SHALL be displayed as client-reported unless a future trusted verifier supplies a stronger type. The latest evidence for a required check in the current attempt MUST determine its result; evidence from another revision, head, or attempt MUST NOT satisfy the current gate. Raw command output, diffs, repository files, credentials, and unbounded content MUST NOT be persisted.

#### Scenario: Current required evidence passes
- **WHEN** the latest current-attempt evidence for every required check reports pass, a zero exit code where a command was reported, and matching plan/base/head bindings
- **THEN** the validation gate can advance while identifying the evidence as client-reported

#### Scenario: A later failure follows an earlier pass
- **WHEN** a current-attempt check receives pass evidence followed by fail evidence
- **THEN** the check is failing and the earlier pass cannot satisfy validation or final acceptance

#### Scenario: Evidence belongs to stale work
- **WHEN** evidence names another attempt, claim generation, plan revision, base commit, or head commit
- **THEN** the system rejects it without changing the current validation state

### Requirement: Independent review and delivery are bound to the exact head
Independent review SHALL use a worker identity distinct from the current build claimant and SHALL submit a bounded severity-classified report for the exact head commit. A passing report MAY advance to Delivery; a rework report MUST invalidate current validation and review and return to Build with a new attempt. If distinct automated review is unavailable, only an explicit desktop human waiver MAY replace it, and the result MUST NOT claim independent automated review. Delivery MUST include a bounded HTTPS pull-request or equivalent review reference, head commit, final evidence summary, and known limitations before final approval becomes available.

#### Scenario: Distinct reviewer passes the current head
- **WHEN** a non-builder worker submits a passing review bound to the validated current head
- **THEN** the run advances to Delivery without changing repository or pull-request state

#### Scenario: Builder attempts to review its own work
- **WHEN** the current build claimant attempts to claim or complete Independent review
- **THEN** the system rejects the action and retains the review phase

#### Scenario: Delivery is incomplete
- **WHEN** the pull-request reference is absent or invalid, the head differs from validated/reviewed evidence, or known limitations are missing
- **THEN** final approval remains unavailable

### Requirement: Final acceptance is a desktop-only exact human decision
Only a desktop user MAY accept, request rework, reject, or cancel the final Factory result. Acceptance MUST require the current approved plan, current validated head, current independent review or explicit human waiver, complete delivery evidence, and explicit waivers for any missing required checks. A terminal decision MUST freeze claims, artifacts, evidence, blockers, and transitions while retaining them for history and coaching.

#### Scenario: User accepts a complete current result
- **WHEN** every final gate is current and the desktop user accepts the run
- **THEN** the Expert Run becomes Accepted and retains the exact delivery and evidence record

#### Scenario: Final decision sees stale evidence
- **WHEN** the head, plan, validation, review, or delivery changed after the final review opened
- **THEN** no terminal decision is recorded and the user must review the refreshed result

### Requirement: Cancellation revokes control-plane authority without claiming process termination
The desktop SHALL allow a user to cancel any non-terminal Factory Run. Cancellation MUST revoke every current claim and reject later submissions, but the system MUST state that it cannot terminate an already-running external Claude Code, Codex, Git, CI, or other process. Cancellation MUST NOT delete repository work, branches, worktrees, artifacts, evidence, or history.

#### Scenario: User cancels active external work
- **WHEN** the desktop user cancels a claimed Build or Validation phase
- **THEN** the run becomes Cancelled, its claim can no longer submit, retained evidence remains visible, and the UI states that the external process may still need to be stopped separately

### Requirement: Factory history is bounded without losing active work
Factory workflows SHALL reuse the existing bounded Expert Run persistence and transactional mutation authority. Restart MUST restore the exact committed run, revision, phase, claim, attempts, approvals, evidence, and delivery metadata. Capacity pruning MAY remove only the oldest terminal runs; if capacity is exhausted entirely by active runs, creation MUST fail closed rather than discarding active work.

#### Scenario: App restarts during a claimed phase
- **WHEN** the application or MCP process restarts before the claim expires
- **THEN** the same committed phase and claim remain authoritative and no external work is automatically resumed

#### Scenario: Active runs fill retention capacity
- **WHEN** no terminal run is available for bounded pruning
- **THEN** creating another run fails with an honest capacity error and every active run remains intact

### Requirement: Factory control is accessible and exception-driven
The existing Experts surface SHALL provide a Factory control-room projection showing title, project, current phase, elapsed time, claimant, attempt, validation state, blockers, required human action, head, delivery reference, limitations, and improvement proposal. Existing Activity Review SHALL surface plan and final approval items. Async state changes MUST be announced, state MUST not rely on color alone, all actions MUST be keyboard reachable, and closing delegated review MUST restore the exact initiating focus.

#### Scenario: User monitors multiple runs
- **WHEN** Factory Runs are progressing, blocked, or awaiting approval
- **THEN** the control room distinguishes active progress, exceptions, and required human decisions without presenting a chat interface

#### Scenario: Keyboard user completes an approval
- **WHEN** a keyboard user opens a Factory approval from Activity and returns after deciding or closing it
- **THEN** focus returns to the exact initiating review control and the resulting state is announced

### Requirement: Factory improvements remain inert proposals
A terminal Factory Run MAY retain one bounded root-cause and improvement proposal that identifies an observed failure class and a proposed test, rule, Skill, Expert, Playbook, or instruction change. The system MUST label external analysis as client-reported and MUST NOT automatically apply, publish, install, approve, or share the proposal.

#### Scenario: External worker proposes a factory improvement
- **WHEN** a terminal submission includes a valid bounded improvement proposal
- **THEN** the proposal remains locally reviewable with its provenance and produces no configuration or repository mutation

### Requirement: Agency Agents never becomes the Factory executor
Factory behavior SHALL remain a local control plane. The system MUST NOT launch models or coding clients, construct arbitrary shell commands, invoke Git or repository tests, create or modify worktrees or branches, write project source files, contact pull-request services, verify Git ancestry, merge, deploy, emit telemetry, upload work artifacts, or represent client-reported execution as independently verified.

#### Scenario: Factory Run reaches Build
- **WHEN** Build becomes available after plan approval
- **THEN** the app exposes the immutable contract to an authorized external worker and performs no repository execution itself
