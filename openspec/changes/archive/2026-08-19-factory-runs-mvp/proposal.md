## Why

Agency Agents can configure Experts, prove project readiness, collect bounded run evidence, and preserve human approval, but it cannot yet carry one bounded work request through planning, external implementation, validation, independent review, and delivery as one durable workflow. Factory Runs add that missing control-plane primitive while keeping Claude Code, Codex, Git, CI, and pull-request tooling outside the application runtime.

## What Changes

- Extend the existing Expert Run lifecycle with an optional bounded Factory workflow for one work order, one registered project, one approved plan, and one merge-ready pull-request reference.
- Add a fixed pull-based worker protocol through the existing MCP server for stage discovery, exclusive claims, immutable contracts, idempotent artifact/evidence submission, blockers, and stage completion.
- Add desktop-only plan and final approval gates, revision-bound validation evidence, bounded rework attempts, cancellation, and honest client-reported provenance.
- Extend the existing Experts and Activity surfaces into a Factory control room without adding a route, chat interface, second approval engine, or separate persistence authority.
- Record a bounded local terminal receipt and an inert improvement proposal without automatically mutating Experts, Skills, Playbooks, instructions, rules, repositories, pull requests, merges, or deployments.
- Preserve the existing no-runtime boundary: Agency Agents does not launch coding clients, run repository commands, create worktrees, verify Git ancestry, execute tests, create pull requests, merge, or deploy.

## Capabilities

### New Capabilities

- `factory-runs`: Durable fixed-stage work orders, revision-bound human gates, attempts, evidence, review, delivery, cancellation, and improvement proposals over the existing Expert Run authority.
- `factory-worker-protocol`: Project-scoped pull-based MCP discovery, claims, immutable contracts, leases, idempotent submissions, authorization, and audit for external Claude Code and Codex workers.

### Modified Capabilities

- `unified-review`: Include Factory plan and final approval items while preserving Expert-run ownership and exact focus restoration.
- `post-action-receipts`: Represent one terminal Factory result as bounded, local, privacy-safe Activity evidence without broadening execution authority.

## Impact

- Extends `src-tauri/src/expert_runs.rs`, `experts.rs`, the existing MCP router in `skills/mcp.rs`, and current Tauri command registration; no new database document, table, router, permission family, runtime, or dependency is added.
- Extends the existing Expert store, Experts detail/run UI, Activity Review projection, shared TypeScript types, API wrappers, Activity receipts, and existing tests; no new top-level navigation surface is added.
- Reuses canonical Project Readiness, Workspace Pack, quality-contract, SQLite, MCP audit, Unified Review, Recovery, Activity, and deterministic improvement-coach behavior.
- Existing non-Factory Expert Runs remain backward compatible through an absent optional Factory workflow.
