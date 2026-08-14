## 1. Portable Contract and Legacy Boundary

- [x] 1.1 Add failing Rust tests for bounded Workspace Pack v1 parsing, exact reference validation, deterministic ordering, one-scope export, absolute-path/credential exclusion, unsupported versions, and count/string limits.
- [x] 1.2 Extend the existing Agentfile/loadout types and export command to emit deterministic exact-reference Workspace Pack v1 JSON for either global scope or one canonical registered project, including both managed Agent and Skill rows.
- [x] 1.3 Add failing compatibility tests and implement read-only Agentfile v1 conversion that blocks malformed, unknown, or ambiguous slugs instead of silently skipping them.

## 2. Complete Read-Only Planning

- [x] 2.1 Add failing Rust tests requiring one complete no-write plan with exact Agent/Skill destinations, dependencies, current/no-op states, warnings, blockers, rollback scope, declarative requirements, and deterministic revision.
- [x] 2.2 Reuse existing source inspection, registered-project truth, tool registry, ledgers, and Agent/Skill plan functions to build the pack plan and require explicit project binding for project scope.
- [x] 2.3 Block every unsafe or unresolved state before approval, keep instruction/MCP requirements explicitly passive, and prove planning leaves files, ledgers, database state, network, and external configuration unchanged.

## 3. Revision-Bound Recoverable Apply

- [x] 3.1 Add failing Rust tests for fresh-preflight drift causing zero writes, current entries remaining byte-identical, successful cross-domain apply, reverse rollback after a later failure, rollback-error reporting, and idempotent crash recovery.
- [x] 3.2 Implement revision-bound apply through existing exact Agent and Skill lifecycle authorities, recording only newly created identities in the existing filesystem-operation journal.
- [x] 3.3 Reconcile both ledgers after success or rollback, return exact terminal results, and preserve all pre-existing managed or unmanaged content.

## 4. Existing Teams Review Surface

- [x] 4.1 Add failing frontend tests for scope selection, file selection, project binding, complete plan rendering, passive instruction/MCP labels, blockers, explicit approval, progress, retained results, and focus/announcement behavior.
- [x] 4.2 Extend existing frontend types/store methods and the Teams file controls into the inspect-review-apply-results state machine without adding a route or standalone pack component.
- [x] 4.3 Replace immediate legacy restore with read-only review, disable approval while truth is stale or blocked, and refresh both Agent and Skill installation stores after terminal apply.

## 5. Exact Completion Evidence

- [x] 5.1 Add failing frontend tests for one mixed Agent/Skill Workspace Pack receipt, exact attempted destinations, aggregate outcome, no-op result retention, redaction, and exact `View Activity` navigation.
- [x] 5.2 Reuse the Phase 12 Activity receipt boundary and existing navigation action for successful or rolled-back pack completion without retrying mutation on journal persistence failure.

## 6. Verification and Integration

- [x] 6.1 Run focused/full frontend tests, Svelte diagnostics, production build, Rust formatting, strict Clippy, focused/full backend tests, crash-recovery tests, and diff checks; fix only Phase 13 regressions.
- [x] 6.2 Run strict OpenSpec validation and audit for deterministic/path-private export, no dependency, network, telemetry, notification, instruction write, MCP configuration/install, new route, runtime execution, or unrelated mutation authority.
- [ ] 6.3 Sync and archive the OpenSpec change, update existing Phase records and the canonical feature roadmap, commit the branch, merge locally while preserving user-owned main changes, and re-run integration smoke gates.
