# 260806_sqlite-control-plane

## Objective

Replace mutable JSON control-plane state with a transactional SQLite authority shared by the
desktop app and MCP processes, without moving package/source artifacts into the database.

## Outcome

- Added a bundled SQLite state store for 17 versioned documents, visible revisions, process
  leases, verified migration backups, integrity checks, and hash-bound legacy conflict warnings.
- Moved settings, catalog configuration, Skills, Agents, Experts, projects, installs, and MCP audit
  state behind the shared store after cutover; legacy JSON remains authoritative before cutover.
- Added recoverable filesystem journals for publish, install, move, uninstall, and Expert
  activation operations, including exact approval revision reconciliation.
- Added an accessible one-time maintenance gate and foreground live refresh without closing open
  approval panels, resetting scroll, stealing focus, or showing transient refresh notifications.
- Kept signed Skill trust keys in macOS Keychain and cryptographically verifies records on import.

## Verification

- Frontend: 25 tests passed; Svelte check reported 0 errors and warnings; production build passed.
- Backend: 521 tests passed; 3 intentional/manual tests ignored; strict Clippy and format passed.
- Copied-state rehearsal: 17/17 documents imported, restored backup integrity `ok`, and all 2,311
  package/artifact files retained the same aggregate hash.
- Permissions: database, WAL, SHM, lock, and backup files `0600`; backup directories `0700`.
- MCP inventory unchanged: 80 Skills/Experts tools and 52 Agent tools. Seven migration-only Tauri
  commands were added and registered.
- Live application data was not modified during rehearsal.

## Files Modified

- `src-tauri/src/state_db.rs` — transactional storage, migration, lease, backup, conflict, and
  filesystem-journal primitives.
- `src-tauri/src/state.rs` — 17-document registry, migration commands, startup recovery, and audit
  migration.
- `src-tauri/src/{skills,agents,install,experts}.rs` and submodules — SQLite-backed selectors,
  mutations, recovery, and exact approval reconciliation.
- `src/lib/components/StorageMigrationGate.svelte` and `src/routes/+page.svelte` — maintenance gate,
  persistent conflicts, and foreground revision refresh.
- `src/lib/{api,types,smoke.test}.ts` — typed IPC and UI regression coverage.

## Architectural Decision

SQLite is the sole mutable control-plane authority after an explicit verified cutover. Domain
documents remain bounded versioned JSON payloads inside SQLite for a low-risk migration; package
artifacts and Keychain secrets remain outside it. See `memory-bank/decisions.md#2026-08-06-sqlite-is-the-control-plane-authority`.

## Artifacts

- Implementation commit: `3d3a838`
- Branch: `feat/sqlite-control-plane`
- Design: `docs/superpowers/specs/2026-08-06-sqlite-control-plane-design.md`
- Plan: `docs/superpowers/plans/2026-08-06-sqlite-control-plane.md`

