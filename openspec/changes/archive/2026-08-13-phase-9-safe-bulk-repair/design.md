## Context

See `proposal.md` for motivation and `specs/safe-bulk-repair/spec.md` for the behavior contract. The current `UpdatesModal.svelte` owns the bulk-update selection and modal focus flow, but it only derives outdated Agents and immediately calls the legacy slug-based bulk method (`src/lib/components/UpdatesModal.svelte:24-97`). Exact Agent plan, diff, mutation, reconciliation, and Activity paths already exist in the install store (`src/lib/stores/install.svelte.ts:262-359`). Skills expose the same reconciled identity and exact update lifecycle (`src/lib/stores/skillSources.svelte.ts:534-616`), while the existing Skill install plan reports destinations, packages, warnings, blockers, and rollback availability (`src/lib/types.ts:556-574`).

The implementation must preserve local modifications, avoid a second repair surface, and make no backend or persistence changes.

## Goals / Non-Goals

**Goals:**

- Extend the existing modal into a combined select, review, apply, and results flow.
- Reuse exact Agent and Skill APIs so existing backups, transactions, reconciliation, and Activity records remain authoritative.
- Bind approval to the exact reviewed preflight and stop before mutation if truth changes.
- Keep candidate classification and plan comparison deterministic and directly testable.

**Non-Goals:**

- Automatically resolving modified, foreign, disabled, or source-unavailable content.
- Adding scheduling, background auto-repair, a new navigation section, or a new backend batch command.
- Adding Skill text diffs; modified Skills are excluded, and their existing package plan is the relevant review artifact for safe states.

## Decisions

### Extend the existing modal and stores

`UpdatesModal.svelte` will retain modal ownership and become a four-state flow: selection, review, applying, and results. It will derive candidates from `install.installed` and `skillSources.installed`; `DivisionsLanding.svelte` and `Sidebar.svelte` will reuse the same simple eligibility predicate for their combined count.

Alternative considered: add a new repair route and orchestration store. Rejected because the existing modal is already the discoverable update action, and no other caller needs persistent repair workflow state.

### Identify each destination exactly

The UI will key Agent candidates by artifact type, source ID, relative path, tool, and project path. Skill candidates use artifact type, source ID, relative path, runtime, and project path. Only tracked rows whose state is outdated or missing are eligible; the four unsafe states are rendered separately with localized reasons.

Alternative considered: retain slug-and-tool grouping from the current modal. Rejected because it collapses project destinations and loses the exact source identity required by the lifecycle APIs.

### Aggregate existing per-item plans

Review construction uses the existing Agent update plan for both outdated and missing managed Agents. Missing is labeled “reinstall” in the UI, but update is the correct lifecycle operation because the ledger identity already exists and the update path restores its destination. Skills use the existing Skill install plan to preview their exact package/destination graph and the existing Skill update lifecycle to execute both states. Agent differences remain available through the existing diff modal.

All selected plans are fetched read-only. A rejected plan request or any blocker is shown inline and disables approval. No aggregate backend plan is added because the existing plans already expose the required facts and the modal is the only cross-artifact coordinator.

### Invalidate changed approvals before the first write

The review stores a normalized signature of selected exact identities and their returned plans. Approval first reconciles both ledgers, rebuilds every selected plan, and compares normalized signatures. Any eligibility or plan change returns to review without invoking a mutation. Agent revisions participate through the existing plan data; Skill plans are compared by their stable returned fields.

Alternative considered: execute immediately from the earlier review. Rejected because a filesystem or source change could turn a safe candidate into divergent content after approval.

### Execute sequentially and preserve exact outcomes

After a matching fresh preflight, the modal invokes `install.updateReference(...)` or `skillSources.lifecycle("update", ...)` one item at a time and catches each result independently. This deliberately reuses each store’s per-item Activity logging and reconciliation. The modal retains a terminal result for every approved item and writes one aggregate Activity entry with only success and failure counts. Both ledgers receive a final reconciliation before results are presented.

Alternative considered: parallel execution. Rejected because current stores have shared busy/reconciliation coordination, and sequential work produces deterministic results with the smallest change.

### Keep logic colocated unless tests prove extraction necessary

Candidate classification, exact keys, and stable plan normalization begin as pure functions in `UpdatesModal.svelte`; focused source-contract and store tests extend the existing `src/lib/smoke.test.ts` and nearby tests. A separate helper file is justified only if the component cannot expose deterministic logic to the existing test setup without coupling tests to markup.

Alternative considered: create a repair domain module immediately. Rejected as a one-consumer abstraction.

## Risks / Trade-offs

- Per-item lifecycle reconciliation makes large repairs slower → keep execution sequential and surface current progress; optimize only if measured.
- A Skill plan is an install-shaped preflight while execution uses update → compare the exact returned destinations/packages and retain the reconciled managed identity; test both missing and outdated cases.
- Source or disk truth can change again after fresh preflight → existing transactional writes and backups remain the last-resort protection, while the immediately preceding preflight minimizes the race.
- Localized copy expands the existing catalog → use the established `agentUpdates.*` namespace and verify every locale has matching keys.

## Migration Plan

1. Replace the current modal behavior in place and update the two existing counts.
2. Run focused frontend tests, Svelte diagnostics, the frontend suite, production build, Rust suite, and strict OpenSpec validation.
3. Roll back by reverting the frontend changes; backend commands, ledgers, and persisted formats remain unchanged.
