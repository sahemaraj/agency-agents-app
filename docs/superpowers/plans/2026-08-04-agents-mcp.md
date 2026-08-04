# Agents MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Do not begin implementation or commit without explicit user approval.

**Goal:** Add Agent resources and all 49 approved `agents_*` tools to the existing `agency-agents` MCP server with separate default-denied Agent mutation permissions, desktop approvals, and durable redacted audit records.

**Architecture:** Reuse the existing stdio/HTTP server, authentication, client identity, subscriptions, resource dispatcher, and audit boundary in `skills/mcp.rs`. Generate a second `ToolRouter<SkillMcpServer>` in `agents/mcp.rs` and merge it into the server's stored router using `rmcp 3.0.0`'s native `ToolRouter::merge`. The internal server type name remains unchanged to avoid a repository-wide rename.

**Tech Stack:** Rust 2021, `rmcp 3.0.0`, Tokio, Serde, Tauri 2 settings, existing Agent domain and lifecycle cores, Svelte 5 settings UI.

## Global Constraints

- Stages 1 and 2 must be accepted and green before starting this plan.
- Existing Skills tools, resources, authorization, and wire responses must remain unchanged.
- Reads are enabled by default. Agent source/install/destructive permissions are separate from Skills permissions and default to false globally and per client.
- Existing Skills grants never imply Agent grants.
- Project mutations require the existing exact canonical allowlist and capability-relative filesystem access.
- Draft publication is desktop-only.
- MCP overwrite, update, rollback, source removal, folder/collection/smart-folder/profile deletion, publisher trust, batch execution, and uninstall enter the desktop approval inbox rather than mutating immediately.
- Every tool attempt produces an `attempt` audit row and one terminal row. Mutation success is not returned if audit persistence fails.
- Audit entries never include prompt/source bytes, credentials, bearer tokens, private key material, or arbitrary request JSON.
- Do not add a second MCP server, port, token, connection registry, or audit log.
- Commit steps require separate explicit user approval.

## Task 1: Compose Skills and Agent Tool Routers Without Regression

**Files:**

- Create: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/agents/mod.rs`
- Modify: `src-tauri/src/skills/mcp.rs`

**Router shape:**

```rust
// skills/mcp.rs
#[tool_router(router = skills_tool_router)]
impl SkillMcpServer { /* existing skills_* tools */ }

// agents/mcp.rs
#[tool_router(router = agents_tool_router, vis = "pub(crate)")]
impl SkillMcpServer { /* agents_* tools */ }

// SkillMcpServer::new_with_client
let mut tool_router = Self::skills_tool_router();
tool_router.merge(Self::agents_tool_router());
```

### Steps

- [ ] Extend the Task 1 characterization test to assert 49 exact Skills names before composition.
- [ ] Rename only the generated Skills router function from `tool_router` to `skills_tool_router`.
- [ ] Add an initially empty `agents/mcp.rs` router named `agents_tool_router` and merge it in `new_with_client`.
- [ ] Expose narrow crate-visible `state()` and `run_tool(...)` methods on `SkillMcpServer`; do not make server fields public.
- [ ] Add one router test asserting no duplicate names and that all 49 Skills tools still exist after merge.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml skills::mcp::tests::tool
```

### Acceptance

- `tools/list` still contains the exact Skills tool set.
- Both tool families share one server object and one dispatch/audit path.
- No transport or handler implementation is duplicated.

### Conditional commit

```bash
git add src-tauri/src/agents/mcp.rs src-tauri/src/agents/mod.rs src-tauri/src/skills/mcp.rs
git commit -m "refactor: compose mcp tool routers"
```

## Task 2: Add Separate Agent MCP Authorization

**Files:**

- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/stores/settings.svelte.ts`
- Modify: `src/lib/components/SettingsSectionMcp.svelte`
- Modify: `src/lib/i18n/locales/en.ts`

**Settings additions:**

```rust
pub struct Settings {
    pub mcp_agent_source_access: bool,
    pub mcp_agent_install_access: bool,
    pub mcp_agent_destructive_access: bool,
}

pub struct McpClientPolicy {
    // existing Skills fields remain
    #[serde(default)] pub agent_source_access: bool,
    #[serde(default)] pub agent_install_access: bool,
    #[serde(default)] pub agent_destructive_access: bool,
}

pub enum McpAction {
    Read,
    Source,
    Install,
    Destructive,
    AgentSource,
    AgentInstall,
    AgentDestructive,
}
```

### Steps

- [ ] Add settings serialization/default/migration tests proving all three Agent permissions load as false from old settings files.
- [ ] Add authorization matrix tests for global and per-client Skills/Agent combinations, paranoid mode, user scope, allowed project, denied project, corrupt settings, and first launch.
- [ ] Extend `Settings`, patches, clamping/save logic, frontend defaults, and settings store with the three separate Agent toggles and per-client overrides.
- [ ] Add the three Agent action variants to the existing authorization function. Keep `Read` shared and always read-only.
- [ ] Extend MCP action classification so every `agents_*` tool is classified before dispatch. Unknown tools remain denied and audited.
- [ ] Render separate Skills and Agents permission groups in `SettingsSectionMcp.svelte`; toggling one group must not modify the other.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml commands::settings::
cargo test --manifest-path src-tauri/Cargo.toml state::tests::authorize_mcp
npm run check
npm run test:frontend
```

### Acceptance

- An existing user with all Skills MCP permissions enabled still has zero Agent mutation permissions after upgrade.
- Agent mutations fail closed on first launch, corrupt settings, paranoid mode, and non-allowlisted projects.
- Settings UI and Rust defaults match exactly.

### Conditional commit

```bash
git add src-tauri/src/commands/settings.rs src-tauri/src/state.rs src-tauri/src/types.rs src-tauri/src/skills/mcp.rs src/lib/types.ts src/lib/stores/settings.svelte.ts src/lib/components/SettingsSectionMcp.svelte src/lib/i18n/locales/en.ts
git commit -m "feat: separate agent mcp permissions"
```

## Task 3: Add Agent MCP Resources and Subscriptions

**Files:**

- Modify: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/skills/mcp.rs`

**Resources:**

```text
agents://catalog
agents://agents/~{sourceId}/~{relativePath}
agents://renders/~{sourceId}/~{relativePath}/~{tool}
```

### Steps

- [ ] Add URI round-trip tests for ASCII, spaces, Unicode, slashes inside encoded identities, malformed percent escapes, non-normalized paths, traversal, and unregistered sources.
- [ ] Reuse the existing strict URI-segment encoder/decoder by making only those helpers crate-visible.
- [ ] Implement `agents://catalog` from validated Agent source generations and expose the current aggregate revision.
- [ ] Implement exact Agent resource reads from `AgentReference`; never search by filename or slug.
- [ ] Implement deterministic bounded render previews through the existing renderer and tool registry. Reject unsupported tools without fallback.
- [ ] Delegate Agent resource list/read/subscribe/unsubscribe handling from the existing `ServerHandler` implementation to Agent resource helpers.
- [ ] Emit list-changed/resource-updated notifications after successful Agent source refresh and draft publication; failed refresh emits no revision change.
- [ ] Add stdio-handler unit tests and authenticated HTTP integration tests for list/read/subscribe and invalid URIs.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::resource
cargo test --manifest-path src-tauri/Cargo.toml skills::mcp::tests::resource
```

### Acceptance

- Agent and Skills resources coexist on both transports.
- Resource reads are bounded, source-aware, and read-only.
- A failed Agent source refresh does not invalidate the previous resource revision.

### Conditional commit

```bash
git add src-tauri/src/agents/mcp.rs src-tauri/src/skills/mcp.rs
git commit -m "feat: expose agent mcp resources"
```

## Task 4: Add Read, Source, and Draft Tools

**Files:**

- Modify: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/skills/mcp.rs`

**Tools:**

```text
agents_search
agents_get
agents_list_files
agents_get_file
agents_installed
agents_list_sources
agents_add_local_source
agents_add_github_source
agents_refresh_source
agents_remove_source
agents_refresh_all
agents_source_status
agents_recommend
agents_submit_draft
agents_list_drafts
agents_get_draft
agents_create_draft
agents_edit_draft
agents_get_insights
```

### Steps

- [ ] Define bounded `#[serde(deny_unknown_fields)]` request DTOs alongside existing MCP request types.
- [ ] Implement search/get/list-files/get-file against exact inspected Agent results. A single-file Agent reports one canonical file entry.
- [ ] Implement installed/source/status/insight responses by calling Stage 1/2 core functions, not Tauri IPC wrappers.
- [ ] Implement recommendation using exact references and preferred-source rules. Return ambiguity explicitly.
- [ ] Allow local/GitHub source add and refresh only under `AgentSource`; route source removal to a pending desktop approval instead of immediate unregister.
- [ ] Allow draft submit/create/edit under `AgentSource`. List/get remain reads. Do not expose draft publish.
- [ ] Ensure request validation and authorization occur before filesystem/network work.
- [ ] Add one success, denied, invalid-input, and audit-persistence-failure test per action class; table-drive the read tools.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::read_tools
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::source_tools
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::draft_tools
```

### Acceptance

- All listed tools appear exactly once in `tools/list`.
- Draft creation is possible over MCP; publication is not.
- Removal requests are visible in the desktop approval inbox and do not mutate immediately.

### Conditional commit

```bash
git add src-tauri/src/agents/mcp.rs src-tauri/src/skills/mcp.rs
git commit -m "feat: add agent mcp catalog tools"
```

## Task 5: Add Library Organization Tools

**Files:**

- Modify: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/agents/organize.rs`

**Tools:**

```text
agents_get_library
agents_list_folders
agents_create_folder
agents_rename_folder
agents_move_folder
agents_delete_folder
agents_assign_folder
agents_set_favorite
agents_save_collection
agents_save_smart_folder
agents_save_profile
agents_delete_collection
agents_delete_smart_folder
agents_delete_profile
agents_set_update_policy
agents_set_preferred_source
agents_request_publisher_trust
agents_submit_approval
agents_list_approvals
```

### Steps

- [ ] Add request DTO and tool-name characterization tests for all 19 tools.
- [ ] Route reads and reversible additive mutations through the Agent organization core with `AgentSource` authorization.
- [ ] Route folder delete, named-item delete, update-policy change, and publisher-trust change into pending approvals with the correct Agent-specific approval action.
- [ ] Ensure an approval carries requesting client identity and bounded structured fields only; it must not retain arbitrary MCP request JSON.
- [ ] Implement `agents_submit_approval` as submission of one validated Agent approval action, not approval execution.
- [ ] Keep approval execution/rejection desktop-only through existing Tauri command patterns.
- [ ] Test duplicate submissions, invalid/stale references, folder limits, request ownership, and approval-state transitions.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::library_tools
cargo test --manifest-path src-tauri/Cargo.toml agents::organize::tests::approval
```

### Acceptance

- Every destructive or overwrite-capable organization action pauses in `Pending`.
- An MCP client cannot approve its own request through any exposed tool.
- Agent approval records remain separate from Skills records.

### Conditional commit

```bash
git add src-tauri/src/agents/mcp.rs src-tauri/src/agents/organize.rs
git commit -m "feat: add agent mcp library tools"
```

## Task 6: Add Lifecycle and Batch Tools

**Files:**

- Modify: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/agents/organize.rs`

**Tools:**

```text
agents_plan_install
agents_install
agents_install_with_dependencies
agents_find_and_install
agents_update
agents_disable
agents_enable
agents_uninstall
agents_version_history
agents_request_rollback
agents_request_batch_collection
```

### Steps

- [ ] Add table-driven request/auth/audit tests for all 11 tools in user and project scope.
- [ ] Return plans and version history as reads.
- [ ] Allow a clean non-conflicting install only under `AgentInstall`; route dependency/batch execution through desktop approval because it can replace multiple destinations.
- [ ] Make find-and-install require one exact normalized match or a valid preferred-source choice.
- [ ] Route update, rollback, uninstall, and any install that would overwrite/replace managed content into a pending approval.
- [ ] Disable/enable require `AgentDestructive`/`AgentInstall` respectively and use Stage 2 core functions; if policy requires review, submit approval instead.
- [ ] Build approval execution around exact plan identity and revision. If source, destination, capabilities, or plan revision changes before desktop approval, invalidate and require a new request.
- [ ] Audit approval submission and later desktop terminal outcome under the same approval ID without storing source content.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::lifecycle_tools
cargo test --manifest-path src-tauri/Cargo.toml agents::mcp::tests::approval_execution
```

### Acceptance

- No destructive or stale approved plan executes silently.
- Project paths are authorized before plan resolution and reused as capabilities during execution.
- All 49 approved Agent tools are now present exactly once.

### Conditional commit

```bash
git add src-tauri/src/agents/mcp.rs src-tauri/src/install/mod.rs src-tauri/src/agents/organize.rs
git commit -m "feat: complete agent mcp lifecycle tools"
```

## Task 7: Verify Durable Redacted Audit Behavior

**Files:**

- Modify: `src-tauri/src/skills/mcp.rs`
- Modify: `src-tauri/src/agents/mcp.rs`
- Modify: `src-tauri/src/state.rs`

### Steps

- [ ] Add an exhaustive test that maps every `skills_*` and `agents_*` tool to one action class; unclassified names must fail before dispatch.
- [ ] Add audit redaction fixtures containing prompt bodies, source content, bearer tokens, private-key-like strings, and credentials; assert none appear in serialized audit rows or log messages.
- [ ] Test exactly one `attempt` and one terminal audit row for success, validation failure, authorization denial, handler failure, approval submission, and unknown tool.
- [ ] Test that terminal audit failure prevents mutation success from being returned and emits a server-side error without request data.
- [ ] Confirm local Activity clearing does not delete the durable MCP audit file.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml skills::mcp::tests::audit
cargo test --manifest-path src-tauri/Cargo.toml state::tests::mcp_audit
```

### Acceptance

- Tool classification is exhaustive for 98 total Skills/Agent tools.
- Audit rows contain only ID, timestamp, client, tool, action, phase, success, and canonical project identity.
- MCP mutation success is never reported without durable terminal audit.

### Conditional commit

```bash
git add src-tauri/src/skills/mcp.rs src-tauri/src/agents/mcp.rs src-tauri/src/state.rs
git commit -m "test: harden agent mcp audit boundary"
```

## Stage 3 Verification Gate

- [ ] Start the stdio MCP server and verify initialize, `tools/list`, `resources/list`, `resources/read`, one allowed read, one denied Agent mutation, one approved mutation, and audit rows.
- [ ] Start bearer-authenticated loopback HTTP MCP and repeat the same checks, including invalid/missing bearer tokens.
- [ ] Assert exactly 49 `skills_*` and 49 `agents_*` tools with no duplicate names.
- [ ] Assert Agent permissions default off after loading an old settings document with Skills permissions on.
- [ ] `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `npm run verify:frontend`
- [ ] Confirm `git diff --check` is clean.
- [ ] Present live transport transcripts with secrets redacted, the exact tool/resource inventory, unified diff, and QA evidence for approval before any commit or Stage 4 work.
