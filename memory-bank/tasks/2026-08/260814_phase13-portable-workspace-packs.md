# 260814_phase13-portable-workspace-packs

## Objective

Provide one deterministic, path-private Workspace Pack format that can export, inspect, review, and recoverably apply exact Agent and Skill selections without executing passive instructions or MCP requirements.

## Outcome

- Extended the existing Agentfile/loadout boundary with bounded Workspace Pack v1 JSON covering exact source-aware Agents, Skills, one logical scope, runbook text, tool targets, instructions, and passive MCP requirements.
- Added strict read-only inspection with explicit project binding, exact destinations, dependencies, current/no-op states, warnings, blockers, rollback scope, and revision-bound approval.
- Reused the existing Agent and Skill lifecycle authorities plus filesystem journal for cross-domain apply, reverse rollback, crash recovery, and final dual-ledger reconciliation.
- Extended the existing Teams surface with export, import, complete-plan review, approval, progress, retained outcomes, accessible focus restoration, and exact Activity navigation.
- Reused Phase 12 receipts for one redacted mixed Agent/Skill completion record, including current/no-op items and exact known destinations.

## Verification

- OpenSpec: 17/17 tasks complete; strict change validation passed; canonical specs validate 7/7 after sync.
- Frontend: 105/105 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend: 550 tests discovered; 547 library tests passed with 3 existing environment-gated ignores; 2/2 binary tests passed; focused Workspace Pack, legacy conversion, and crash-recovery tests passed.
- Rust quality: formatting and strict Clippy passed.
- Safety: path privacy, credential rejection, deterministic serialization, revision drift, rollback, recovery, redaction, no-network, no-execution, no-instruction-write, no-MCP-mutation, dependency, route, and diff audits passed.

## Integration Points

- `src-tauri/src/install/mod.rs` owns the bounded portable contract, legacy conversion, inspect plan, revision-bound apply, rollback, and parent-journal recovery.
- `src-tauri/src/state.rs` and `src-tauri/src/state_db.rs` connect Workspace Pack parent recovery to the existing startup journal authority.
- `src/lib/stores/install.svelte.ts` bridges export, inspection, apply, dual-store refresh, and the mixed Activity receipt.
- `src/lib/components/Teams.svelte` hosts the existing-surface export and inspect-review-apply-results flow.
- `openspec/specs/workspace-packs/spec.md` is the canonical capability contract.

## Security and Safety

- Import inspection is bounded, local, deterministic, and write-free; unknown fields, malformed references, oversized input, obvious credentials, unsafe paths, ambiguity, stale revisions, and unsupported targets block apply.
- Project packs require an explicit canonical registered-project binding and never serialize the source machine's absolute project path.
- Instructions, runbooks, tool targets, and MCP requirements remain declarative; the app does not execute them or mutate MCP configuration.
- Apply changes only missing managed items through existing lifecycle authorities and preserves current, foreign, modified, and otherwise unsafe content.

## Artifacts

- Implementation commit: `2e86fb8`
- Branch: `feat/phase-13-workspace-packs`
- OpenSpec archive: `openspec/changes/archive/2026-08-14-phase-13-portable-workspace-packs/`
