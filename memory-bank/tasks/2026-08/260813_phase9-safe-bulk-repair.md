# 260813_phase9-safe-bulk-repair

## Objective

Provide one reviewed repair workflow for recoverable Agent and Skill installations without automatically mutating divergent, disabled, foreign, or source-unavailable content.

## Outcome

- Combined tracked outdated and missing Agent and Skill installations into the existing repair entry points.
- Kept modified, foreign, disabled, and source-unavailable installations visible but non-selectable with state-specific guidance.
- Added a complete read-only review using existing Agent update plans, Skill install plans, exact destinations, blockers, warnings, package details, source hashes, backups, and Agent differences.
- Bound approval to a fresh dual-ledger reconciliation and normalized preflight signature; changed identity, state, plan, or Skill source bytes invalidate approval before mutation.
- Reused existing exact-reference Agent and Skill lifecycle operations, continuing after individual failures and retaining per-item outcomes plus a bounded aggregate summary.
- Reconciled both ledgers after execution and displayed final recorded installation truth.

## Verification

- OpenSpec: 16/16 tasks complete; strict change validation passed; canonical specs validate 3/3 after sync and archive.
- Frontend: 84/84 tests passed.
- Svelte: 0 errors and 0 warnings.
- Build: production frontend build passed.
- Backend: 527 library tests plus 2 binary tests passed; 3 existing environment-gated tests ignored.
- `git diff --check`: passed before integration.

## Integration Points

- `src/lib/components/UpdatesModal.svelte` owns selection, review, approval-bound preflight, sequential execution, and results.
- `src/lib/components/DivisionsLanding.svelte` and `src/lib/components/Sidebar.svelte` expose the combined eligible repair count.
- Existing Agent and Skill stores remain the lifecycle, reconciliation, backup, and Activity authorities.
- `openspec/specs/safe-bulk-repair/spec.md` is the canonical capability contract.

## Security and Safety

- Only exact tracked outdated or missing references are selectable.
- Fresh reconciliation failure or reviewed-plan drift causes zero writes.
- Existing transactional backup and rollback paths perform every mutation.
- No backend command, dependency, persistence format, or network behavior changed.

## Artifacts

- Implementation commit: `e36ec10`
- Branch: `feat/phase-9-safe-bulk-repair`
- OpenSpec archive: `openspec/changes/archive/2026-08-13-phase-9-safe-bulk-repair/`
