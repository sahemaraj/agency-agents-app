# Agent Lifecycle Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Do not begin implementation or commit without explicit user approval.

**Goal:** Upgrade Agent installation from the current five-state built-in-only ledger to source-aware, backup-first, seven-state lifecycle parity with Skills, including plans, dependencies, collections, history, rollback, disable/enable, and source-unavailable recovery.

**Architecture:** Keep `src-tauri/src/install/mod.rs` as the only owner of Agent destination resolution, rendering, ledger reconciliation, and mutations. Add one focused history submodule for snapshot persistence. Resolve source content through the Stage 1 `agents` facade immediately before every mutation; no Agent source logic enters `install`.

**Tech Stack:** Rust 2021, Tauri 2, Tokio, Serde, SHA-256, existing capability-relative filesystem helpers and renderers, Svelte 5, TypeScript 5.6, Vitest.

## Global Constraints

- Stage 1 must be accepted and green before starting this plan.
- Never overwrite foreign content, silently adopt it, or remove modified content without a verified backup.
- Re-resolve and revalidate the exact `AgentReference` before every install/update.
- Persist the ledger only after destination publication succeeds; restore the prior destination when ledger persistence fails.
- Preserve old `installs.json` rows and accept the old `removed` wire value as `missing`.
- Migration must not modify installed files. It must be idempotent, atomic, and recoverable.
- Project writes remain beneath an authorized canonical project root using existing capability-relative operations.
- Batch install is all-or-rollback for writes made by that batch.
- Recommended dependencies are informational only.
- Do not add a second destination resolver, renderer registry, project registry, or install ledger.
- Commit steps are conditional on separate explicit user approval.

## Task 1: Extend and Migrate the Agent Install Ledger

**Files:**

- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/install/mod.rs`

**Ledger additions:**

```rust
pub struct InstallRecord {
    // existing fields remain
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub disabled_path: Option<String>,
    #[serde(default)]
    pub source_snapshot_hash: String,
}

pub enum InstallState {
    Current,
    Outdated,
    Modified,
    #[serde(alias = "removed")]
    Missing,
    Foreign,
    Disabled,
    SourceUnavailable,
}
```

### Steps

- [ ] Add fixtures for a pre-feature ledger, a mixed migrated/unmigrated ledger, an unknown legacy slug, the old `removed` state, and malformed source identity.
- [ ] Implement an in-memory migration function:

```rust
fn migrate_install_records(
    records: Vec<InstallRecord>,
    built_in: &AgentSourceResult,
) -> Result<(Vec<InstallRecord>, bool), AppError>;
```

- [ ] Resolve a legacy slug only when exactly one built-in package matches. Use its canonical relative path and `builtin:agency-agents` source ID.
- [ ] Assign an unresolved row the stable source ID `legacy:unresolved` and derive a normalized collision-free relative path from its existing destination filename; preserve all existing provenance.
- [ ] On a changed ledger, write a timestamped migration backup beside `installs.json`, write the migrated file to a temporary sibling, sync it, and atomically replace the original.
- [ ] Keep the migration backup until one successful reconciliation has completed; then prune it with the same bounded-retention rule used for Agent history.
- [ ] Test that repeated loads are byte-stable after the first successful migration and that injected save failure leaves the original ledger untouched.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml install::tests::migration
```

### Acceptance

- Every loaded record has a stable source-aware identity.
- Unknown rows remain visible, removable, and recoverable.
- Migration performs no destination write or move.

### Conditional commit

```bash
git add src-tauri/src/types.rs src-tauri/src/install/mod.rs
git commit -m "feat: migrate agent install identity"
```

## Task 2: Implement Seven-State Reconciliation

**Files:**

- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/types.rs`

**Classification order:**

```text
managed + disabled path matches       -> Disabled
managed + source unavailable          -> SourceUnavailable
managed + destination missing         -> Missing
managed + rendered hash differs       -> Modified
managed + source hash differs         -> Outdated
managed + all hashes match            -> Current
unmanaged recognized destination      -> Foreign
```

### Steps

- [ ] Add a table-driven test covering all seven states, modified-plus-source-missing precedence, occupied disabled destination, wrong disabled content, and same-name Agents from distinct sources.
- [ ] Change reconciliation lookup from slug-only to `AgentReference`, while using stored ledger provenance when the source cannot resolve.
- [ ] Verify disabled content against the ledger's rendered hash and refuse to claim an unrelated hidden sibling.
- [ ] Keep the current foreign sweep, but attach provenance only when an exact source/render match exists; otherwise leave it untracked.
- [ ] Return `sourceId` and `relativePath` on `InstalledAgent`; preserve `slug` for display and old frontend call sites.
- [ ] Update all existing Rust assertions from `Removed` to `Missing` and add serde coverage for reading `"removed"`.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml install::tests::reconcile
cargo test --manifest-path src-tauri/Cargo.toml install::
```

### Acceptance

- Classification is deterministic and source-aware.
- Removing a source changes managed installs to `SourceUnavailable`, not `Current` or `Foreign`.
- Existing unmanaged detection remains non-destructive.

### Conditional commit

```bash
git add src-tauri/src/install/mod.rs src-tauri/src/types.rs
git commit -m "feat: reconcile seven agent lifecycle states"
```

## Task 3: Add Backup, History, Rollback, Disable, and Enable

**Files:**

- Create: `src-tauri/src/install/history.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/lib.rs`

**History identity:**

```rust
pub struct AgentInstallIdentity {
    pub reference: AgentReference,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
}

pub struct AgentVersionSnapshot {
    pub id: String,
    pub created_at: String,
    pub source_hash: String,
    pub rendered_hash: String,
    pub content_path: String,
}
```

### Steps

- [ ] Add failure-injection tests for backup creation, snapshot-index save, destination move/write, ledger save, rollback restoration, and occupied enable destination.
- [ ] Store bounded snapshots under `<app-data>/agents/history/`, keyed by a hash of `AgentInstallIdentity`; never use source-provided path text directly as a storage path.
- [ ] Snapshot the exact current file or directory before every managed replacement. Verify the snapshot hash before any destination mutation.
- [ ] Refactor current install/update/uninstall commands around crate-visible core functions that accept resolved `AppState`, `AgentReference`, tool, scope, and authorized project capability. Tauri wrappers continue resolving `AppHandle` paths.
- [ ] Add `agent_version_history`, `agent_version_rollback`, `disable_agent`, and `enable_agent` commands.
- [ ] Disable by moving the exact managed destination to one deterministic same-parent hidden sibling and storing `disabledPath`. Refuse if either destination is occupied by unrelated content.
- [ ] Roll back only a snapshot owned by the same install identity; back up modified current content first.
- [ ] For any failure after destination mutation, restore the prior bytes/path and prior ledger row before returning an error.
- [ ] Retain a bounded history per install and prune only after the new snapshot and ledger are durable.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml install::history::
cargo test --manifest-path src-tauri/Cargo.toml install::tests::lifecycle
```

### Acceptance

- No update, rollback, disable, or destructive uninstall loses divergent content.
- Enable refuses an occupied target.
- History and rollback cannot cross Agent, tool, scope, or project identities.

### Conditional commit

```bash
git add src-tauri/src/install/history.rs src-tauri/src/install/mod.rs src-tauri/src/types.rs src-tauri/src/lib.rs
git commit -m "feat: add agent lifecycle recovery"
```

## Task 4: Make Install, Update, Track, and Uninstall Source-Aware

**Files:**

- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/agents/mod.rs`

### Steps

- [ ] Add tests for exact-reference install, same-slug source collision, destination collision, foreign tracking, source-unavailable uninstall, modified uninstall backup, missing uninstall, and update capability broadening.
- [ ] Change desktop mutation inputs to accept `sourceId + relativePath`; retain deprecated slug-only adapters only for exact unambiguous built-in matches.
- [ ] Resolve and validate the source bytes immediately before rendering. Compare the inspection hash with the bytes being rendered.
- [ ] Resolve every destination through the existing `Tool` registry and renderer; do not infer destination from source paths.
- [ ] Block install/update when another ledger row or foreign item owns the destination, returning both identities.
- [ ] Make tracking non-destructive: hash the existing destination, match it when possible, and create a ledger record without rewriting content.
- [ ] Apply update-policy decisions from `AgentLibraryState`: `Notify`, `AutoTrusted`, `Pin`, and `ReviewScripts`. `AutoTrusted` requires unchanged source/publisher trust and no broadened capability inventory.
- [ ] Permit `SourceUnavailable` uninstall from stored provenance. Missing uninstall removes only the exact ledger row. Modified uninstall must complete a verified backup first.
- [ ] Keep existing command names as adapters where frontend compatibility requires them; expose the source-aware request shape in TypeScript during Task 6.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml install::tests::install
cargo test --manifest-path src-tauri/Cargo.toml install::tests::uninstall
cargo test --manifest-path src-tauri/Cargo.toml install::
```

### Acceptance

- No mutation selects an Agent by display name or slug alone.
- Existing foreign content is never overwritten during install or tracking.
- Source-unavailable and modified installs remain safely uninstallable.

### Conditional commit

```bash
git add src-tauri/src/install/mod.rs src-tauri/src/lib.rs src-tauri/src/types.rs src-tauri/src/agents/mod.rs
git commit -m "feat: make agent lifecycle source aware"
```

## Task 5: Add Dependency and Collection Mutation Plans

**Files:**

- Modify: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/lib.rs`

**DTOs:**

```rust
pub struct AgentPlanItem {
    pub reference: AgentReference,
    pub name: String,
    pub dependency: bool,
    pub destination: String,
    pub rendered_file_count: u32,
    pub capabilities: Vec<String>,
}

pub struct AgentMutationPlan {
    pub operation: String,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
    pub agents: Vec<AgentPlanItem>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub rollback_available: bool,
}
```

### Steps

- [ ] Add graph tests for deterministic topological order, missing dependency, ambiguous name, preferred-source resolution, self-cycle, multi-node cycle, invalid dependency, and recommended-only entries.
- [ ] Resolve required dependency names to exact references. A unique match is accepted; multiple matches require a valid preferred-source record or become a blocker.
- [ ] Build plans without writing files. Include destinations, capabilities, warnings, blockers, processing order, and rollback availability.
- [ ] Add Tauri commands for single/dependency install plans, install-with-dependencies, and collection install/update/uninstall plans.
- [ ] Execute batch install as one transaction log: on failure, roll back only changes made by the batch in reverse order and report rollback status.
- [ ] Apply the same preflight and preservation checks to collection update/uninstall; recommended Agents remain out of the executable set.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::tests::dependency
cargo test --manifest-path src-tauri/Cargo.toml install::tests::plan
cargo test --manifest-path src-tauri/Cargo.toml install::tests::batch
```

### Acceptance

- Any blocker prevents all writes.
- Dependency order is deterministic across platforms.
- A failed batch leaves pre-existing destinations and ledger rows restored.

### Conditional commit

```bash
git add src-tauri/src/agents/mod.rs src-tauri/src/install/mod.rs src-tauri/src/types.rs src-tauri/src/lib.rs
git commit -m "feat: add agent dependency and collection plans"
```

## Task 6: Add Desktop Lifecycle Controls

**Files:**

- Create: `src/lib/components/AgentInstallPlanModal.svelte`
- Modify: `src/lib/components/AgentsWorkspace.svelte`
- Modify: `src/lib/components/DeploymentMatrix.svelte`
- Modify: `src/lib/components/InstallModal.svelte`
- Modify: `src/lib/stores/install.svelte.ts`
- Modify: `src/lib/stores/agentLibrary.svelte.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/i18n/locales/en.ts`
- Modify: `src/lib/i18n/messages.test.ts`

### Steps

- [ ] Add exact TypeScript DTOs and source-aware API inputs; keep any old slug adapter private to the API module.
- [ ] Extend the install store to key rows by reference + tool + scope + project, not slug.
- [ ] Display all seven lifecycle states with visible text, not color alone; show source provenance when names collide.
- [ ] Add plan review before dependency or collection mutations, including blockers, warnings, destinations, capabilities, and rollback availability.
- [ ] Add Update, Disable, Enable, Version History, Rollback, Track, and Uninstall actions with existing confirmation and diff components.
- [ ] Require explicit confirmation when policy is `Notify` or capabilities broaden. Show why `Pin`, `AutoTrusted`, or `ReviewScripts` blocked an update.
- [ ] Keep destination selection in the existing `DeploymentTargetGrid`/install modal flow.
- [ ] Add English strings and locale-key parity tests.
- [ ] Add pure store/model tests for collision keys, state grouping, policy decisions, and plan blockers.
- [ ] Run:

```bash
npm run check
npm run test:frontend
npm run build
```

### Acceptance

- Every lifecycle state has an accessible action path and explanatory text.
- Dependency/collection operations cannot execute until a blocker-free plan is reviewed.
- Duplicate names never share frontend state or action targets.

### Conditional commit

```bash
git add src/lib/components/AgentInstallPlanModal.svelte src/lib/components/AgentsWorkspace.svelte src/lib/components/DeploymentMatrix.svelte src/lib/components/InstallModal.svelte src/lib/stores/install.svelte.ts src/lib/stores/agentLibrary.svelte.ts src/lib/api.ts src/lib/types.ts src/lib/i18n/locales/en.ts src/lib/i18n/messages.test.ts
git commit -m "feat: expose complete agent lifecycle"
```

## Stage 2 Verification Gate

- [ ] Rehearse migration against a copy of a real pre-feature ledger and verify installed destination hashes are unchanged.
- [ ] Test user and project scope for every tool registry entry supporting that scope.
- [ ] Test Windows separators/case collisions and macOS/Linux same-parent disable paths using platform-specific unit fixtures.
- [ ] `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `npm run verify:frontend`
- [ ] Confirm `git diff --check` is clean.
- [ ] Present migration evidence, lifecycle matrix results, unified diff, and QA output for approval before any commit or Stage 3 work.
