# Codebase Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise repository readiness by adding frontend tests and complete quality gates, decomposing the largest frontend and Rust modules without changing behavior, removing inherited dead code, reducing initial bundle size, and restoring documentation/tooling consistency.

**Architecture:** Preserve the existing Tauri command surface, serialized DTOs, Svelte stores, and transactional Rust implementation. Extract cohesive modules behind the same public interfaces, characterize frontend behavior before moving it, and keep every batch independently green.

**Tech Stack:** Rust 2021, Tauri 2, Svelte 5 runes, SvelteKit 2, Vite 6, Vitest 4, jsdom, GitHub Actions.

## Global Constraints

- Preserve every registered Tauri command name and DTO wire shape.
- Preserve the existing default-denied MCP mutation policy and capability-based filesystem boundaries.
- Do not rewrite the app or introduce speculative behavior.
- Use failing tests before behavioral production changes; use the existing green suite as the safety net for pure moves.
- Work only in the isolated `chore/codebase-hardening` worktree.
- Commit each independently verified batch atomically.
- Do not update Memory Bank completion documentation until the final human approval gate.

---

### Task 1: Establish the clean baseline and verification contract

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `vite.config.js`
- Modify: `.github/workflows/linux-build.yml`
- Modify: `.github/workflows/windows-build.yml`
- Create: `.github/workflows/macos-verify.yml`

**Interfaces:**
- Produces: `npm run test:frontend`, `npm run verify:frontend`, and CI gates used by all later tasks.

- [ ] Install the official Svelte/Vite unit and component test stack.
- [ ] Configure Vitest with a DOM environment and browser package conditions.
- [ ] Add frontend test and verification scripts.
- [ ] Add one intentionally failing smoke test and run it to prove the harness is active.
- [ ] Replace the smoke assertion with the minimal passing assertion and rerun it.
- [ ] Apply the patched `cookie >=0.7.0` transitive override and regenerate the lockfile.
- [ ] Extend Linux CI with Rustfmt, Clippy, frontend tests, npm audit, and Cargo audit.
- [ ] Extend Windows CI with frontend tests.
- [ ] Add macOS verification using the repository's `TAURI_CONFIG` contract.
- [ ] Run frontend check, tests, build, audit, Rust formatting, Clippy, and full Rust tests.
- [ ] Commit the verified quality-gate batch.

### Task 2: Characterize and extract the Skills library model

**Files:**
- Create: `src/lib/skills/libraryModel.ts`
- Create: `src/lib/skills/libraryModel.test.ts`
- Modify: `src/lib/components/SkillsWorkspace.svelte`

**Interfaces:**
- Produces: pure functions for package filtering, sorting, taxonomy grouping, smart-folder matching, duplicate detection, and library metrics.
- Consumes: existing `SkillPackageResult`, `SkillSource`, and organization DTOs from `src/lib/types.ts`.

- [ ] Write failing tests for each extracted behavior using realistic typed fixtures.
- [ ] Verify failures are caused by the missing model API.
- [ ] Move only the corresponding pure logic into `libraryModel.ts`.
- [ ] Update `SkillsWorkspace.svelte` to consume the pure functions.
- [ ] Run the focused tests, Svelte check, and production build.
- [ ] Commit the model extraction.

### Task 3: Decompose the Skills workspace UI

**Files:**
- Modify: `src/lib/components/SkillsWorkspace.svelte`
- Create: `src/lib/components/skills/SkillSourceManager.svelte`
- Create: `src/lib/components/skills/SkillLibrarySidebar.svelte`
- Create: `src/lib/components/skills/SkillPackageList.svelte`
- Create: `src/lib/components/skills/SkillDetailPanel.svelte`
- Create: `src/lib/components/skills/SkillCreatorModal.svelte`
- Create: `src/lib/components/skills/SkillOrganizerModal.svelte`
- Create: `src/lib/components/skills/SkillFolderModal.svelte`
- Create: `src/lib/components/skills/SkillInstallPlanModal.svelte`
- Create: focused component tests where state or event contracts are non-trivial.

**Interfaces:**
- `SkillsWorkspace.svelte` remains the orchestration owner.
- Children receive typed props and emit callback props; they do not duplicate store ownership.
- Existing generic `Modal`, `DeploymentTargetGrid`, `DestructiveConfirm`, `Button`, and `Input` components remain the UI primitives.

- [ ] Add failing component contract tests before extracting each stateful section.
- [ ] Extract one cohesive section at a time.
- [ ] Run focused tests and `npm run check` after every extraction.
- [ ] Keep the workspace orchestration shell below 800 lines unless an evidence-backed exception is documented.
- [ ] Run full frontend verification and commit.

### Task 4: Split the Rust Skills domain facade

**Files:**
- Modify: `src-tauri/src/skills/mod.rs`
- Create: `src-tauri/src/skills/source.rs`
- Create: `src-tauri/src/skills/package.rs`
- Create: `src-tauri/src/skills/trust.rs`
- Create: `src-tauri/src/skills/operations.rs`
- Reuse: `src-tauri/src/skills/install.rs`
- Reuse: `src-tauri/src/skills/drafts.rs`
- Reuse: `src-tauri/src/skills/organize.rs`

**Interfaces:**
- `skills/mod.rs` continues to expose all Tauri commands registered by `src-tauri/src/lib.rs`.
- Existing command signatures and serialized types remain unchanged.
- `install.rs` remains the low-level transactional filesystem implementation.

- [ ] Capture the current focused Rust test results.
- [ ] Move source registration and Git refresh logic into `source.rs`.
- [ ] Move discovery, validation, inventory, and bounded file access into `package.rs`.
- [ ] Move signature and trust-record logic into `trust.rs`.
- [ ] Move high-level install/update/disable/enable/uninstall/reconcile orchestration into `operations.rs`.
- [ ] Run focused tests after each move and the full Rust suite after the facade is complete.
- [ ] Run Rustfmt and Clippy with warnings denied.
- [ ] Commit the domain split.

### Task 5: Split the MCP server responsibilities

**Files:**
- Modify: `src-tauri/src/skills/mcp.rs`
- Create: `src-tauri/src/skills/mcp/transport.rs`
- Create: `src-tauri/src/skills/mcp/resources.rs`
- Create: `src-tauri/src/skills/mcp/recommend.rs`
- Create: `src-tauri/src/skills/mcp/tools.rs`

**Interfaces:**
- `skills::mcp::serve` and `skills::mcp::serve_http` retain their signatures.
- Resource URI formats, tool names, audit semantics, authorization order, and resource-change notifications remain byte/behavior compatible.

- [ ] Capture the current MCP-focused test results.
- [ ] Move bearer/HTTP/stdio transport code into `transport.rs`.
- [ ] Move resource URI/list/read behavior into `resources.rs`.
- [ ] Move search/recommendation/tokenization behavior into `recommend.rs`.
- [ ] Move mutation classification and dispatch into `tools.rs`.
- [ ] Run MCP-focused tests after every move.
- [ ] Run the full Rust suite, Rustfmt, and Clippy.
- [ ] Commit the MCP split.

### Task 6: Reduce initial frontend bundle size

**Files:**
- Modify: `src/routes/+page.svelte`
- Create: `src/routes/pageLoading.test.ts` only if loading-map logic is extracted.

**Interfaces:**
- Sidebar section identifiers and navigation behavior remain unchanged.
- Major section components are loaded through dynamic `import()` only when selected.

- [ ] Add a failing test for any extracted section-to-loader map.
- [ ] Replace static imports for major workspaces with Svelte 5 supported lazy imports.
- [ ] Keep shell-critical overlays and titlebar components eager.
- [ ] Build and record chunk sizes.
- [ ] Verify no generated JavaScript chunk exceeds the existing 500 kB warning threshold.
- [ ] Commit the performance change.

### Task 7: Remove inherited dead code and reconcile documentation

**Files:**
- Delete: `src-tauri/tests/integration_brew.rs`
- Delete: obsolete `src-tauri/tests/fixtures/brew_*` and `trending_30d.json`
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src/lib/types.ts`
- Modify: frontend settings consumers as required by compiler evidence
- Modify: `package.json`
- Modify: `README.md`
- Modify: `docs/BUILD.md`
- Modify: `.github/workflows/linux-build.yml`
- Modify: `.github/workflows/windows-build.yml`

**Interfaces:**
- Old settings JSON remains readable because Serde ignores removed unknown fields.
- Active settings and MCP policy fields remain unchanged.

- [ ] Write a failing Rust test proving legacy settings JSON still loads after the fields are removed.
- [ ] Remove unused legacy cask/trending/enrichment/vulnerability settings from Rust and TypeScript.
- [ ] Verify old JSON is accepted and rewritten without resurrecting removed fields.
- [ ] Delete the ignored Homebrew-only integration suite and fixtures.
- [ ] Reconcile “210 personas” versus “251 repository files” wording.
- [ ] Replace obsolete v0.1 workflow/release comments with current behavior.
- [ ] Run all verification and commit.

### Task 8: Final integration verification

**Files:**
- Modify only files required by evidence from the final checks.

**Interfaces:**
- No public interface changes beyond removal of unused legacy settings fields.

- [ ] Run `npm run verify:frontend`.
- [ ] Run the production build and verify bundle warnings.
- [ ] Run `cargo fmt --check`.
- [ ] Run Clippy with warnings denied.
- [ ] Run the complete Rust suite and record pass/ignore counts.
- [ ] Run npm and Cargo dependency audits.
- [ ] Compare Tauri command registration and DTO serialization against the baseline.
- [ ] Re-index CodeGraph and confirm the new modules are present.
- [ ] Confirm the worktree is clean.
- [ ] Present diff, security review, QA evidence, and integration choices at the human approval gate.
