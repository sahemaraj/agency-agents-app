# MCP Skills Platform Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the local-first MCP workflow so Claude Code and Codex can discover, inspect, install, maintain, recommend, and safely contribute skills through the Agency Agents app, with optional authenticated loopback HTTP access.

**Architecture:** Extend the existing Rust skill validator, source registry, transactional installer, lifecycle ledger, settings, and Activity surfaces. One `SkillMcpServer` serves both stdio and Streamable HTTP; all mutation tools pass through a shared permission policy and durable audit boundary. Client connection management delegates configuration merges to the official Claude and Codex CLIs.

**Tech Stack:** Rust 1.97, Tauri 2, rmcp 3, Tokio, Svelte 5, TypeScript.

## Global Constraints

- Preserve the existing dirty `feat/skills-library` branch; do not revert or reformat unrelated work.
- Claude Code and Codex only.
- No telemetry, accounts, hosted service, embeddings, background daemon, or arbitrary executable execution.
- Read operations are enabled by default; source, install, and destructive MCP mutations are disabled until explicitly enabled in app settings.
- Never overwrite Foreign or Modified installs; retain existing backup-first lifecycle guarantees.
- Reject symlinks, reparse points, special files, path traversal, oversize files, and files outside validated package inventories.
- Generated skills enter an app-owned draft inbox and require human approval in the desktop app before publication.
- Remote HTTP is disabled by default, loopback-only, bearer-authenticated, and never accepts a token via CLI arguments, URL, logs, or `settings.json`.
- No documentation, commit, push, PR, or release until the final human approval gate.

---

### Task 1: Validated package access and MCP resources

**Files:**
- Modify: `src-tauri/src/skills/mod.rs`
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/types.rs`

**Interfaces:**
- Produces: `resolve_skill_package(state, source_id, relative_path) -> ResolvedSkillPackage`
- Produces: `list_skill_files(...) -> Vec<SkillPackageFile>`
- Produces: `read_skill_file(..., file_path) -> SkillFileContent`
- Produces MCP tools: `skills_list_files`, `skills_get_file`
- Produces MCP resources: `skills://catalog`, `skills://packages/{source_id}/{relative_path}/{file_path}`

- [ ] Write failing tests proving package resolution rejects invalid packages, unlisted files, links, traversal, and oversize reads.
- [ ] Run focused tests and confirm missing resolver/file APIs cause failure.
- [ ] Extract one resolver from existing install/read flows; canonicalize source/package/file and verify exact inventory membership.
- [ ] Return UTF-8 as text and other bytes as base64 with MIME `application/octet-stream`; enforce the existing per-file cap.
- [ ] Add resource listing/templates/read handlers backed by the same resolver.
- [ ] Run focused tests and an MCP initialize → resources/list → resources/read exchange.

### Task 2: MCP install and lifecycle workflow

**Files:**
- Modify: `src-tauri/src/skills/mod.rs`
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/skills/install.rs`

**Interfaces:**
- Produces core functions accepting `&AppState`: `install_skill`, `reconcile_skill_installs`, `update_skill`, `disable_skill`, `enable_skill`, `uninstall_skill`
- Produces MCP tools: `skills_installed`, `skills_install`, `skills_update`, `skills_disable`, `skills_enable`, `skills_uninstall`

- [ ] Write failing tests for exact runtime/scope/project targeting and lifecycle state transitions through core functions.
- [ ] Run focused tests and confirm command-only wrappers cannot satisfy MCP callers.
- [ ] Extract Tauri-independent core functions; keep existing commands as thin adapters.
- [ ] Add MCP parameter schemas with exact `source_id`, `relative_path`, `runtime`, and optional canonical project path.
- [ ] Reuse existing ledger locks, hashes, backup, rollback, Foreign/Modified protection, and linked-ancestor checks.
- [ ] Run lifecycle tests plus a real MCP call against a temporary source and destination.

### Task 3: Permission policy, durable audit, and compound workflows

**Files:**
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/stores/activity.svelte.ts`
- Modify: `src/lib/components/ActivityHistory.svelte`

**Interfaces:**
- Produces settings: `mcp_source_access`, `mcp_install_access`, `mcp_destructive_access`, `mcp_project_allowlist`
- Produces: `authorize_mcp(policy, action, project_path) -> Result<(), AppError>`
- Produces: `append_mcp_audit(app_data_dir, entry)` and `mcp_audit_list`
- Produces MCP tool: `skills_find_and_install`

- [ ] Write failing tests for default-denied mutation classes, allowlisted project paths, and read-only defaults.
- [ ] Run focused tests and confirm no policy exists.
- [ ] Add settings fields with backward-compatible serde defaults and clamps/deduplication.
- [ ] Add bounded append-only `state/mcp-audit.jsonl`, cross-process lock, capped field lengths, and secret-free entries.
- [ ] Wrap every MCP tool with policy and audit; record success/failure without skill contents or bearer tokens.
- [ ] Implement exact find-and-install: install only one exact normalized name match; return candidates for zero/ambiguous matches.
- [ ] Expose audit entries to Tauri and merge them into Activity without replacing its local journal.
- [ ] Run policy, audit, Activity type-check, and compound workflow tests.

### Task 4: Source lifecycle and deterministic recommendations

**Files:**
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/skills/mod.rs`

**Interfaces:**
- Produces MCP tools: `skills_remove_source`, `skills_refresh_all`, `skills_source_status`, `skills_recommend`
- Produces: `catalog_revision(results) -> sha256`
- Produces: `recommend_skills(results, task, languages, limit) -> Vec<SkillRecommendation>`

- [ ] Write failing table tests with hand-derived recommendation ordering and reasons.
- [ ] Run tests and confirm recommendation/source bulk functions are absent.
- [ ] Score normalized exact metadata tokens and requested language tokens; stable tie-break by name/source/path.
- [ ] Add remove, refresh-all, source-status, and catalog revision responses using existing source operations.
- [ ] Enable rmcp resource-list-changed notification when the serving connection supports it; retain revision polling fallback.
- [ ] Run deterministic recommendation and source lifecycle tests.

### Task 5: Managed skill draft inbox

**Files:**
- Create: `src-tauri/src/skills/drafts.rs`
- Modify: `src-tauri/src/skills/mod.rs`
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/stores/skillSources.svelte.ts`
- Modify: `src/lib/components/SkillsWorkspace.svelte`
- Modify: `src/lib/i18n/locales/en.ts`

**Interfaces:**
- Produces: `SkillDraft`, `SkillDraftFile`, `SkillDraftState`
- Produces MCP tools: `skills_submit_draft`, `skills_list_drafts`, `skills_get_draft`
- Produces Tauri commands: `skill_drafts_list`, `skill_draft_publish`, `skill_draft_reject`

- [ ] Write failing tests for normalized relative paths, duplicate rejection, caps, link/special-file impossibility, validation diagnostics, and atomic staging.
- [ ] Run focused tests and confirm the draft subsystem is absent.
- [ ] Store drafts under `app_data/skills/drafts/<uuid>/` with manifest, bounded files, deterministic tree hash, and no executable permission.
- [ ] Validate draft packages through the existing validator; MCP can submit/read but cannot publish or reject.
- [ ] Add a single app-owned local source for published drafts; publish by atomic rename only after explicit desktop action.
- [ ] Add accessible Draft inbox controls to the existing Skills workspace and log publish/reject actions.
- [ ] Run Rust tests, Svelte check, and production build.

### Task 6: Claude Code and Codex connection management

**Files:**
- Create: `src-tauri/src/commands/mcp_clients.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/components/Settings.svelte`
- Create: `src/lib/components/SettingsSectionMcp.svelte`
- Modify: `src/lib/i18n/locales/en.ts`

**Interfaces:**
- Produces: `McpClientStatus { client, installed, state, command, detail }`
- Produces commands: `mcp_clients_status`, `mcp_client_connect`, `mcp_client_disconnect`, `mcp_client_repair`

- [ ] Write failing tests for argv construction with spaces, stable executable detection, PATH/known-bin resolution, timeout/output caps, and exact/conflict/missing states.
- [ ] Run focused tests and confirm connection commands are absent.
- [ ] Resolve `current_exe()` and reject macOS App Translocation/ephemeral paths.
- [ ] Invoke `claude mcp ...` and `codex mcp ...` directly without a shell; query before mutation and cap execution at 10 seconds.
- [ ] Treat exact registration as idempotent, conflict as non-mutating, and Repair as explicit remove+add.
- [ ] Add Settings → MCP status cards with Connect, Disconnect, Repair, refresh, accessible state text, and manual command fallback.
- [ ] Run fake-client integration tests, Svelte check, and production build.

### Task 7: Authenticated loopback Streamable HTTP

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/skills/mcp.rs`

**Interfaces:**
- Produces CLI mode: `--mcp-http [--bind 127.0.0.1:PORT]`
- Produces endpoint: `POST /mcp`
- Consumes secret: `AGENCY_AGENTS_MCP_TOKEN`

- [ ] Write failing parser tests for default/explicit loopback and rejection of wildcard, hostname, and non-loopback binds.
- [ ] Write failing middleware tests for missing, malformed, short, and wrong bearer tokens.
- [ ] Run focused tests and confirm HTTP mode/auth are absent.
- [ ] Enable rmcp Streamable HTTP server transport, Axum listener, Tokio net/signal, and constant-time token comparison.
- [ ] Require a token of at least 43 bytes; compare `Authorization: Bearer` values without logging them.
- [ ] Retain rmcp Host/Origin validation and default body limits; serve only `/mcp`.
- [ ] Add graceful Ctrl-C cancellation and refuse all non-loopback addresses.
- [ ] Run ephemeral-port initialize/tools-list/auth/Host/Origin/shutdown integration tests.

### Task 8: Full verification and approval handoff

**Files:**
- Review all files above; do not create documentation or commits yet.

- [ ] Run `cargo test` and record totals/failures/ignored tests.
- [ ] Run focused live stdio MCP initialize, tools/list, resources/list, search, and read calls.
- [ ] Run authenticated HTTP initialize/tools-list against an ephemeral loopback port.
- [ ] Run `npm run check` and record errors/warnings.
- [ ] Run `npm run build`.
- [ ] Run formatting checks on touched Rust files and `git diff --check`.
- [ ] Review every MCP mutation against permissions, canonical paths, backup/rollback, audit redaction, network policy, and cross-process locks.
- [ ] Compare implemented MCP tool/resource list against Tasks 1–7 and close any gap through another TDD loop.
- [ ] Present diff, verification evidence, known warnings, and documentation plan at the human approval gate.
