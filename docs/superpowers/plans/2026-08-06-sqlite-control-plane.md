# SQLite Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task. Do not commit or merge without explicit user approval.

**Goal:** Replace fragmented live JSON metadata with a recoverable, multi-process SQLite control plane while preserving filesystem package formats, Tauri/MCP APIs, domain validation, and human approval boundaries.

**Architecture:** Add one backend-owned `StateDatabase` beneath the existing domain loaders and mutation functions. Store current validated Serde documents as capped, versioned JSON rows; serialize every read-modify-write in `BEGIN IMMEDIATE`; coordinate package files with a minimal durable operation journal; cut over only after a verified backup and exclusive maintenance migration.

**Tech Stack:** Rust 2021, Tauri 2, Tokio `spawn_blocking`, Serde, rusqlite with bundled SQLite and Online Backup API, Svelte 5, TypeScript 5.6, Vitest 4.

---

## Global Constraints

- Work only in `/Users/home/.config/superpowers/worktrees/agency-agents-app/sqlite-control-plane` on `feat/sqlite-control-plane`.
- Preserve every Tauri command name, MCP tool/resource name, DTO wire shape, exact identity, and approval policy.
- Do not move, rewrite, or blob-store Skill/Agent package artifacts.
- Do not introduce a repository trait, ORM, connection pool, document database, cloud service, FTS, or speculative normalized schema.
- Never hold a SQLite transaction across `.await`, network access, or filesystem I/O.
- Never silently fall back to or merge legacy JSON after successful cutover.
- Reuse every existing domain validator, size/count cap, atomic staging path, hash check, containment check, and rollback routine.
- All behavioral production changes follow red-green-refactor using `test-driven-development`.
- Commit commands below are approval gates, not authorization.

## Reuse Analysis

- Extend `src-tauri/src/state.rs` for lifecycle ownership and Tauri state; it is already the app-data root and cross-cutting state owner.
- Add `src-tauri/src/state_db.rs` because no current module can own SQLite schema, connection configuration, transactions, online backup, migration state, and operation journal without mixing those concerns into the 1,400-line authorization/audit module.
- Extend existing private `load_*`/`save_*` functions in each domain; do not replace public commands or create parallel domain services.
- Extend `src/lib/api.ts` and `src/lib/types.ts` for migration/revision DTOs.
- Add one `StorageMigrationGate.svelte`; existing settings modals cannot represent a startup-blocking, multi-stage, persistent success/failure flow without coupling settings navigation to application boot.
- Follow existing plan/spec locations rather than creating a new documentation hierarchy.

## Task 1: Characterize Legacy Persistence and Add the SQLite Dependency

**Files:**

- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/state.rs`
- Modify tests in: `src-tauri/src/skills/{mod.rs,drafts.rs,organize.rs,install.rs}`
- Modify tests in: `src-tauri/src/agents/{mod.rs,drafts.rs,organize.rs}` and `src-tauri/src/install/mod.rs`
- Modify tests in: `src-tauri/src/{experts.rs,expert_runs.rs}`

### Steps

- [ ] Add failing characterization tests that create realistic bounded JSON documents and assert current loaders preserve empty state, IDs, exact references, approval revisions, trust signatures, install destinations, and Expert run evidence.
- [ ] Add a test inventory table containing each canonical legacy path, document name, document version, maximum bytes, parser, and semantic validator. Assert no migration-owned JSON file is missing from the table.
- [ ] Run the focused tests and confirm the inventory test fails because no inventory exists.
- [ ] Add `rusqlite = { version = "0.40.1", features = ["bundled", "backup"] }`.
- [ ] Add the minimum inventory constants/types to `state.rs`; keep all existing loaders active.
- [ ] Run:

```bash
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH \
  cargo test --manifest-path src-tauri/Cargo.toml persistence_inventory
```

**Acceptance:** Inventory is exhaustive and existing JSON behavior is unchanged.

**Conditional commit:**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/state.rs src-tauri/src/skills src-tauri/src/agents src-tauri/src/install/mod.rs src-tauri/src/experts.rs src-tauri/src/expert_runs.rs
git commit -m "test: characterize persisted application state"
```

## Task 2: Implement the File-Backed SQLite Primitive

**Files:**

- Create: `src-tauri/src/state_db.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/error.rs`

**Core interface:**

```rust
pub(crate) struct StateDatabase {
    path: PathBuf,
}

pub(crate) struct DocumentSpec<T> {
    pub name: &'static str,
    pub version: u32,
    pub max_bytes: u64,
    pub validate: fn(&T) -> Result<(), AppError>,
}

impl StateDatabase {
    pub(crate) fn open(app_data_dir: &Path) -> Result<Self, AppError>;
    pub(crate) async fn read<T>(&self, spec: DocumentSpec<T>) -> Result<Option<T>, AppError>;
    pub(crate) async fn mutate<T, R>(
        &self,
        spec: DocumentSpec<T>,
        default: T,
        mutation: impl FnOnce(&mut T) -> Result<R, AppError> + Send + 'static,
    ) -> Result<R, AppError>;
    pub(crate) async fn visible_revision(&self) -> Result<u64, AppError>;
}
```

### Steps

- [ ] Write file-backed tests for schema creation, supported/newer `user_version`, explicit empty documents, corrupt JSON, semantic validation, exact byte caps, and permissions.
- [ ] Write a two-connection test: hold `BEGIN IMMEDIATE` on connection A; assert connection B waits no more than five seconds and returns the stable busy error without changing data.
- [ ] Write a concurrent mutation test proving two independent `StateDatabase` instances preserve both changes rather than last-writer-wins.
- [ ] Verify tests fail because `state_db` does not exist.
- [ ] Implement the three-table schema from the approved design using `STRICT` tables and JSON validity checks.
- [ ] Configure WAL, `synchronous=FULL`, foreign keys, and a five-second busy timeout on every opened connection.
- [ ] Run all connection work in `spawn_blocking`; do not share a `rusqlite::Connection` across async tasks.
- [ ] Implement `read` and transactional `mutate`; deserialize, validate, serialize, cap, update the document, and bump `state_revision` in one transaction.
- [ ] Map busy, corrupt, unsupported-schema, and internal database failures to stable `AppError` payloads without paths or SQL text.
- [ ] Run:

```bash
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH \
  cargo test --manifest-path src-tauri/Cargo.toml state_db::
```

**Acceptance:** Multiple file-backed connections are correct; no test relies only on an in-memory database.

**Conditional commit:**

```bash
git add src-tauri/src/state_db.rs src-tauri/src/lib.rs src-tauri/src/state.rs src-tauri/src/error.rs
git commit -m "feat: add transactional sqlite state store"
```

## Task 3: Add Migration Inventory, Exclusive Lease, and Verified Backup

**Files:**

- Modify: `src-tauri/src/state_db.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`

**Migration DTO:**

```rust
pub(crate) enum StorageMigrationState {
    Legacy, InProgress, Complete, Corrupt, Unsupported,
}

pub(crate) struct StorageMigrationStatus {
    pub state: StorageMigrationState,
    pub stage: Option<String>,
    pub detail: Option<String>,
    pub legacy_conflicts: Vec<String>,
}
```

### Steps

- [ ] Write failing tests for shared process leases, exclusive cutover refusal, interrupted import, intentionally empty documents, malformed/oversized legacy files, unsupported schema, backup failure, and concurrent first startup.
- [ ] Add a fault-injection test that aborts before the `complete` marker and proves the JSON backend remains authoritative.
- [ ] Add tests proving missing keychain material with existing trust records and invalid HMAC signatures block migration without copying keys.
- [ ] Implement a shared lifetime lock on `state/storage.lock`; migration/restore uses a non-blocking exclusive lock and refuses while another current process holds a lease.
- [ ] Inventory and hash every legacy file before import; preserve exact byte caps and call existing semantic and cryptographic validators.
- [ ] Create a timestamped legacy backup directory, fsync/verify its sizes and SHA-256 inventory, and keep package artifacts out of it.
- [ ] Implement the import engine against registered document specifications and prove it with synthetic typed documents. Refuse `complete` unless every required inventory entry has a registered parser and semantic validator.
- [ ] Let Tasks 4–6 register their domain documents with the engine; production cutover remains unavailable throughout Checkpoint A.
- [ ] On failure, rollback the import, report `corrupt`, and continue using untouched legacy JSON only when no newer/unsupported schema is present.
- [ ] After `complete`, never read legacy JSON as state. Compare fingerprints at startup and report later modifications as conflicts.
- [ ] Add Online Backup API tests that write committed WAL content, create a backup, open it independently, run integrity check, and compare revisions.

**Acceptance:** The migration engine is repeatable before completion, exclusive among current processes, and cannot cut over with an incomplete validator registry.

**Conditional commit:**

```bash
git add src-tauri/src/state_db.rs src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/types.rs
git commit -m "feat: add recoverable sqlite migration"
```

## Task 4: Move Skills Control-Plane Documents Behind SQLite

**Files:**

- Modify: `src-tauri/src/skills/mod.rs`
- Modify: `src-tauri/src/skills/drafts.rs`
- Modify: `src-tauri/src/skills/organize.rs`
- Modify: `src-tauri/src/skills/install.rs`
- Modify: `src-tauri/src/state.rs`

### Steps

- [ ] Add dual-backend contract tests that run the same source, trust, draft, folder/approval, and install scenarios once against legacy temp JSON and once against migrated SQLite.
- [ ] Add two-independent-state tests for concurrent source registration, draft submission, folder mutation, approval submission/resolution, and install-ledger updates.
- [ ] Verify at least one test fails against the current per-process Skill library lock.
- [ ] Make each existing private loader select JSON only in `Legacy` state and SQLite only in `Complete` state.
- [ ] Replace every SQLite-mode read-modify-save sequence with one `StateDatabase::mutate` closure; retain current validators and JSON import/export wire shape.
- [ ] Include `skill-trust.json`; verify signatures before import and on read, while leaving the HMAC key in the keychain.
- [ ] Remove no legacy JSON code yet; it remains the pre-cutover backend and explicit export format.
- [ ] Run:

```bash
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH \
  cargo test --manifest-path src-tauri/Cargo.toml skills::
```

**Acceptance:** Skills desktop and MCP commands share SQLite after cutover with unchanged wire behavior and no lost updates.

**Conditional commit:**

```bash
git add src-tauri/src/skills src-tauri/src/state.rs
git commit -m "feat: persist skill lifecycle in sqlite"
```

## Task 5: Move Agents Control-Plane Documents Behind SQLite

**Files:**

- Modify: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/agents/drafts.rs`
- Modify: `src-tauri/src/agents/organize.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/state.rs`

### Steps

- [ ] Add dual-backend contract tests for Agent sources, drafts, library/approvals, install records, and project registry.
- [ ] Add independent-process-equivalent concurrency tests for duplicate display names and exact `(sourceId, relativePath)` mutations.
- [ ] Route SQLite-mode loaders and mutation closures through `StateDatabase` without changing public Tauri/MCP functions.
- [ ] Preserve legacy install migration, seven reconciliation states, project scoping, and source-aware identity.
- [ ] Keep version snapshot files and manifests on disk; store only their control-plane index/reference in SQLite.
- [ ] Run focused Agent, install, corpus, and render tests.

**Acceptance:** Same-name Agents remain independent and every existing lifecycle/recovery test passes against SQLite mode.

**Conditional commit:**

```bash
git add src-tauri/src/agents src-tauri/src/install/mod.rs src-tauri/src/state.rs
git commit -m "feat: persist agent lifecycle in sqlite"
```

## Task 6: Move Experts and Remaining Mutable Backend State

**Files:**

- Modify: `src-tauri/src/experts.rs`
- Modify: `src-tauri/src/expert_runs.rs`
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/install/mod.rs`

### Steps

- [ ] Add dual-backend tests for Expert definitions/change requests, activation requests/history, run evidence/blockers/waivers/review, settings/MCP policy, projects, and audit.
- [ ] Preserve fail-closed settings behavior when the database is corrupt or newer than the app.
- [ ] Move Expert and project documents to transactional mutations with existing ownership, idempotency, version, and capacity rules.
- [ ] Move the bounded MCP audit journal last; preserve redaction and maximum-entry behavior without incrementing the user-visible revision.
- [ ] Leave corpus indexes, catalog caches, GitHub cache, keychain records, and package/history artifacts on disk.
- [ ] Preserve portable, scoped, redacted JSON exports.

**Acceptance:** All mutable control-plane state has one live source after cutover; rebuildable caches and artifacts remain files by design.

**Conditional commit:**

```bash
git add src-tauri/src/experts.rs src-tauri/src/expert_runs.rs src-tauri/src/commands/settings.rs src-tauri/src/state.rs src-tauri/src/install/mod.rs
git commit -m "feat: persist expert and shared state in sqlite"
```

## Task 7: Add the Recoverable Filesystem Operation Journal

**Files:**

- Modify: `src-tauri/src/state_db.rs`
- Modify: `src-tauri/src/skills/drafts.rs`
- Modify: `src-tauri/src/skills/install.rs`
- Modify: `src-tauri/src/agents/drafts.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/experts.rs`

### Steps

- [ ] Define bounded journal payloads only for operations that cross SQLite/filesystem boundaries: Skill publish/install/update/disable/enable/uninstall, Agent publish/install lifecycle, Expert activation.
- [ ] Write fault-injection tests terminating at prepared, filesystem-applied, metadata-committed, and cleanup boundaries.
- [ ] Assert restart recovery is idempotent across two consecutive runs and never follows a link/reparse point or accepts a changed hash.
- [ ] Insert `prepared` before filesystem work; reuse existing staging/quarantine/backup implementation; then atomically apply metadata and `filesystem_applied`.
- [ ] Revalidate before cleanup and mark `committed`; retain failed recovery details without attempting destructive guessing.
- [ ] Reconcile approvals left Running using the operation ID and exact revision.
- [ ] Run all lifecycle, rollback, failure-injection, and security tests.

**Acceptance:** Forced termination at every boundary converges to one valid state without data loss or unsafe deletion.

**Conditional commit:**

```bash
git add src-tauri/src/state_db.rs src-tauri/src/skills src-tauri/src/agents src-tauri/src/install/mod.rs src-tauri/src/experts.rs
git commit -m "feat: recover database filesystem operations"
```

## Task 8: Add the Maintenance Gate and Cross-Process Refresh

**Files:**

- Create: `src/lib/components/StorageMigrationGate.svelte`
- Modify: `src/routes/+page.svelte`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/stores/skillSources.svelte.ts`
- Modify: `src/lib/stores/agentLibrary.svelte.ts`
- Modify: `src/lib/stores/experts.svelte.ts`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src/lib/smoke.test.ts`

### Steps

- [ ] Add Tauri commands for migration status/start/retry, user-visible revision, backup, conflict dismissal, and opening the data directory; keep raw SQL private.
- [ ] Write failing component tests for legacy prompt, named stages, `aria-busy`, permanent completion, reassuring failure, unsupported schema, recovery, copy details, and keyboard/focus behavior.
- [ ] Implement the blocking gate with the approved user language and no fake percentage.
- [ ] Write failing store/component tests showing an MCP-side revision updates an already-open approval inbox without closing it, resetting scroll, stealing focus, or showing a toast.
- [ ] Poll only while the app is foregrounded and a relevant surface is visible; use a one-second maximum interval, refresh on focus, and refresh immediately after local writes.
- [ ] Reuse existing stale-approval presentation for revision conflicts.
- [ ] Show post-cutover legacy file conflicts as a persistent warning, never a transient toast.
- [ ] Run `npm run verify:frontend`.

**Acceptance:** Migration is understandable and state created by MCP becomes visible within one second on open desktop surfaces.

**Conditional commit:**

```bash
git add src/lib/components/StorageMigrationGate.svelte src/routes/+page.svelte src/lib/api.ts src/lib/types.ts src/lib/stores src/lib/smoke.test.ts src-tauri/src/lib.rs src-tauri/src/state.rs
git commit -m "feat: add sqlite migration and live refresh UI"
```

## Task 9: Migration Rehearsal, Security Review, and Final Verification

**Files:**

- Modify only files required by failing evidence.
- Do not write Memory Bank completion records until final human approval.

### Steps

- [ ] Copy—not move—the real app-data state into a temporary directory and record all source file hashes, IDs, counts, and package/artifact hashes.
- [ ] Run migration against the copy; compare every imported document and verify package/artifact hashes are unchanged.
- [ ] Exercise simultaneous first startup, held write locks, busy timeout, malformed and oversized input, invalid trust signatures, unsupported versions, interrupted migration, legacy-file rewrites, and every journal crash boundary.
- [ ] Restore an Online Backup API snapshot into a second temp directory and validate it independently.
- [ ] Run:

```bash
npm run verify:frontend
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo test --manifest-path src-tauri/Cargo.toml
git diff --check
```

- [ ] Verify the Tauri command inventory and MCP Skills/Agents/Experts tool counts are unchanged.
- [ ] Verify database, backup, WAL, and SHM permissions contain no group/world access.
- [ ] Present diff, test counts, migration inventory comparison, security review, known limitations, and rollback evidence at the human approval gate.

**Acceptance:** All suites pass; copied real state migrates with exact semantic equality and unchanged artifacts; no unreviewed write touches live app data.

## Explicitly Deferred

- Relational normalization and foreign-key relationships inside domain JSON.
- FTS5/full catalog search.
- Cloud synchronization, accounts, collaboration, and marketplace hosting.
- SQLCipher/database-level encryption unless secrets become part of the database.
- Automatic mixed-version merge or downgrade. Export/restore is explicit and version-checked.

## Execution Checkpoints

- **Checkpoint A:** Tasks 1–3 — foundation and migration rehearsal; no production cutover.
- **Checkpoint B:** Tasks 4–6 — every mutable domain behind SQLite mode.
- **Checkpoint C:** Tasks 7–8 — crash recovery and user-visible cutover.
- **Checkpoint D:** Task 9 — full evidence and final human approval.

Execute at most three tasks per batch and report evidence before continuing, per `executing-plans`.
