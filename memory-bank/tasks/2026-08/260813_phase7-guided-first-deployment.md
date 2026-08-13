# 260813_phase7-guided-first-deployment

## Objective

Guide a new user from catalog selection to a safe, reconciliation-verified first Claude Code or Codex preset-team deployment in under 60 seconds.

## Outcome

- Continued the existing catalog first-run screen into deployment preparation, plan review, transactional apply, and verified success or defer.
- Preferred AI Builders only when complete, otherwise selected the first complete bundled preset in declaration order; partial and absent presets block approval.
- Displayed truthful Claude Code/Codex detection, reused the existing user/project destination grid, and exposed every Agent, dependency, destination, warning, blocker, and rollback status before Apply.
- Added bounded exact-reference batch plan/apply commands. Apply requires explicit confirmation, rebuilds the plan, rejects stale revisions, and reuses the existing file-snapshot and ledger rollback transaction.
- Reported success only when fresh reconciliation classifies every planned exact reference as current at the approved target and scope, then showed exact destinations and the existing preset starter prompt.

## Verification

- OpenSpec: strict change validation passed before archive; canonical spec validation passed after archive.
- Frontend: 69/69 tests passed; Svelte check reported 0 errors and 0 warnings; production build passed.
- Backend: 527 library tests plus 2 binary tests passed; 3 existing environment-gated tests ignored.
- Rust format and `git diff --check`: passed.
- Transaction regressions prove dependency expansion, warning/blocker visibility, no-write planning, and restoration of captured files plus the prior ledger.

## Integration Points

- `src/lib/components/CatalogFirstRun.svelte` extends the existing foreground first-run surface.
- `src/lib/components/InstallModal.svelte` remains the single deployment-plan and scope UI.
- `src-tauri/src/install/mod.rs` reuses `build_mutation_plan`, `require_plan_revision`, and `execute_install_plan`.
- `openspec/specs/guided-first-deployment/spec.md` is the canonical capability contract.

## Accepted Evidence Limits

The user approved the existing manual-platform waiver. Real 375px timing and native Linux/Windows visual evidence remain UNAVAILABLE rather than passed.

## Artifacts

- Implementation commit: `7b1c19b`
- Branch: `feat/phase-7-guided-first-deployment`
- OpenSpec archive: `openspec/changes/archive/2026-08-13-phase-7-guided-first-deployment/`
