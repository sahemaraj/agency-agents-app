## Context

See `proposal.md` for motivation and `specs/*/spec.md` for observable behavior. The app already owns source-aware Agent and Skill lifecycle, domain-specific approvals, exact history/rollback, Workspace Pack inspection, project instructions, MCP inventory, Doctor evidence, Activity receipts, a bounded SQLite document store, and prepared filesystem journals. The design must extend those authorities without creating parallel state, installers, approvals, recovery engines, or runtime execution.

## Goals / Non-Goals

**Goals:**

- Keep the eight capabilities coherent through one bounded control-center document and the existing domain ledgers.
- Make every derived status explainable, independently unavailable, and unable to mutate by itself.
- Preserve exact revision, ownership, no-link/reparse, journal, verification, and rollback guarantees for new artifact shapes.
- Reuse existing Activity, Projects, Runbooks, Settings, plan/apply, and recovery surfaces with keyboard-safe focus and 375px reflow.

**Non-Goals:**

- Running Agents or playbooks, hot-restoring SQLite, configuring foreign MCP servers, executing OpenClaw, cloud sync, telemetry, marketplace behavior, or silent repair.
- Generalizing all existing lifecycle code behind a new framework.
- Replacing per-domain approval authority with the unified Review projection.

## Decisions

### Persist cross-domain cursors and baselines in one bounded control-center document

Catalog snapshot/feed, project baselines, subscriptions, recommendations, dismissal state, and cursors share one versioned SQLite document. Refresh ordering is old snapshot → durable feed/new snapshot → durable recommendations → cursor advance.

Alternative: Activity rows or browser storage. Rejected because clearing either would lose durable catalog truth and crash ordering.

### Derive readiness from explicit intent and independent evidence

A project baseline stores exact source-aware references and passive requirements. Agent, roster, Skill, instruction, MCP, and tool checks run independently and combine through a conservative precedence; opaque requirements remain unverifiable.

Alternative: infer intent from current installs. Rejected because observed state cannot prove what the project requires.

### Keep recommendations and unified review as projections

Recommendations re-resolve exact references into existing plans. Review delegates to existing approval surfaces. Neither stores a new approval decision nor gains mutation authority.

Alternative: one new cross-domain approval service. Rejected because it would duplicate revision checks, audit, and ownership.

### Aggregate recovery without broadening recovery authority

The Recovery Center lists existing Agent/Skill history and verified storage backup controls. Exact rollback stays with the owning domain; SQLite restore is documented as offline/manual only.

Alternative: replace the live database from the UI. Rejected because WAL state, locks, and in-memory caches could diverge.

### Treat catalog playbooks as hostile bounded text

Only fixed roots and normalized Markdown files are indexed. Every component is inspected without following links, content is size/count/depth bounded, and the UI uses preformatted text rather than a renderer.

Alternative: render Markdown/HTML. Rejected because it adds an unnecessary dependency and execution/sanitization surface.

### Apply security presets through the existing settings transaction

Preset classification and apply cover paranoid mode, all six mutation flags, and client overrides. Apply locks, reloads, changes the complete matrix, persists once, then refreshes cache while preserving unrelated settings and the allowlist.

Alternative: invoke existing individual toggles. Rejected because partial writes could display a named posture that is not actually enforced.

### Make artifact shape part of lifecycle authority

Kimi/OpenClaw use a complete artifact manifest on the existing install record. Aider/Windsurf use separate bounded project roster records with exact ordered Agent references. Generic per-Agent flows either route to the correct aggregate planner or block before apply. Project inventory/removal includes roster rows.

Alternative: represent each aggregate target as one Agent install. Rejected because secondary files and roster membership would drift outside ledger truth.

### Preserve Antigravity and prohibit OpenClaw execution

Antigravity uses its existing renderer and lifecycle with added parity evidence only. OpenClaw has file lifecycle only and is excluded at every executable-probe boundary, including generic version detection.

Alternative: invoke OpenClaw for registration/version. Rejected because the approved boundary allows no OpenClaw CLI execution and gateway lifecycle is externally owned.

## Risks / Trade-offs

- [A cross-domain document can grow] → Enforce byte, collection, item, text, and path caps before persistence and prune feed/recommendation history deterministically.
- [A failed evidence source can make readiness conservative] → Keep successful evidence visible and label the result Unavailable rather than overclaiming.
- [Generic flows may not have enough roster context] → Block early with an explicit reason and a handoff to multi-Agent roster review.
- [Multi-artifact rollback can fail] → Preserve the prepared journal and exact prior bytes, surface recovery honestly, and never commit ledger success early.
- [A project could be removed while aggregate artifacts remain] → Include roster truth in inventory and require lifecycle completion or retained registration before forgetting.
- [Responsive CSS can compress dense controls] → Wrap/stack controls at narrow widths and preserve visible focus and document access rather than hiding overflow.

## Migration Plan

1. Register and validate the control-center document with backward-compatible defaults; an absent document means no baseline, subscription, feed, or recommendation state.
2. Add read-only feed, readiness, Review, Recovery, and Playbook projections before enabling their explicit actions.
3. Add atomic settings presets through the existing settings lock and document.
4. Extend install records for multi-artifact manifests and add a separate roster ledger migration that preserves existing single-file rows.
5. Reconcile current disk truth before exposing new lifecycle actions; never adopt foreign aggregate files automatically.
6. Verify crash recovery, exact rollback, 375px reflow, keyboard focus, strict dependency/security gates, and live upstream renderer parity.
7. Roll back by hiding new surfaces and reverting commands while retaining readable versioned state and journals; do not delete user artifacts or force-downgrade persisted documents.
