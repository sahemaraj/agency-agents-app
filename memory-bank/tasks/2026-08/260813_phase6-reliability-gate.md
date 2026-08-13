# 260813_phase6-reliability-gate

## Objective

Make application failures semantic, keep installation truth usable when reconciliation fails, and restrict filesystem reveal to existing canonical paths inside supported roots.

## Outcome

- Replaced affected object-coercion error surfaces with semantic `AppErrorPayload` rendering while preserving the three approved native string fallbacks.
- Agent and Skill reconciliation now retains the last successful rows, exposes explicit fresh/reconciling/stale state, coalesces Retry, and blocks stale mutations without clearing armed confirmations.
- Filesystem reveal now derives allowed roots in Rust, canonicalizes existing targets, rejects URL-like, relative, missing, unrelated, prefix-sibling, and symlink-escape paths, uses shell-free platform arguments, and reports nonzero opener exits.
- Extended the existing smoke and Rust test surfaces; no new dependency or generalized abstraction was added.

## Verification

- Frontend: 65/65 tests passed.
- Backend: 526 Rust library plus 2 binary tests passed; 3 intentional/manual tests ignored.
- Svelte check: 0 errors and 0 warnings.
- Production build, Rust formatting, and `git diff --check`: passed.
- Nyquist: compliant, audit result PASS.
- Security: zero blocking threats; low T-06-07 and T-06-12 explicitly accepted.
- Independent goal verification: REL-01, REL-02, and REL-03 satisfied with zero implementation blockers.

## Files Modified

- `src/lib/types.ts` consumers, stores, and affected Svelte components — semantic failure display and truthful reconciliation state.
- `src-tauri/src/install/mod.rs` and `src-tauri/src/state.rs` — state-derived reveal roots, canonical containment, and checked native opener execution.
- `src/lib/smoke.test.ts` — executable semantic, reconciliation, stale-mutation, Retry, and inventory coverage.

## Integration Points

- Existing `appErrorMessage()` remains the single semantic renderer.
- Existing Agent and Skill singleton stores remain the installation-truth authorities.
- Existing Tauri `reveal_path` ABI is unchanged; authorization moved behind the Rust command boundary.

## Accepted Evidence Limits

The user approved a manual-platform waiver on 2026-08-13. Real 375px browser geometry and native Linux/Windows builds remain UNAVAILABLE rather than passed. macOS is tied to the exact verified content digest recorded in the Phase 6 evidence matrix.

## Artifacts

- Implementation commit: `73e7c5d`
- Branch: `feat/v2-activation-truthful-state`
- GSD phase: `.planning/phases/SKILLS-06-reliability-gate/`
