# 260804_agent-lifecycle-parity

## Objective

Bring the source-aware Agent installation lifecycle to Skills-equivalent safety and control while
preserving exact Agent identity across sources, nested paths, tools, scopes, and destinations.

## Outcome

- ✅ Migrated legacy Agent install rows to exact source-aware identities without changing installed
  destination content.
- ✅ Reconciled Current, Outdated, Modified, Missing, Foreign, Disabled, and SourceUnavailable
  states from the ledger, validated source package, and destination bytes.
- ✅ Added transactional install/update, backup-first uninstall, disable/enable, bounded version
  history, and exact rollback.
- ✅ Added dependency and collection planning with blockers, warnings, capabilities, destinations,
  and rollback details before execution.
- ✅ Added desktop lifecycle review and actions to the shared install modal, including all seven
  states, exact provenance, update policy, collections, and version history.
- ✅ Rust: 434 passed, 0 failed, 2 environment-dependent tests ignored; 2 CLI tests passed.
- ✅ A copied real pre-feature ledger migrated successfully; both real installed destination hashes
  remained unchanged.
- ✅ Strict Clippy, Rust formatting, Svelte check, 13 frontend tests, production build, and diff
  hygiene passed.

## Files Modified

- `src-tauri/src/install/mod.rs` — source-aware migration, reconciliation, lifecycle transactions,
  exact mutation plans, dependencies, collections, and Tauri commands.
- `src-tauri/src/install/history.rs` — bounded Agent version snapshots and rollback support.
- `src-tauri/src/agents/mod.rs` — exact Agent package resolution used by lifecycle operations.
- `src-tauri/src/{types.rs,lib.rs}` — lifecycle DTOs and command registration.
- `src/lib/components/InstallModal.svelte` — shared plan review, policy, history, and lifecycle
  actions.
- `src/lib/components/{AgentsWorkspace,DeploymentMatrix,DiffModal,AgentLibrarySidebar}.svelte` —
  exact state consumers and collection entry points.
- `src/lib/stores/install.svelte.ts` — exact lifecycle state and action orchestration.
- `src/lib/agents/libraryModel.ts` — collision-safe frontend install identity.
- `src/lib/agents/libraryModel.test.ts` — source/path identity, state, policy, and blocker coverage.
- `src/lib/{api.ts,types.ts}` and locale files — exact desktop contracts and lifecycle copy.

## Patterns Applied

- Extended the existing ledger reconciliation and transactional installation model rather than
  creating a second Agent state store.
- Kept `(sourceId, relativePath)` through every lifecycle plan and mutation boundary.
- Reused the existing install modal for Agent plan review and lifecycle actions.
- Kept source removal non-destructive: installed content and ledger provenance survive as
  SourceUnavailable.

## Integration Points

- Source validation resolves through the Stage 1 Agent registry before installation or update.
- Destination writes reuse existing renderer/tool-registry and capability-relative path rules.
- History snapshots are created at the transaction boundary and rollback targets one exact
  source-aware install record.
- Frontend state keys include source, relative path, tool, scope, project, and destination so
  same-name Agents cannot collide.

## Scope Boundaries

Agents MCP, final integration, commits, pushes, PRs, and releases were not part of Stage 2.

## Artifacts

- Design: `docs/superpowers/specs/2026-08-04-agents-skills-feature-parity-design.md`
- Plan: `docs/superpowers/plans/2026-08-04-agent-lifecycle-parity.md`
