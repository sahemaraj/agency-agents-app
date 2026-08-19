## Context

See `proposal.md` for motivation and the capability specs for normative behavior. The application already has one durable Expert Run authority in `src-tauri/src/expert_runs.rs`, transactional Expert activation in `src-tauri/src/experts.rs`, canonical project readiness in `src-tauri/src/install/mod.rs`, an authorized and audited MCP router in `src-tauri/src/skills/mcp.rs`, and Expert/Activity projections in the existing Svelte stores and components. Factory Runs must extend those paths without becoming a second job database, approval engine, or execution runtime.

Existing run persistence is a bounded SQLite-backed collection with a 4 MiB document limit and a 500-run capacity. Existing clients use both `claude` and `claudeCode` labels at different boundaries. Existing MCP HTTP traffic has a shared generic identity, so it cannot establish the distinct worker-session identity required for Factory mutations.

## Goals / Non-Goals

**Goals:**

- Preserve one transactional source of truth for normal and Factory-enabled Expert Runs.
- Make every worker mutation project-scoped, revision-bound, claim-bound, idempotent, authorized, and auditable.
- Keep desktop approval decisions authoritative while exposing read/claim/report operations to eligible external workers.
- Reuse the current Experts and Activity surfaces and their accessibility/focus patterns.
- Remain backward compatible with stored non-Factory runs and existing MCP clients.

**Non-Goals:**

- Running agents, shell commands, Git, repository checks, CI, pull-request APIs, merge, or deployment inside Agency Agents.
- Adding a scheduler, global work queue, new database document, permission family, route, runtime, dependency, or background daemon.
- Treating client-reported commits, commands, results, artifacts, or reviewer identity as independently verified.
- Automatically applying Factory improvement proposals.

## Decisions

### 1. Extend the existing Expert Run document

Add a serde-defaulted optional `factory` workflow to the existing Expert Run model in `src-tauri/src/expert_runs.rs`. The nested workflow owns the immutable work-order snapshot, phase, blocker overlay, revision, attempts, claim, idempotency records, plan approval, bounded artifacts/evidence, review, delivery, terminal decision, and improvement proposal. Normal runs omit the field and retain current behavior.

This keeps all state changes under the existing serialized mutation and SQLite commit boundary. A separate Factory table/document was rejected because it would create synchronization and recovery failure modes for no MVP benefit. New production files are unnecessary: the existing module already owns run validation, persistence, retention, and transitions.

Retention continues to use the existing 4 MiB and 500-run limits, but pruning changes to select only terminal runs. Creation fails closed if active runs consume capacity. All Factory strings and vectors receive explicit conservative bounds before serialization.

### 2. Use a fixed state machine with optimistic revisions

The optional workflow uses the spec's fixed phases plus a terminal result and a blocker overlay. Every mutation takes `expected_revision`; the existing serialized mutation lock validates it and commits exactly one increment. Rework increments the build/review attempt and invalidates evidence derived from the prior attempt. The attempt ceiling is the constant three, not configuration.

Claims use an opaque id, monotonically increasing generation, server-assigned worker identity, fixed two-hour expiry, and last-renewed time. Expiry or desktop release clears current ownership without advancing the phase; the next claim increments generation so old submitters remain invalid. A clock value is passed into transition helpers so unit tests stay deterministic without a new time abstraction.

### 3. Canonicalize client identity once at the MCP boundary

Add one small normalizer in `src-tauri/src/skills/mcp.rs` that maps existing desktop/config labels such as `claudeCode` to the canonical MCP identity `claude`, while preserving `codex` and rejecting unknown identities for Factory mutations. Authorization uses the server transport context only; actor labels in tool payloads are not trusted.

The generic shared HTTP MCP identity remains unable to claim or submit. This deliberately limits MVP worker mutation to transports that already provide a stable server-assigned client/connection identity. Adding authentication to the generic HTTP transport is deferred until a real remote-worker requirement exists.

### 4. Carry Factory preflight through current activation

Extend the existing activation input and transaction in `src-tauri/src/experts.rs` with an optional bounded work order. Factory creation reuses the same Expert/version, project registration, Workspace Pack, policy, and recovery checks as normal activation. Expose the existing project-readiness computation from `src-tauri/src/install/mod.rs` as an internal helper rather than duplicating readiness rules.

The work-contract revision is a deterministic digest over canonical serialized work-order and configuration fields. The approved plan revision similarly includes the full work contract, plan, declared checks, limitations, and base commit. Existing digest utilities are reused; no hashing dependency is added.

### 5. Add Factory tools to the existing MCP router

Register only these bounded operations in `src-tauri/src/skills/mcp.rs`: project-scoped work discovery, claim, claim-contract read, artifact submission, evidence submission, blocker submission, and phase completion. All requests pass through the current tool classification, exact project allowlist, policy lease, source/read action decision, audit attempt, and terminal audit result before reaching Expert Run transition helpers.

Read operations use existing read authority. Claims and submissions use the existing default-denied source action because they mutate only the app's control-plane record; no new permission family is introduced. Plan/final decisions, waivers, cancellation, and claim release are Tauri desktop commands registered through the existing command path in `src-tauri/src/lib.rs`, never MCP tools.

Idempotency is stored inside the bounded Factory workflow as a key plus canonical request digest and logical result reference. Identical retries return that result; conflicting reuse fails before mutation. This is simpler and safer than a separate idempotency store because the record and state transition commit atomically.

### 6. Keep evidence metadata-only and latest-result authoritative

Extend the existing Expert Run evidence shape rather than creating an artifact service. A Factory evidence record contains only bounded labels, bindings, digests, sizes, summaries, result, reported command label/exit code, provenance, and timestamps. Raw outputs, diffs, files, prompts, credentials, and waiver reasons are rejected or omitted at the input boundary.

Gate evaluation filters to the current work/plan/attempt/claim/base/head tuple and selects the latest record per required check. Therefore a later failure overrides an earlier pass without destructive edits. All external evidence is displayed as `clientReported`; the app does not infer Git ancestry or re-run checks.

### 7. Project Factory state through existing UI authorities

Extend shared types and API wrappers in `src/lib/types.ts` and `src/lib/api.ts`, then project Factory state through `src/lib/stores/experts.svelte.ts`. `src/lib/components/Experts.svelte` remains the control and decision authority: it adds the bounded work-order creation fields, stage/evidence detail, desktop approval/rework/reject/cancel/release actions, and terminal improvement proposal.

Extend the existing Activity projection in `src/lib/stores/activity.svelte.ts` and `src/lib/components/ActivityHistory.svelte` for two review-item kinds and one Factory receipt kind. Activity delegates decisions back to Experts and stores no duplicate gate state. Existing disclosure, live-region, keyboard, and initiating-focus restoration patterns are reused. No new route or component is planned; a new component is justified only if the existing file becomes measurably harder to test during implementation.

### 8. Extend the current receipt union

Add a Factory terminal variant to the existing bounded post-action receipt model. Receipt creation is best-effort after the authoritative Expert Run terminal commit, matching current Activity semantics: journal failure cannot repeat or roll back an accepted/rejected/cancelled decision. Delivery references are validated as bounded HTTPS values but remain inert text until the user explicitly chooses a future link action.

## Risks / Trade-offs

- Client-reported evidence can be false or mistaken -> label provenance everywhere, bind it tightly, reject inconsistent exit codes, and never call it verified.
- A worker can keep operating after lease expiry or cancellation -> reject all stale submissions and state clearly that external process termination is outside the app.
- Two-hour leases may delay recovery after a crash -> allow safe desktop release and generation-based reassignment; avoid heartbeat infrastructure until needed.
- Generic HTTP cannot provide safe mutable worker identity -> keep it read-only for Factory MVP rather than weakening distinct-review guarantees.
- Nested bounded state increases Expert Run document size -> apply per-field/per-vector limits and the existing 4 MiB serialization limit; store metadata rather than content.
- Terminal-only pruning can exhaust capacity -> fail closed with a clear capacity error rather than lose active work.
- Adding Factory controls to Experts increases UI density -> show summary first, disclose phase evidence on demand, and keep Activity exception-driven.
- One large existing Svelte component may become difficult to maintain -> first reuse local sections and tests; extract only if implementation evidence demonstrates a distinct reusable unit.

## Migration Plan

1. Add serde-defaulted optional Factory fields and TypeScript optional counterparts; verify legacy fixtures and persisted runs load unchanged.
2. Add and test pure validation/transition helpers before exposing commands or MCP tools.
3. Thread optional Factory creation through existing activation and readiness checks.
4. Register MCP tools and desktop commands behind their existing authorization boundaries.
5. Add Experts/Activity projections and receipts after the backend contract is stable.
6. Run Rust, frontend, build, lint, security-boundary, MCP inventory/smoke, browser accessibility, and strict OpenSpec verification before merge.

Rollback is a code rollback. Stored runs containing the optional `factory` field remain valid JSON for old readers because unknown fields are ignored; no database migration or destructive data rollback is required. During rollout, no automatic conversion of existing runs occurs.
