# 260730_phase3-transactional-skill-installation

## Objective

Install exact validated multi-file skill packages into Claude Code and Codex user or project
destinations without overwriting foreign or locally modified content.

## Outcome

- ✅ Added Claude Code and Codex installation at user and project scope.
- ✅ Added a dedicated atomic `skill-installs.json` ledger.
- ✅ Added deterministic directory hashing and reconciliation for Current, Outdated, Modified,
  Missing, Foreign, Disabled, and SourceUnavailable.
- ✅ Added staging, backup-first managed replacement, atomic publication, and rollback.
- ✅ Rejected linked package entries, linked destination ancestors, invalid names, and non-real roots.
- ✅ Reused one destination grid across agent and skill deployment.
- ✅ Kept update, disable, enable, uninstall, and source lifecycle actions deferred to Phase 4.
- ✅ Tests: 301 passed, 0 failed, 1 environment-gated parity test ignored.
- ✅ Svelte check: 0 errors, 1 pre-existing missing `@types/node` warning.
- ✅ Production build and whitespace audit succeeded.

## Files Modified

- `src-tauri/src/skills/install.rs` — ledger, hashing, target resolution, transaction, and tests.
- `src-tauri/src/skills/mod.rs` — validated install and reconciliation commands.
- `src-tauri/src/types.rs` — installation ledger, state, and IPC DTOs.
- `src-tauri/src/state.rs` — serialized skill-ledger write lock.
- `src-tauri/src/lib.rs` — command registration.
- `src/lib/api.ts` — typed installation and reconciliation wrappers.
- `src/lib/types.ts` — mirrored installation DTOs.
- `src/lib/stores/skillSources.svelte.ts` — installed state, busy state, errors, and reconciliation.
- `src/lib/components/DeploymentTargetGrid.svelte` — shared deployment target grid.
- `src/lib/components/InstallModal.svelte` — reuses the shared grid.
- `src/lib/components/SkillsWorkspace.svelte` — safe skill installation controls and states.

## Patterns Applied

- Extended Phase 1 validation and Phase 2 package discovery; no parallel parser or validator.
- Used the existing atomic-write helper for the dedicated skill ledger.
- Kept all filesystem trust-boundary checks and mutations in Rust.
- Protected unknown and changed content by default; only verified managed content is replaceable.
- Published staged directories with a same-parent rename and restored prior ledger/files on failure.

## Integration Points

- `skill_install` re-discovers and revalidates the selected package immediately before installation.
- `skill_installs_reconcile` compares source, ledger, and exact destination directory hashes.
- The Skills workspace uses the existing project registry for destination rows.
- Agent deployment and Skills deployment share `DeploymentTargetGrid.svelte`.

## Scope Boundaries

Phase 3 installs and reports lifecycle states. Update, disable, enable, uninstall, source removal
effects, and backup-management UI remain Phase 4.
