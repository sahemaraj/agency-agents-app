# 260813_phase8-foreground-reconciliation

## Objective

Refresh Agent and Skill installation truth when the app returns to the foreground without duplicate scans, network access, mutation, or loss of usable state after failure.

## Outcome

- Added one root-owned 250 ms window-focus debounce for both installation ledgers.
- Reused the Agent single-flight promise and Skill scope-aware queue so mount and focus requests share existing work.
- Loaded registered project paths at startup and reconciled Skills again only when the canonical scope changed.
- Limited focus work to local installation and backup reads; catalog, Skill source, network, ledger, and managed destination mutations are excluded.
- Preserved the existing stale-data, actionable-error, and Retry contracts independently for Agent and Skill failures.
- Removed the focus listener and pending timer during root cleanup.

## Verification

- OpenSpec: 10/10 tasks complete; strict change validation passed; canonical spec validation passed after archive.
- Frontend: 74/74 tests passed; five focused foreground lifecycle tests cover debounce, overlap, cleanup, read-only commands, retained failure state, and recovery.
- Svelte: 0 errors and 0 warnings.
- Build: production frontend build passed.
- Backend: 527 library tests plus 2 binary tests passed; 3 existing environment-gated tests ignored.
- `git diff --check`: passed.

## Integration Points

- `src/routes/+layout.svelte` owns startup and foreground reconciliation.
- `src/lib/stores/install.svelte.ts` remains the Agent single-flight and stale-state authority.
- `src/lib/stores/skillSources.svelte.ts` remains the Skill scope queue and stale-state authority.
- `openspec/specs/foreground-reconciliation/spec.md` is the canonical capability contract.

## Accepted Evidence Limits

The existing manual-platform waiver remains in force. Native focus behavior is covered through the browser lifecycle integration test; no separate Windows/Linux GUI claim is made.

## Artifacts

- Implementation commit: `af9aa2b`
- Branch: `feat/phase-8-foreground-reconciliation`
- OpenSpec archive: `openspec/changes/archive/2026-08-13-phase-8-foreground-reconciliation/`
