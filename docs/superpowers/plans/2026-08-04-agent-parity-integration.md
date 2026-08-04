# Agent Parity Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Do not begin implementation or commit without explicit user approval.

**Goal:** Complete the desktop workflow, Activity integration, localization, accessibility, migration/security rehearsals, cross-platform verification, and evidence-backed Skills-to-Agents feature-parity audit.

**Architecture:** Keep `AgentsWorkspace.svelte` as the route/workspace coordinator. Move only the remaining cohesive flows that would push it beyond the existing workspace-size guideline into focused Agent components. Reuse the established stores, generic UI primitives, Agent domain APIs, lifecycle core, and MCP approval inbox; do not create a second UI framework or capability registry.

**Tech Stack:** Svelte 5, TypeScript 5.6, Vitest 4, Tauri 2, Rust 2021, existing i18n fallback, Activity journal, MCP audit, and native dialogs.

## Global Constraints

- Stages 1–3 must be accepted and green before starting this plan.
- “Complete” means every capability in the approved parity matrix is usable and verified, except the explicitly approved non-goals: execution/orchestration, cloud sync, collaboration, and public marketplace.
- Preserve the built-in Agent browsing/install path and Skills workspace behavior.
- Do not display duplicate-name actions without source provenance.
- Do not treat color, icon, hover, or animation as the only carrier of meaning.
- Keep local Activity clearable and durable MCP audit non-clearable.
- English is the authoritative baseline; existing locale fallback supplies untranslated additions.
- No release/milestone ID, public rollout, or marketplace work is part of this plan.
- Documentation and commits occur only after explicit final approval.

## Task 1: Complete the Agent Workspace Information Architecture

**Files:**

- Create: `src/lib/components/AgentDetailTabs.svelte`
- Create: `src/lib/components/AgentOrganizerModal.svelte`
- Create: `src/lib/components/AgentApprovalInbox.svelte`
- Modify: `src/lib/components/AgentsWorkspace.svelte`
- Modify: `src/lib/components/AgentLibrarySidebar.svelte`
- Modify: `src/lib/components/AgentSourceManager.svelte`
- Modify: `src/lib/components/AgentCreatorModal.svelte`
- Modify: `src/lib/components/AgentInstallPlanModal.svelte`
- Modify: `src/lib/stores/agentLibrary.svelte.ts`

### Steps

- [ ] Add component-contract tests for selection persistence, collision provenance, creator return focus, organizer mutations, approval decisions, and install-plan blocker behavior.
- [ ] Keep `AgentsWorkspace.svelte` responsible only for loading state, selected reference, layout mode, modal routing, and cross-component refresh events.
- [ ] Move Agent source/render/security detail tabs into `AgentDetailTabs.svelte`. Reuse `PersonaBody` for canonical source display and `DeploymentTargetGrid` for targets.
- [ ] Implement source tab, per-tool rendered preview, validation/quality diagnostics, publisher/trust status, capability disclosures, dependencies/recommendations, version history, and lifecycle actions.
- [ ] Implement organizer editing for favorites, nested folder assignment, collections, smart folders, profiles, update policy, and preferred source. All state is keyed by `sourceId + relativePath`.
- [ ] Implement the desktop approval inbox for source removal, destructive folder/named-item operations, update, rollback, publisher trust, batch collection, and uninstall. Show requesting client, structured action, current plan revision, and stale-plan state.
- [ ] Require the desktop to revalidate and execute approvals through backend approval commands; the frontend never converts a pending approval into a direct mutation call.
- [ ] Keep the source manager, creator, organizer, approval inbox, and install-plan modal as the only new Agent-specific UI modules. Do not split list rows or buttons into one-use components.
- [ ] Run:

```bash
npm run check
npm run test:frontend
```

### Acceptance

- Every Agent library/lifecycle/MCP approval flow is reachable from the Agents workspace.
- `AgentsWorkspace.svelte` remains orchestration-focused and does not duplicate child mutation logic.
- Built-in Agents remain read-only; duplicate/edit creates a new independent draft.

### Conditional commit

```bash
git add src/lib/components/AgentDetailTabs.svelte src/lib/components/AgentOrganizerModal.svelte src/lib/components/AgentApprovalInbox.svelte src/lib/components/AgentsWorkspace.svelte src/lib/components/AgentLibrarySidebar.svelte src/lib/components/AgentSourceManager.svelte src/lib/components/AgentCreatorModal.svelte src/lib/components/AgentInstallPlanModal.svelte src/lib/stores/agentLibrary.svelte.ts
git commit -m "feat: complete agent library workspace"
```

## Task 2: Integrate Agent Operations with Activity

**Files:**

- Modify: `src/lib/stores/activity.svelte.ts`
- Modify: `src/lib/components/ActivityHistory.svelte`
- Modify: `src/lib/stores/agentLibrary.svelte.ts`
- Modify: `src/lib/stores/install.svelte.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/i18n/locales/en.ts`

**Activity coverage:**

```text
source add / refresh / remove
draft create / edit / publish / reject
folder / collection / smart-folder / profile mutations
install / update / disable / enable / rollback / track / uninstall
dependency and collection batches
MCP approval submission / approval / rejection / stale invalidation / terminal outcome
```

### Steps

- [ ] Add Activity store tests covering success, failure, batch summaries, collision provenance, MCP client identity, and clear behavior.
- [ ] Extend `JournalEntry` action/subject unions only with values required by the coverage list; retain existing serialized entries.
- [ ] Add one shared Agent-operation logging helper in `agentLibrary.svelte.ts` that records reference, display name, source label, action, status, and a bounded user-safe error.
- [ ] Route lifecycle store events through the same helper rather than logging independently in components.
- [ ] Record one summary row plus failed item details for batch operations; do not flood Activity with every successful dependency when the summary is sufficient.
- [ ] Show Agent source provenance and MCP requesting client in `ActivityHistory.svelte` where applicable.
- [ ] Verify clearing Activity affects only the local journal and leaves the backend MCP audit untouched.
- [ ] Run:

```bash
npm run test:frontend -- --run src/lib/smoke.test.ts
npm run check
```

### Acceptance

- Every approved Agent operation produces a useful local Activity outcome.
- Errors are actionable but contain no prompt/source body, credentials, tokens, or private key data.
- Clearing Activity cannot erase compliance audit records.

### Conditional commit

```bash
git add src/lib/stores/activity.svelte.ts src/lib/components/ActivityHistory.svelte src/lib/stores/agentLibrary.svelte.ts src/lib/stores/install.svelte.ts src/lib/types.ts src/lib/i18n/locales/en.ts
git commit -m "feat: record agent library activity"
```

## Task 3: Finish Localization and Accessibility

**Files:**

- Modify: `src/lib/i18n/locales/en.ts`
- Modify: `src/lib/i18n/messages.test.ts`
- Modify: `src/lib/components/AgentsWorkspace.svelte`
- Modify: `src/lib/components/AgentLibrarySidebar.svelte`
- Modify: `src/lib/components/AgentSourceManager.svelte`
- Modify: `src/lib/components/AgentCreatorModal.svelte`
- Modify: `src/lib/components/AgentDetailTabs.svelte`
- Modify: `src/lib/components/AgentOrganizerModal.svelte`
- Modify: `src/lib/components/AgentInstallPlanModal.svelte`
- Modify: `src/lib/components/AgentApprovalInbox.svelte`

### Steps

- [ ] Extend the English baseline with every visible label, state, error, help text, confirmation, empty state, and accessible name introduced by Stages 1–4.
- [ ] Extend locale tests to assert every Agent workspace key resolves through the existing fallback for all current locales.
- [ ] Add component tests for keyboard-only folder navigation, tab navigation, modal focus trap/return, Escape behavior, destructive confirmation, approval controls, and disabled/blocker announcements.
- [ ] Use semantic buttons, headings, lists, tabs, and tree labels; avoid click handlers on non-interactive elements.
- [ ] Add visible text for lifecycle, trust, validation, and approval states; icons and colors remain supplementary.
- [ ] Ensure live regions announce source refresh, validation completion, mutation outcome, and stale approval without repeatedly announcing list rerenders.
- [ ] Respect existing reduced-motion styles and avoid adding required motion.
- [ ] Run:

```bash
npm run check
npm run test:frontend -- --run src/lib/i18n/messages.test.ts
npm run build
```

### Manual checks

- [ ] Complete source add, draft create, folder assignment, install plan, and approval flows using keyboard only.
- [ ] Verify VoiceOver on macOS or an equivalent screen reader announces controls, state changes, validation errors, and dialog titles.
- [ ] Verify 200% zoom/reflow at the minimum supported window size.
- [ ] Verify reduced-motion preference.

### Acceptance

- No Agent workspace copy bypasses i18n.
- All workflows remain operable without a pointer.
- State and errors are understandable without color.

### Conditional commit

```bash
git add src/lib/i18n/locales/en.ts src/lib/i18n/messages.test.ts src/lib/components/AgentsWorkspace.svelte src/lib/components/AgentLibrarySidebar.svelte src/lib/components/AgentSourceManager.svelte src/lib/components/AgentCreatorModal.svelte src/lib/components/AgentDetailTabs.svelte src/lib/components/AgentOrganizerModal.svelte src/lib/components/AgentInstallPlanModal.svelte src/lib/components/AgentApprovalInbox.svelte
git commit -m "fix: make agent parity flows accessible"
```

## Task 4: Rehearse Persistence and Failure Recovery

**Files:**

- Modify only failing implementation/tests in: `src-tauri/src/agents/mod.rs`
- Modify only failing implementation/tests in: `src-tauri/src/agents/drafts.rs`
- Modify only failing implementation/tests in: `src-tauri/src/agents/organize.rs`
- Modify only failing implementation/tests in: `src-tauri/src/install/mod.rs`
- Modify only failing implementation/tests in: `src-tauri/src/install/history.rs`
- Modify only failing implementation/tests in: `src-tauri/src/state.rs`

### Steps

- [ ] Create temporary-directory integration fixtures for old settings, old install ledger, empty Agent state, partially written temp files, corrupt Agent source/library/draft/history documents, and a source removed after install.
- [ ] Rehearse load-time migration twice and compare resulting bytes after the first run.
- [ ] Inject failure at each mutation boundary: source registry save, source activation, draft content write, draft index write, publish registry refresh, library save, backup write, destination publish, ledger save, history index save, audit append.
- [ ] Assert each failure leaves either the full prior state or the full new state, never a mixed state.
- [ ] Assert corrupt mutation policy/state fails closed and returns a repair-oriented typed error; read-only built-in browsing remains available where safe.
- [ ] Assert source unregister never deletes source content, app-published content, or installed destinations.
- [ ] Assert `SourceUnavailable` uninstall and rollback use stored provenance and preserve modified content.
- [ ] Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml agents::
cargo test --manifest-path src-tauri/Cargo.toml install::
cargo test --manifest-path src-tauri/Cargo.toml state::
```

### Acceptance

- All durable writes demonstrate rollback or atomic replacement under injected failure.
- Migration is idempotent and never mutates installed content.
- Source removal cannot strand an install without uninstall/recovery information.

### Conditional commit

```bash
git add src-tauri/src/agents src-tauri/src/install src-tauri/src/state.rs
git commit -m "test: verify agent state recovery"
```

## Task 5: Complete Security and Cross-Platform Validation

**Files:**

- Modify only code/tests implicated by failures in:
  - `src-tauri/src/library.rs`
  - `src-tauri/src/agents/`
  - `src-tauri/src/install/`
  - `src-tauri/src/skills/mcp.rs`
  - `src-tauri/src/state.rs`
  - `src-tauri/src/util/fs.rs`

### Security checks

- [ ] Reject symlink and Windows reparse source roots and entries.
- [ ] Reject absolute, parent, current-directory, backslash, NUL, non-normalized, oversized, and case-colliding relative paths.
- [ ] Verify local/GitHub source count, discovered file count, file size, draft count, folder/named-item count, history retention, and audit retention bounds.
- [ ] Verify project mutations stay beneath the exact authorized canonical root even during rename/disable/rollback.
- [ ] Verify Agent prompt content is never executed or interpreted as a script.
- [ ] Verify trust invalidates when source/content/publisher identity changes.
- [ ] Verify network operations obey paranoid mode and existing network policy.
- [ ] Verify foreign and modified content survives failed or rejected actions.

### Cross-platform checks

- [ ] Windows: separators, drive roots, case-insensitive collision key, UNC/custom tool roots, reparse points, file and directory destinations.
- [ ] macOS: symlinks, same-parent hidden disable sibling, user/project destinations, app-data atomic replacement.
- [ ] Linux: symlinks, same-parent hidden disable sibling, user/project destinations, file and directory units.
- [ ] Every tool registry entry: supported user scope and project scope render/destination tests; unsupported combinations return unavailable without fallback.

### Commands

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run verify:frontend
npm run build:phase-c
```

Use `npm run build:phase-c:full` only when the configured VM runners are available; absence of an optional VM is reported, not masked.

### Acceptance

- No open HIGH or CRITICAL security finding.
- All host-platform tests pass and available cross-platform runners pass.
- Unsupported platform/tool combinations fail closed and visibly.

### Conditional commit

```bash
git add src-tauri/src/library.rs src-tauri/src/agents src-tauri/src/install src-tauri/src/skills/mcp.rs src-tauri/src/state.rs src-tauri/src/util/fs.rs
git commit -m "test: harden agent parity across platforms"
```

## Task 6: Run the Exact Feature-Parity Audit

**Reference:** `docs/superpowers/specs/2026-08-04-agents-skills-feature-parity-design.md#Feature-Parity-Matrix`

### Evidence required for every row

- [ ] Multiple local/GitHub sources: desktop action, backend test, refresh rollback test.
- [ ] Source status/refresh/removal: desktop action, Agent MCP tools, source-unavailable reconciliation.
- [ ] Create/edit/import drafts: blank, duplicate, file import, folder-as-source, MCP submission, desktop publication.
- [ ] Nested folders: create, rename, move, assign, recursive/non-recursive delete, depth/collision limits.
- [ ] Favorites/recent/collections/smart folders/profiles: persistence, import/export, desktop controls, MCP tools.
- [ ] Validation/quality/security: invalid retention, install block, source/body hashes, capability disclosure, trust invalidation.
- [ ] Dependencies/recommendations: deterministic required plan, ambiguity/cycle blockers, recommended-only display.
- [ ] Seven lifecycle states: backend state table and visible desktop state/action.
- [ ] Update policies: Notify, AutoTrusted, Pin, ReviewScripts behavior and UI reason.
- [ ] History/rollback: identity-bound snapshots, modified-content backup, desktop request, MCP approval.
- [ ] Disable/enable: exact move, occupied destination refusal, desktop and MCP paths.
- [ ] Batch collection operations: plan, all-or-rollback install, guarded update/uninstall.
- [ ] Publisher trust/preferred sources: exact identity binding, collision resolution, approval controls.
- [ ] Approvals: separate Agent inbox, desktop execution, stale revision invalidation, terminal audit.
- [ ] MCP resources: catalog, exact Agent source, rendered preview on stdio and HTTP.
- [ ] MCP tools: exact 49-name list with authorization and audit coverage.
- [ ] Activity: all approved operation families, failure rows, local-clear/durable-audit separation.
- [ ] Cross-platform support: available runner evidence and explicit unsupported states.

### Final verification commands

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run verify:frontend
git diff --check
git status --short
```

### Completion rule

- [ ] Mark a matrix row `PASS` only with a test name/command plus a desktop or MCP integration point.
- [ ] Any missing row keeps the overall feature `INCOMPLETE`; do not substitute a workaround or waiver.
- [ ] Confirm the four approved non-goals remain absent and no speculative execution/cloud/marketplace scaffolding was added.
- [ ] Present the complete diff, QA output, security/accessibility results, migration rehearsal, transport checks, cross-platform evidence, and parity matrix to the user in the APPROVAL state.
- [ ] Wait for explicit approval before committing or writing Memory Bank task documentation.

## Task 7: Apply and Document Only After Final Approval

**Files:**

- Create after approval: `memory-bank/tasks/2026-08/040826_agents-skills-feature-parity.md`
- Modify after approval: `memory-bank/tasks/2026-08/README.md`
- Modify after approval if architecture changed: `memory-bank/systemPatterns.md`
- Modify after approval if a new durable decision was made: `memory-bank/decisions.md`
- Modify after approval: `memory-bank/activeContext.md`
- Modify after approval: `memory-bank/progress.md`
- Modify after approval if needed: `memory-bank/toc.md`

### Steps

- [ ] Apply the user-approved final diff to the sandbox feature branch.
- [ ] Run the smallest post-apply smoke suite: full Rust tests, frontend verification, and one stdio MCP initialize/tools-list check.
- [ ] Create the task document with objective, approved outcome, exact files, tests, security/accessibility evidence, migration evidence, parity matrix result, and non-goals.
- [ ] Update only Memory Bank files justified by actual implemented architecture and decisions; do not invent a release/milestone ID.
- [ ] Present final branch status and commit proposal.
- [ ] Commit only when the user separately authorizes the commit.

### Conditional commit

```bash
git add \
  src-tauri/src/agents src-tauri/src/library.rs src-tauri/src/lib.rs \
  src-tauri/src/types.rs src-tauri/src/state.rs src-tauri/src/corpus/mod.rs \
  src-tauri/src/corpus/parse.rs src-tauri/src/install/mod.rs \
  src-tauri/src/install/history.rs src-tauri/src/skills/mod.rs \
  src-tauri/src/skills/mcp.rs src-tauri/src/skills/organize.rs \
  src-tauri/src/commands/settings.rs src-tauri/src/util/fs.rs \
  src/lib/agents src/lib/stores/agentLibrary.svelte.ts \
  src/lib/stores/install.svelte.ts src/lib/stores/settings.svelte.ts \
  src/lib/stores/activity.svelte.ts src/lib/components/AgentsWorkspace.svelte \
  src/lib/components/AgentLibrarySidebar.svelte \
  src/lib/components/AgentSourceManager.svelte \
  src/lib/components/AgentCreatorModal.svelte \
  src/lib/components/AgentDetailTabs.svelte \
  src/lib/components/AgentOrganizerModal.svelte \
  src/lib/components/AgentInstallPlanModal.svelte \
  src/lib/components/AgentApprovalInbox.svelte \
  src/lib/components/DeploymentMatrix.svelte \
  src/lib/components/InstallModal.svelte \
  src/lib/components/ActivityHistory.svelte \
  src/lib/components/SettingsSectionMcp.svelte src/lib/api.ts src/lib/types.ts \
  src/lib/i18n/locales/en.ts src/lib/i18n/messages.test.ts \
  memory-bank/tasks/2026-08/040826_agents-skills-feature-parity.md \
  memory-bank/tasks/2026-08/README.md memory-bank/activeContext.md \
  memory-bank/progress.md
git commit -m "feat: bring agents to skills feature parity"
```

Add `memory-bank/systemPatterns.md`, `memory-bank/decisions.md`, or `memory-bank/toc.md` to that command only when the approved documentation diff actually changes them.
