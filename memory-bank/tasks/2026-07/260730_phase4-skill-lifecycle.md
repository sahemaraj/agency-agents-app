# 260730_phase4-skill-lifecycle

## Objective

Complete the managed skill lifecycle so users can update, disable, re-enable, and safely uninstall
tracked skills without overwriting foreign destinations or losing modified content.

## Outcome

- ✅ Added Update for Outdated installs and repair for Missing installs.
- ✅ Added reversible Disable and Enable using exact same-filesystem directory moves.
- ✅ Added confirmed Uninstall with automatic backup of modified content.
- ✅ Preserved installed content and ledger records when a source is removed.
- ✅ Kept SourceUnavailable installs visible and manageable outside the removed source.
- ✅ Added recoverable backup visibility in the Skills workspace.
- ✅ Rejected linked directory roots, linked package entries, and occupied enable destinations.
- ✅ Tests: 305 passed, 0 failed, 1 environment-gated parity test ignored.
- ✅ Svelte check: 0 errors, 1 pre-existing missing `@types/node` warning.
- ✅ Production build and whitespace audit succeeded.

## Files Modified

- `src-tauri/src/skills/install.rs` — lifecycle directory transactions and regression tests.
- `src-tauri/src/skills/mod.rs` — update, disable, enable, uninstall, and backup-list commands.
- `src-tauri/src/lib.rs` — lifecycle command registration.
- `src/lib/api.ts` — typed lifecycle command wrappers.
- `src/lib/stores/skillSources.svelte.ts` — lifecycle busy/error/reconciliation state.
- `src/lib/components/SkillsWorkspace.svelte` — state-specific actions, confirmation, unavailable
  source management, and backup visibility.

## Patterns Applied

- Extended the Phase 3 ledger and transaction implementation; no parallel lifecycle subsystem.
- Serialized every ledger mutation through the existing skill-install write lock.
- Re-hashed destinations immediately before mutation and refused unsafe linked roots.
- Kept foreign content immutable and backed up modified tracked content before removal.
- Rolled ledger state back when a filesystem lifecycle operation failed.

## Integration Points

- `skill_update` reuses the validated Phase 3 install transaction.
- `skill_disable` and `skill_enable` update `disabledPath` in the dedicated skill ledger.
- `skill_uninstall` removes only a matching tracked record and its exact directory.
- Source removal remains registration-only; reconciliation reports affected installs as
  SourceUnavailable.
- The existing Skills destination UI surfaces lifecycle actions according to reconciled state.

## Scope Boundaries

Phase 4 does not adopt foreign skills, schedule automatic updates, perform bulk lifecycle actions,
or delete backups.
