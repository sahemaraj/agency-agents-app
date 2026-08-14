# 260814_phase17-expert-improvement-coach

## Objective

Turn existing local Expert quality evidence into useful performance signals only after enough directly comparable terminal runs exist, without adding telemetry, model inference, or mutation authority.

## Outcome

- Added an exact cohort selector for the same Expert id, Expert version, contract version, and ordered quality-check definitions.
- Excluded in-progress, awaiting-review, and cancelled runs because they do not contain a terminal human quality verdict.
- Required five comparable accepted, rework, or rejected runs before exposing any rate or suggestion.
- Added local acceptance, rework/rejection, waiver, and per-check latest-evidence summaries.
- Added deterministic suggestions only for signals present in at least two runs and 40% of the eligible cohort.
- Extended the existing Expert detail surface with below-threshold disclosure and eligible performance guidance.

## Verification

- OpenSpec: 7/7 tasks complete; strict change validation passed; canonical specs validate 11/11 after sync.
- Frontend: 113/113 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend regression: 570 library tests discovered; 567 passed with 3 existing environment-gated ignores; 2/2 binary tests passed.
- Rust quality: formatting and strict Clippy passed.
- Dependency audit: production npm dependency audit reported 0 vulnerabilities.
- Safety: exact cohort identity, non-verdict exclusion, five-run gating, latest-evidence semantics, bounded deterministic copy, local-only derivation, no-network, no-model, no-telemetry, no-mutation, no-persistence, and diff audits passed.

## Integration Points

- `src/lib/stores/experts.svelte.ts` derives the bounded summary from the existing loaded run list.
- `src/lib/components/Experts.svelte` renders the threshold and eligible signals on the existing Expert detail surface.
- `src/lib/smoke.test.ts` verifies selection, gating, latest-evidence behavior, rates, recurring signals, and UI disclosure.
- `openspec/specs/expert-improvement-coach/spec.md` is the canonical capability contract.

## Security and Safety

- All analysis remains in the renderer over already-loaded local records; no evidence or waiver reason leaves the device.
- The helper cannot approve a run, edit an Expert, install content, or call any backend command.
- Suggestions describe observed frequencies and do not claim a root cause.

## Artifacts

- Implementation commit: `64fe1f8`
- Branch: `feat/phase-17-improvement-coach`
- OpenSpec archive: `openspec/changes/archive/2026-08-14-phase-17-expert-improvement-coach/`
