# Expert Quality and Local Analytics Design

**Status:** Approved in design review
**Date:** 2026-07-30

## Goal

Improve Expert execution quality with measurable, local evidence. Each activation produces a run scorecard, each Expert may define a quality contract, and completed runs feed transparent performance analytics and human-reviewed improvement suggestions.

## Principles

- The user remains the authority for acceptance, rejection, ratings, and waivers.
- Claude and Codex may report evidence but cannot approve their own work.
- Performance data remains local to the desktop.
- Metrics distinguish reported evidence from independently confirmed evidence.
- Expert definitions remain portable; run history and project details do not travel with exports.
- The app stores structured summaries, not prompts, source excerpts, raw command output, logs, or secrets.
- Small samples produce an "insufficient evidence" state rather than a recommendation.

## Non-Goals

- Running project commands inside the desktop app
- Monitoring Claude or Codex sessions
- Cloud telemetry or team dashboards
- Automatically modifying, activating, or rolling back Experts
- Treating correlation as proof that an agent, skill, or Runbook caused an outcome
- Comparing runs whose quality contracts are materially incompatible without a warning

## Expert Quality Contract

An Expert may include an optional quality contract containing required and optional checks. Initial check kinds are:

- Tests
- Lint
- Build
- Security review
- Manual confirmation

Each check has a stable logical name, kind, requirement level, and evidence mode. Required checks block a normal accepted verdict when failed or missing. Optional checks produce warnings.

Evidence modes are:

- `clientReported`: Claude or Codex reports the result through MCP.
- `userConfirmed`: The user confirms the result in the desktop review.
- `appExecuted`: Reserved for a future trusted command runner and not implemented in the initial release.

A user may accept a run with missing or failed required checks only through an explicit waiver. Each waiver records the affected check, a bounded reason, and its timestamp.

Quality contracts belong to the portable Expert definition. Project-specific command lines and paths do not. At activation, the app maps logical checks such as tests or build to detected project tooling when unambiguous. Ambiguous checks remain unresolved and require user confirmation rather than guessed commands.

## Run Scorecard

Every activation creates a local run with an opaque ID and an immutable snapshot of:

- Expert ID and version
- Quality contract version
- Canonical registered project identity
- Selected client
- Lead and supporting agents
- Required and optional skills
- Runbook
- Start time

The mutable portion of a run contains:

- Current lifecycle state
- End time
- Structured check evidence
- Blockers
- Waivers
- User verdict
- Optional rating from 1 to 5
- Optional bounded private note

Run lifecycle states are:

1. `inProgress`
2. `awaitingReview`
3. One terminal state: `accepted`, `rework`, `rejected`, `cancelled`

Only the desktop user can move a run into a terminal verdict state. Failed and abandoned runs remain visible and count toward operational metrics where relevant.

## Evidence Submission

Activation returns the run ID with the generated starter prompt. Claude or Codex uses MCP to read the run contract and submit structured evidence.

Each evidence record contains:

- Stable check name
- Check kind
- Result: pass, fail, or skipped
- Evidence source
- Bounded command label
- Bounded summary
- Timestamp
- Caller-supplied idempotency key

MCP evidence writes require:

- Authenticated Claude or Codex client identity
- Existing per-client Source permission
- Exact canonical registered project
- Membership in the MCP project allowlist
- A run created for the same client and project

Repeated idempotency keys return the existing evidence record. Cross-client, cross-project, replayed with changed content, oversized, or terminal-run submissions fail closed. MCP clients cannot set verdicts, ratings, waivers, or modify the Expert.

Client-reported evidence is displayed as reported evidence. The app does not claim it independently executed or verified a command.

## Local Persistence and Retention

Run history is stored behind Tauri in a separate bounded local state file. It is not stored in renderer `localStorage`, the portable Expert definition, Activity, or MCP audit records.

The state uses the project's existing atomic-write and cross-process locking patterns. Retention is bounded by record count and total file size. When the cap is reached, the oldest terminal runs are evicted first. New runs fail rather than evicting active runs when all retained records are non-terminal.

The user can delete one run, an Expert's run history, or all run history. Deleting run history does not delete Experts, skill drafts, agents, Runbooks, or Activity entries.

Activity receives only safe lifecycle summaries such as "Expert run accepted." It excludes the requested outcome, starter prompt, project excerpts, evidence summaries, notes, waiver reasons, and command labels.

## Metrics

Metrics are derived from retained run records rather than persisted separately:

- Completion rate
- First-pass acceptance rate
- Verification pass rate
- Rework rate
- Rejection rate
- Waiver rate
- Median completed-run duration
- Most frequent blocker
- Results by Expert version

The Performance view can filter by project, client, Expert version, agent roster, and date. It clearly shows the active filter and sample size.

Version comparisons use only runs with compatible quality contracts. Incompatible or substantially changed contracts are shown separately. Cancelled runs affect completion rate but not verification or acceptance denominators.

## Improvement Coach

The Improvement Coach becomes available after at least five comparable terminal runs. It may surface:

- Regression warnings for an Expert version
- Repeated agent-substitution or blocker patterns
- Skills associated with stronger or weaker outcomes
- Frequently failed or waived checks
- Runbook steps associated with recurring failures
- A prior version that outperformed the current version

Every suggestion shows its sample size, comparison window, and supporting metric. Language uses "associated with" rather than causal claims.

Suggestions open an editable Expert draft or clone a prior version. They never modify the active Expert, publish a skill, change a Runbook, or activate an Expert automatically.

## Desktop Experience

### Expert detail

Add a `Performance` subview containing:

- Acceptance, verification, rework, duration, and waiver summary cards
- Sample-size and insufficient-evidence messaging
- Version comparison
- Recent runs
- Recurring blockers
- Improvement suggestions when eligible

### Activation review

Show the quality contract before activation, including unresolved checks. The generated prompt includes the run ID and instructs the client to submit structured evidence.

### Run review

Show the immutable activation snapshot, evidence source for every check, blockers, and waivers. The user can:

- Confirm manual checks
- Add a waiver reason
- Choose accepted, rework, rejected, or cancelled
- Add an optional rating and private note

The normal Accepted action remains disabled while required checks are failed or missing. Accept-with-waiver is a separate explicit action.

## Error Handling

- A stale or missing run returns a stable not-found error.
- Evidence for an unknown contract check is rejected.
- Conflicting idempotent retries are rejected.
- Evidence submitted after terminal review is rejected.
- Corrupt or oversized run state fails closed without replacing the last valid file.
- A failed persistence write leaves both the prior run state and Expert state unchanged.
- Analytics omit malformed records and surface a local data-integrity warning.
- Missing project tooling leaves a check unresolved; it does not guess a command.

## Testing

Backend tests cover:

- Every valid and invalid run lifecycle transition
- User-only verdicts and waivers
- Evidence normalization, bounds, and idempotency
- Cross-client and cross-project rejection
- Exact canonical path and allowlist enforcement
- Contract snapshot and version isolation
- Retention and active-run preservation
- Atomic failure behavior
- Metric denominator and median correctness
- Incompatible contract comparisons
- Improvement threshold and sample-size reporting
- Redaction from Activity and MCP audit records

Frontend checks cover:

- Keyboard and focus order
- Accessible labels and status text
- Empty, insufficient-evidence, loading, corrupt-state, and error states
- Required-check gating and the separate waiver action
- Filters and visible sample sizes
- Version comparison and run drill-down
- Local deletion confirmation

End-to-end smoke tests cover activation, MCP evidence submission from Claude and Codex, user review, metric refresh, regression visibility, and history deletion.

## Delivery

### Phase 1: Measurable runs

- Portable quality contracts
- Local run lifecycle and retention
- MCP contract read and evidence submission
- Desktop run review and waivers
- Performance dashboard and version comparison

### Phase 2: Reliability feedback

- Regression alerts
- Recurring blocker and waiver detection
- Evidence-backed improvement suggestions
- Clone a prior version as a new editable draft

Phase 2 starts only after Phase 1 produces trustworthy local run records and metrics.
