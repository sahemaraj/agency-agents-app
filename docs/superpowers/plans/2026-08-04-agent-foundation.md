# Agent Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Do not begin implementation or commit without explicit user approval.

**Goal:** Give Agents source-aware identity, local/GitHub/custom sources, validated drafts, and the complete personal-library data model while preserving every existing built-in catalog workflow.

**Architecture:** Keep `corpus` as the built-in catalog provider. Add an `agents` domain facade for mutable sources, drafts, and organization. Extract only the path/reference/folder invariants that are genuinely shared with Skills into one small `library.rs` module; do not introduce a generic package framework.

**Tech Stack:** Rust 2021, Tauri 2, Tokio, Serde, existing `git2`/filesystem helpers, Svelte 5, TypeScript 5.6, Vitest.

## Global Constraints

- Preserve the current `corpus_*` and `install_*` wire contracts during this stage.
- Treat `AgentReference { sourceId, relativePath }` as identity; slug and name remain metadata.
- Use the built-in source ID `builtin:agency-agents` everywhere, independent of its physical catalog mode.
- Keep source files read-only. Draft publication writes only to the app-owned published source.
- Reject symlink/reparse roots, traversal, absolute paths, case-folded collisions, oversized files, and duplicate identities before persistence.
- Source refresh is transactional: retain the prior generation and registry entry on any failure.
- Invalid drafts remain saved with diagnostics but are not publishable or installable.
- Do not add dependencies. Reuse `atomic_write`, existing network gates, GitHub source code, parser, and UI primitives.
- Run commands from the repository root unless a task says otherwise.
- Commit steps below are approval gates, not authorization; execute them only after the user explicitly approves committing.

## Task 1: Characterize Built-In Agent and Skills Behavior

**Files:**

- Modify: `src-tauri/src/corpus/mod.rs`
- Modify: `src-tauri/src/corpus/parse.rs`
- Modify: `src-tauri/src/skills/organize.rs`
- Modify: `src-tauri/src/skills/mcp.rs`

**Purpose:** Protect the existing parser, rendered identity, Skills folder rules, and Skills MCP surface before extracting shared invariants.

### Steps

- [ ] Add a corpus regression test with two nested Markdown files that share a filename/slug. Assert discovery preserves two rows in deterministic relative-path order; mark the current overwrite behavior as the failing assertion that Task 3 will fix.
- [ ] Add parser fixtures asserting unknown frontmatter fields remain tolerated and the existing source/frontmatter/body hashes do not change.
- [ ] Add Skills organization tests for the current limits: 256 folders, depth eight, segment length 64, case-insensitive uniqueness, descendant rewrites, non-recursive deletion refusal, and recursive reference cleanup.
- [ ] Add an MCP router characterization test asserting the exact current `skills_*` tool-name set and count, so later router composition cannot drop or rename a Skills tool.
- [ ] Run the focused tests and record the single expected nested-identity failure:

```bash
cargo test --manifest-path src-tauri/Cargo.toml corpus::
cargo test --manifest-path src-tauri/Cargo.toml skills::organize::
cargo test --manifest-path src-tauri/Cargo.toml skills::mcp::
```

### Acceptance

- Existing characterization tests pass.
- The nested duplicate-slug test fails only because the current index is keyed by slug.
- No production behavior changes.

### Conditional commit

```bash
git add src-tauri/src/corpus/mod.rs src-tauri/src/corpus/parse.rs src-tauri/src/skills/organize.rs src-tauri/src/skills/mcp.rs
git commit -m "test: characterize agent and skill library behavior"
```

## Task 2: Extract Shared Reference and Folder Invariants

**Files:**

- Create: `src-tauri/src/library.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/skills/organize.rs`

**Interface:**

```rust
pub(crate) const MAX_LIBRARY_FOLDERS: usize = 256;
pub(crate) const MAX_LIBRARY_FOLDER_DEPTH: usize = 8;
pub(crate) const MAX_LIBRARY_FOLDER_SEGMENT_CHARS: usize = 64;

pub(crate) fn normalize_relative_path(value: &str) -> Result<String, AppError>;
pub(crate) fn portable_path_key(value: &str) -> Result<String, AppError>;
pub(crate) fn validate_reference(source_id: &str, relative_path: &str) -> Result<(), AppError>;
pub(crate) fn validate_folder_path(value: &str) -> Result<(), AppError>;
pub(crate) fn create_folder(folders: &mut Vec<String>, path: String) -> Result<(), AppError>;
pub(crate) fn rename_folder_paths(folders: &[String], path: &str, new_name: &str) -> Result<Vec<(String, String)>, AppError>;
pub(crate) fn move_folder_paths(folders: &[String], path: &str, new_parent: Option<&str>) -> Result<Vec<(String, String)>, AppError>;
pub(crate) fn deleted_folder_paths(folders: &[String], path: &str, recursive: bool) -> Result<Vec<String>, AppError>;
```

The rewrite helpers return old/new path pairs. Skills and Agents remain responsible for applying those pairs to their own assignment and profile types.

### Steps

- [ ] Write unit tests in `library.rs` for normalized slash-separated relative paths, `.`/`..`, absolute paths, backslashes, NULs, Unicode, ASCII case collisions, and folder boundary limits.
- [ ] Implement path normalization with `std::path::Component`; do not canonicalize a user-provided relative path into a different identity.
- [ ] Move only the pure folder path calculations from `skills/organize.rs` into `library.rs`.
- [ ] Adapt Skills organization code to call the shared helpers while preserving Skills error semantics and serialized state.
- [ ] Register `mod library;` in `lib.rs`.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml library::
cargo test --manifest-path src-tauri/Cargo.toml skills::organize::
```

### Acceptance

- Shared helpers have no filesystem or domain-specific DTO dependency.
- All pre-extraction Skills tests remain green.
- No new dependency or generalized repository/service abstraction is introduced.

### Conditional commit

```bash
git add src-tauri/src/library.rs src-tauri/src/lib.rs src-tauri/src/skills/organize.rs
git commit -m "refactor: share library path invariants"
```

## Task 3: Add Source-Aware Agent Inspection

**Files:**

- Create: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/corpus/mod.rs`
- Modify: `src-tauri/src/corpus/parse.rs`
- Modify: `src-tauri/src/state.rs`

**DTOs:**

```rust
pub struct AgentReference {
    pub source_id: String,
    pub relative_path: String,
}

pub enum AgentSourceKind {
    BuiltIn,
    Local { root: String },
    Github { repository: String, reference: Option<String>, subdirectory: Option<String>, checkout: String },
    Published { root: String },
}

pub struct AgentSource {
    pub id: String,
    pub kind: AgentSourceKind,
    pub label: String,
    pub enabled: bool,
}

pub struct AgentPackageResult {
    pub reference: AgentReference,
    pub agent: Agent,
    pub source_hash: String,
    pub frontmatter_hash: String,
    pub body_hash: String,
    pub version: Option<String>,
    pub channel: Option<String>,
    pub changelog: Option<String>,
    pub publisher: Option<String>,
    pub publisher_key: Option<String>,
    pub required_agents: Vec<String>,
    pub recommended_agents: Vec<String>,
    pub groups: Vec<String>,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub quality_score: u8,
    pub diagnostics: Vec<AgentValidationError>,
    pub installable: bool,
}

pub struct AgentSourceResult {
    pub source: AgentSource,
    pub agents: Vec<AgentPackageResult>,
    pub errors: Vec<AgentValidationError>,
}
```

### Steps

- [ ] Add serde-compatible Agent source/reference/package/diagnostic DTOs to `types.rs`. Optional metadata defaults empty so existing upstream Markdown stays valid.
- [ ] Extend the parser's private frontmatter DTO with the approved optional fields and return an inspection payload without reserializing or changing canonical hashes.
- [ ] Change built-in corpus indexing to retain normalized relative paths and prevent slug overwrite. Keep the legacy slug lookup only when it resolves exactly one built-in Agent; ambiguous slug calls return a typed conflict.
- [ ] Replace `find_md_under(category, filename)` source lookup with exact `relativePath` lookup. Keep the legacy category/slug wrapper as an unambiguous adapter for current callers during Stage 1.
- [ ] Add `agents/mod.rs` with bounded source registry persistence at `state/agent-sources.json`, an implicit built-in entry, discovery, inspection, exact-source reads, revision hashing, and refresh transactions.
- [ ] Reuse `skills/mod.rs` GitHub URL validation, network authorization, checkout layout, file bounds, link/reparse checks, and atomic activation by extracting only crate-visible helpers needed by both domains. Do not duplicate Git command execution.
- [ ] Add one Agent source cache/refresh lock to `AppState`; keep the existing corpus cache unchanged.
- [ ] Register Tauri commands:

```rust
agents::agent_sources_list
agents::agent_sources_inspect
agents::agent_source_add_local
agents::agent_source_add_github
agents::agent_source_refresh
agents::agent_source_remove
agents::agent_source_status
agents::agent_get
agents::agent_text_read
```

- [ ] Test duplicate display names/slugs across sources, duplicate relative paths within one source, nested paths, deterministic ordering, refresh rollback, source unregister without deletion, and link/reparse rejection.
- [ ] Make the Task 1 nested-identity regression pass.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml corpus::
cargo test --manifest-path src-tauri/Cargo.toml agents::
cargo test --manifest-path src-tauri/Cargo.toml skills::
```

### Acceptance

- Two same-slug Agents can coexist when their canonical references differ.
- Exact reads never search recursively by filename.
- Built-in Agent list/get behavior remains backward-compatible for unambiguous slugs.
- Failed refresh leaves the previous source generation active.

### Conditional commit

```bash
git add src-tauri/src/agents/mod.rs src-tauri/src/lib.rs src-tauri/src/types.rs src-tauri/src/corpus/mod.rs src-tauri/src/corpus/parse.rs src-tauri/src/state.rs src-tauri/src/skills/mod.rs
git commit -m "feat: add source-aware agent catalog"
```

## Task 4: Add Agent Draft Creation and Publication

**Files:**

- Create: `src-tauri/src/agents/drafts.rs`
- Modify: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`

**Interface:**

```rust
pub enum AgentDraftState { Pending, Published, Rejected }

pub struct AgentDraft {
    pub id: String,
    pub submitted_at: String,
    pub state: AgentDraftState,
    pub relative_path: String,
    pub source_hash: String,
    pub validation: AgentPackageResult,
    pub published_source_id: Option<String>,
}

pub struct AgentDraftInput {
    pub relative_path: String,
    pub text: String,
}
```

### Steps

- [ ] Write tests for blank-form serialization, duplicate-as-draft, edit-as-draft, bounded file size/count, invalid-but-retained drafts, conflict refusal, publish rollback, rejection, and source-file immutability.
- [ ] Implement one-file drafts under `<app-data>/agents/drafts/` and an atomic index at `state/agent-drafts.json`, following `skills/drafts.rs` locking and rollback behavior.
- [ ] Validate exact input bytes through the Agent parser and store diagnostics even when validation fails.
- [ ] Publish only a valid pending draft to `<app-data>/agents/published/<relativePath>` using exclusive creation plus atomic index update. Never replace an existing target.
- [ ] Register the app-owned published source once and refresh it after successful publication.
- [ ] Expose Tauri commands for list/get/create/edit/publish/reject and duplicate-to-draft.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::drafts::
cargo test --manifest-path src-tauri/Cargo.toml agents::
```

### Acceptance

- Blank, duplicate, imported file, and edit-as-draft paths converge on one validated draft store.
- Invalid drafts survive reload but cannot publish.
- Publication conflicts return an explicit rename-required error and do not overwrite.

### Conditional commit

```bash
git add src-tauri/src/agents/drafts.rs src-tauri/src/agents/mod.rs src-tauri/src/lib.rs src-tauri/src/types.rs
git commit -m "feat: add agent draft workflow"
```

## Task 5: Add Agent Personal-Library Persistence

**Files:**

- Create: `src-tauri/src/agents/organize.rs`
- Modify: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`

**State:**

```rust
pub struct AgentLibraryState {
    pub folders: Vec<String>,
    pub assignments: Vec<AgentFolderAssignment>,
    pub favorites: Vec<AgentReference>,
    pub recent: Vec<AgentRecent>,
    pub collections: Vec<AgentCollection>,
    pub smart_folders: Vec<AgentSmartFolder>,
    pub profiles: Vec<AgentWorkspaceProfile>,
    pub update_policies: Vec<AgentUpdatePolicyRecord>,
    pub publisher_trust: Vec<AgentPublisherTrust>,
    pub preferred_sources: Vec<AgentPreferredSource>,
    pub usage: Vec<AgentUsage>,
    pub approvals: Vec<AgentApproval>,
}
```

### Steps

- [ ] Add Agent-specific organization DTOs. Reuse shared `AgentReference` but keep Agent smart-folder fields domain-specific: query, division, source, capability, lifecycle state, installable, and favorite.
- [ ] Implement versioned atomic persistence at `state/agent-library.json` with a file lock and the same numeric bounds as Skills.
- [ ] Use `library.rs` for folder create/rename/move/delete calculations; apply rewrites atomically to Agent assignments and profiles.
- [ ] Implement favorites, recent, collections, smart folders, profiles, update policies, publisher trust, preferred sources, usage counters, approval list, and `contentKind: agents` import/export.
- [ ] Reject Skills organization imports and non-normalized/unknown references.
- [ ] Register Tauri organization commands matching the existing Skills desktop surface with `agent_` names.
- [ ] Test every mutation, recursive deletion semantics, case collisions, stale references, import/export versioning, and concurrent write serialization.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::organize::
cargo test --manifest-path src-tauri/Cargo.toml skills::organize::
```

### Acceptance

- Nested logical folders do not move source or installed files.
- Folder rewrites update descendants, assignments, and profiles in one persisted transaction.
- Agent and Skills organization documents cannot be cross-imported.

### Conditional commit

```bash
git add src-tauri/src/agents/organize.rs src-tauri/src/agents/mod.rs src-tauri/src/lib.rs src-tauri/src/types.rs
git commit -m "feat: add agent library organization"
```

## Task 6: Wire the Foundation into the Agents Workspace

**Files:**

- Create: `src/lib/agents/libraryModel.ts`
- Create: `src/lib/agents/libraryModel.test.ts`
- Create: `src/lib/stores/agentLibrary.svelte.ts`
- Create: `src/lib/components/AgentLibrarySidebar.svelte`
- Create: `src/lib/components/AgentSourceManager.svelte`
- Create: `src/lib/components/AgentCreatorModal.svelte`
- Modify: `src/lib/components/AgentsWorkspace.svelte`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/i18n/locales/en.ts`
- Modify: `src/lib/i18n/messages.test.ts`

### Steps

- [ ] Add TypeScript DTOs and API wrappers matching the Rust camelCase wire types exactly.
- [ ] Port only the pure tree/filter/collision helpers needed from `src/lib/skills/libraryModel.ts`; keep Agent smart-filter logic separate and cover it in `libraryModel.test.ts`.
- [ ] Add one Agent library store responsible for source inspection, draft/library state, selected reference, refresh revisions, and mutation reloads.
- [ ] Add a nested logical-folder sidebar using buttons/tree semantics, keyboard navigation, accessible labels, and visible source provenance for collisions.
- [ ] Add source management for local folder and GitHub repository registration, refresh/status, and unregister confirmation. Use native directory/file dialogs through the existing Tauri dialog plugin.
- [ ] Add one creator modal supporting blank, duplicate, import file, edit-as-draft, diagnostics, publish, and reject. Folder import routes to source registration instead of copying files.
- [ ] Keep `AgentsWorkspace.svelte` as orchestration owner and reuse `Modal`, `DestructiveConfirm`, `Button`, `Input`, `EmptyState`, `LoadingState`, and current Agent list/detail rendering.
- [ ] Add all visible strings to English; verify locale fallback with `messages.test.ts` rather than duplicating incomplete translations in this stage.
- [ ] Run:

```bash
npm run check
npm run test:frontend -- --run src/lib/agents/libraryModel.test.ts src/lib/i18n/messages.test.ts
npm run build
```

### Acceptance

- Users can register/refresh/remove sources, browse nested logical folders, create/edit/import/duplicate drafts, and publish valid drafts.
- Built-in rows remain read-only; edit always creates an independent draft.
- Keyboard focus returns to the initiating control when a modal closes.

### Conditional commit

```bash
git add src/lib/agents src/lib/stores/agentLibrary.svelte.ts src/lib/components/AgentLibrarySidebar.svelte src/lib/components/AgentSourceManager.svelte src/lib/components/AgentCreatorModal.svelte src/lib/components/AgentsWorkspace.svelte src/lib/api.ts src/lib/types.ts src/lib/i18n/locales/en.ts src/lib/i18n/messages.test.ts
git commit -m "feat: add agent source and creator workspace"
```

## Stage 1 Verification Gate

- [ ] `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `npm run verify:frontend`
- [ ] Manually verify local and GitHub source add/refresh/remove, duplicate names, nested folders, invalid draft retention, publish conflict, and built-in read-only behavior.
- [ ] Confirm `git diff --check` is clean.
- [ ] Present the unified diff and QA evidence for approval before any commit or Stage 2 work.
