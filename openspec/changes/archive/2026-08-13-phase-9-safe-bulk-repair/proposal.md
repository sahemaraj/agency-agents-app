## Why

Agency Agents already identifies outdated and missing managed Agent and Skill installs, but users must repair them one at a time and the existing bulk update modal covers only outdated Agents without a complete preflight review. Phase 9 turns reconciled truth into one safe, explicitly approved repair workflow while keeping divergent content out of automation.

## What Changes

- Extend the existing updates modal into a combined Agent and Skill repair review for tracked installs classified as outdated or missing.
- Show modified, foreign, disabled, and source-unavailable installs separately with a specific manual-review reason; never select or mutate them automatically.
- Build and display every selected item's existing mutation plan, exact destination, warnings, blockers, and Agent diff where relevant before enabling approval.
- Apply the reviewed set only after one explicit approval, using the existing exact Agent and Skill update paths, backups, reconciliation, and Activity logging.
- Continue after an individual repair failure and report exact per-item success or failure plus a bounded summary.
- Update the existing dashboard/sidebar entry point and locale catalog so repair availability reflects both Agent and Skill ledgers without adding a new navigation section.

## Capabilities

### New Capabilities

- `safe-bulk-repair`: Selection, review, explicit approval, execution, and per-item reporting for safely repairable Agent and Skill installs.

### Modified Capabilities

None.

## Impact

- Extends `src/lib/components/UpdatesModal.svelte` and its existing counts in `src/lib/components/DivisionsLanding.svelte` and `src/lib/components/Sidebar.svelte` rather than adding a parallel repair surface.
- Reuses exact-reference Agent plan, diff, update, backup, reconciliation, and Activity paths from `src/lib/stores/install.svelte.ts` and `src/lib/api.ts`.
- Reuses exact Skill install-plan, update, backup, reconciliation, and Activity paths from `src/lib/stores/skillSources.svelte.ts` and `src/lib/api.ts`.
- Adds only the deterministic view-model logic and focused frontend tests needed by that surface; no backend command, persistence format, network call, dependency, or automatic divergent-content mutation is introduced.
