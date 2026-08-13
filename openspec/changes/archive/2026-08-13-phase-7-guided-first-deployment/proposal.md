## Why

After choosing a catalog, a new user currently lands in the general workspace without a clear path to a first useful deployment. Phase 7 closes that activation gap while preserving the app's explicit-review, transactional-write, and reconciliation-backed truth guarantees.

## What Changes

- Continue first-run directly from catalog selection into a guided deployment flow.
- Show actual Claude Code and Codex detection state and one deterministic compatible preset-team recommendation.
- Let the user choose user or registered-project scope, review exact destinations, dependencies, warnings, and blockers, and explicitly approve before any write.
- Route the recommended preset through the existing mutation-plan and transactional batch installation engine using exact Agent references.
- Report success only after reconciliation confirms the installed destinations, then provide a suitable reusable starter prompt.
- Offer an honest blocked state when neither supported first-run target is detected and a secondary option to finish later.

## Capabilities

### New Capabilities

- `guided-first-deployment`: Catalog-to-deployment guidance, deterministic preset recommendation, reviewed target selection, transactional approval, and reconciliation-backed completion.

### Modified Capabilities

None. This repository does not yet contain OpenSpec baseline capabilities.

## Impact

- Frontend: first-run orchestration, guided deployment UI, preset recommendation, localized copy, and focused tests.
- Backend/API: a minimal exact-reference batch plan/apply command that reuses the current Agent mutation-plan and rollback transaction internals.
- Existing systems reused unchanged: catalog selection, Claude Code/Codex detection, project registration, presets, transaction rollback, install ledger reconciliation, Activity logging, and starter prompts.
- No breaking API changes, production dependencies, telemetry, accounts, or automatic writes.
