# Agents–Skills Feature Parity Design

**Status:** Approved design
**Date:** 2026-08-04
**Branch:** `feat/skills-library`

## Purpose

Give Agents every user-facing capability currently available to Skills while preserving the existing agent catalog, deterministic renderers, installations, and filesystem-safety guarantees.

Users must be able to create, import, edit, validate, organize, install, maintain, and manage custom agents through the desktop app or authorized MCP without using the terminal.

## Decisions Locked by the User

- Match every current Skills capability with an Agent equivalent.
- Keep built-in catalog agents read-only. Editing or duplicating one creates an independent custom agent.
- Support blank creation, duplication, local import, folder import, local sources, and GitHub sources.
- Support logical nested folders and folder-level operations. Nested folders do not imply agent execution or orchestration.
- Preserve unmanaged agents discovered on disk and offer non-destructive tracking.
- Support macOS, Windows, and Linux wherever a destination tool supports agents.
- Permit duplicate display names, but require unique source identities and collision-free installation paths.
- Never merge or replace conflicting sources silently.
- Save invalid or incomplete agents as drafts, but block publishing and installation until validation passes.
- Removing a source preserves installed agents and reports them as SourceUnavailable.
- Modified content is backup-first for update, rollback, and uninstall.
- MCP may create and inspect drafts. Publishing and destructive or overwriting mutations require desktop involvement, authorization, and durable audit history.
- Deliver through staged checkpoints, but do not call the overall feature complete until the full parity matrix passes.
- Do not add agent execution, orchestration, cloud sync, collaboration, or a public marketplace.

## Current Architecture

### Agents

Agents currently follow this path:

```text
single active corpus
  -> slug-keyed in-memory index
  -> deterministic per-tool renderer
  -> installs.json ledger
  -> Current / Outdated / Modified / Removed / Foreign reconciliation
```

The current model identifies an agent primarily by filename slug and top-level category. Recursive source discovery exists, but nested relative paths are discarded after discovery. This is insufficient for multiple sources or duplicate slugs.

### Skills

Skills currently follow this path:

```text
multiple registered sources
  -> bounded discovery and validation
  -> drafts and desktop publication
  -> folders, favorites, collections, smart folders, profiles
  -> dependency-aware install plans
  -> seven-state lifecycle and version history
  -> Skills MCP tools/resources
  -> policy, project capabilities, approvals, and audit
```

The implementation already supplies the behavior to reuse, but most domain objects and validation rules are named for multi-file Skill packages and cannot be applied directly to a single-file Agent source.

## Chosen Approach

Use staged vertical parity with a small shared library capability layer.

- Share behavior that is genuinely domain-neutral: references, folder-tree mutations, collections, profiles, approvals, source transactions, authorization, audit, atomic state persistence, and deployment-grid UI.
- Keep domain-specific adapters for Skill packages and Agent files: discovery, parsing, validation, metadata, rendering, destination resolution, permission analysis, and lifecycle orchestration.
- Preserve existing serialized Skills APIs and state. Shared extraction must not force a migration of the completed Skills platform.
- Preserve legacy Agent slugs as compatibility aliases while all new Agent operations use source-aware identity.

Rejected alternatives:

1. A big-bang generic rewrite would change Skills and Agents simultaneously and create unnecessary regression risk.
2. Copying Skills under Agents would duplicate security and lifecycle logic and guarantee drift.

## Delivery Decomposition

The work is divided into four specifications and implementation plans. Each stage produces independently testable software, but only Stage 4 may close the overall parity objective.

### Stage 1 — Agent Foundation

Deliver source-aware identity, sources, validation, drafts, creation, importing, nested organization, favorites, recent items, collections, smart folders, profiles, and compatibility migration.

### Stage 2 — Lifecycle Parity

Deliver dependencies, recommendations, install plans, collection operations, update policies, preferred sources, seven reconciliation states, backups, indexed history, rollback, disable, enable, and uninstall.

### Stage 3 — Agents MCP

Deliver Agent resources, 49 parity tools, separate default-denied Agent policy, desktop approvals, and durable audit integration through the existing stdio and authenticated HTTP transports.

### Stage 4 — Integration and Parity Verification

Complete workspace integration, localization, Activity, cross-platform validation, migration testing, security review, accessibility verification, and the final Skills-to-Agents parity audit.

## Domain Model

### AgentReference

Every non-legacy operation uses a source-aware reference:

```text
AgentReference
  sourceId: string
  relativePath: string
```

Rules:

- `sourceId` is stable for the life of a registered source.
- `relativePath` is normalized, slash-separated, relative, and case-collision checked.
- The pair is the canonical identity.
- Display name and slug are metadata, not identity.
- The built-in catalog has one stable logical source ID even when its physical source changes between bundled, managed, and user-clone modes.
- Legacy ledger rows without a source identity migrate to the built-in source when their slug resolves. Unresolvable legacy rows receive a synthetic legacy reference and remain removable and recoverable.

### AgentSource

Agent sources support these kinds:

- Built-in catalog: implicit, read-only, non-removable.
- Local folder: registered by canonical path; source files are never silently modified.
- GitHub repository: optional ref and subdirectory; refreshed through the existing network policy.
- Published custom source: app-owned, local, and populated only through approved draft publication.

Removing a registered source unregisters it only. It does not delete the original source, app-owned published content, or installed agents.

### AgentPackageResult

Each discovered Agent produces an inspection result containing:

- source identity and normalized relative path;
- name, description, slug, division, logical groups, and tags;
- optional version, channel, changelog, publisher, publisher key, and signature status;
- required and recommended Agent references or names;
- capability and permission disclosures derived from the prompt;
- source hash, frontmatter hash, body hash, quality score, and validation diagnostics;
- installation eligibility.

Existing upstream files remain valid. New optional metadata must be ignored safely by older parsers and must not change deterministic rendering unless the destination format already consumes that field.

### AgentDraft

An Agent draft contains one bounded Markdown source plus validation results and state:

- Pending;
- Published;
- Rejected.

Creation paths:

- blank Agent form;
- duplicate built-in or custom Agent;
- import one Agent file;
- import a folder as a source;
- MCP draft submission;
- edit-as-draft from any inspected source.

Publishing writes atomically into the app-owned custom source. A conflicting relative path fails and returns an explicit rename requirement. Publishing never modifies the originating source.

### AgentLibraryState

Agent organization state mirrors the Skills capability set while remaining separate from Skills state:

- nested folders and assignments;
- favorites and recent items;
- named collections;
- smart folders;
- workspace profiles;
- update policies;
- publisher trust;
- preferred sources;
- usage counters;
- approval inbox.

Folder limits initially match Skills: 256 folders, eight segments deep, 64 characters per segment, and case-insensitive uniqueness. Logical folders do not alter a source file's physical path or an installed destination.

### AgentInstallRecord

The existing install ledger is extended backward-compatibly with:

- source ID and relative path;
- disabled path;
- installed source and rendered hashes;
- version-history identity;
- enough stored provenance to uninstall or back up an Agent when its source is unavailable.

Existing `slug`, tool, scope, project path, destination, hashes, timestamp, and corpus version remain readable during migration.

## Identity and Collision Rules

- Two sources may contain the same display name or slug.
- Two items with the same `sourceId + relativePath` are one identity and cannot coexist.
- The UI shows source provenance whenever names collide.
- Preferred-source selection resolves name-based recommendations and find-and-install workflows; it never rewrites identity.
- Before installation, the renderer resolves every physical destination.
- If another managed or foreign Agent owns the same destination, installation is blocked with both conflicting identities shown.
- Existing foreign content is never adopted by overwriting. Tracking remains non-destructive.

## Shared Capability Layer

The shared layer owns only domain-neutral behavior:

- normalized source references;
- folder validation, creation, rename, move, recursive-safe deletion, and assignment;
- favorites, recent items, collections, profiles, and portable import/export;
- approval state transitions;
- atomic JSON persistence and write serialization;
- authorization classification and redacted audit records;
- source registration transaction primitives;
- generic deployment-grid presentation.

Skills and Agents each own their own smart-folder rule type because Skills filter on package type and trust while Agents filter on division, lifecycle, source, capabilities, and deployment state.

## Source and Draft Flow

```text
Add source or submit draft
  -> canonicalize identity and reject linked/reparse roots
  -> bounded discovery
  -> parse exact source bytes
  -> validate metadata, path, identity, size, and capabilities
  -> retain diagnostics even when invalid
  -> publish only after desktop approval and successful validation
  -> refresh Agent registry revision
  -> notify UI and MCP resource subscribers
```

Source refresh is transactional. A failed refresh preserves the last active generation and source registration.

## Validation and Trust

An Agent is installable only when:

- its path is normalized and beneath its registered source;
- the source root and file contain no followed links or reparse points;
- the file is valid UTF-8 and within the existing Agent size bound;
- frontmatter is well formed and contains a non-empty name;
- its canonical identity is unique within the source;
- its publisher signature is absent or valid according to the selected policy;
- required Agent dependencies resolve without ambiguity or cycles;
- no destination collision blocks the requested install.

Capability analysis reports prompt references to filesystem writes, network access, shell or external tools, credentials, and destructive operations. Agency Agents does not execute the prompt or any bundled script.

Exact-version trust is bound to source identity, source-tree hash, capability inventory, and publisher identity. Any relevant content or source change invalidates that approval.

## Organization Behavior

- Personal folders are logical and support nested create, rename, move, assignment, and safe deletion.
- Folder rename or move updates descendants, assignments, and profiles atomically.
- Non-recursive deletion fails when descendants or assignments exist.
- Recursive deletion removes organization references only; it never deletes Agent sources or installations.
- Collections are explicit AgentReference sets and support install, update, and uninstall plans.
- Smart folders save live filters rather than static membership.
- Profiles snapshot selected folders, collections, default tool/runtime, and optional project destination.
- Portable export/import is versioned and declares `contentKind: agents`; importing Skills organization into Agents is rejected.

## Lifecycle Design

### States

Agents use the same seven lifecycle states as Skills:

- Current;
- Outdated;
- Modified;
- Missing;
- Foreign;
- Disabled;
- SourceUnavailable.

The old Removed wire value is accepted during migration and normalized to Missing.

### Installation

- Re-resolve and revalidate the Agent immediately before installation.
- Resolve all destinations through the existing tool registry and deterministic renderer.
- Produce an install plan before any dependency or collection operation.
- Refuse foreign or conflicting destinations.
- Back up divergent managed content before replacement.
- Publish all destination writes atomically where the platform permits and roll back the full operation on failure.
- Persist the ledger only after successful publication.

### Update Policies

- Notify: require user confirmation before updating.
- AutoTrusted: allow automatic update only when source and publisher remain trusted and capability inventory does not broaden.
- Pin: keep the installed version until explicitly changed.
- ReviewScripts: for Agents, require review when capability or external-tool instructions change.

### Version History and Rollback

- Every managed replacement preserves a bounded indexed snapshot.
- History is keyed by AgentReference, tool, scope, and project.
- Rollback verifies the selected snapshot belongs to that installation.
- Modified current content is backed up before rollback.
- Project-scoped rollback stays beneath the authorized project capability.

### Disable and Enable

- Disable moves the exact managed file or directory to a same-parent hidden sibling and records `disabledPath`.
- Enable refuses an occupied destination.
- Both operations verify identity and content before and after the move.

### Uninstall

- Current canonical content may be removed without an additional backup because it is reproducible.
- Modified or replaced content is backed up before removal.
- Missing destinations remove only the matching ledger row.
- SourceUnavailable installations remain uninstallable using stored ledger provenance.
- Failure to preserve content aborts removal.

## Dependency and Collection Plans

- Required Agents form a directed acyclic graph.
- Missing, ambiguous, cyclic, invalid, or blocked dependencies appear as plan blockers.
- Recommended Agents appear as optional information and are never installed implicitly.
- Plans show processing order, destination, rendered file count, capability disclosures, warnings, blockers, and rollback availability.
- Batch installation is all-or-rollback for changes made by that batch.
- Batch update and uninstall use the same preflight and preservation rules as individual lifecycle actions.

## MCP Design

### Transport

Use the existing `agency-agents` MCP server, stdio transport, bearer-authenticated loopback HTTP transport, client connection management, resource subscriptions, and audit boundary.

### Resources

- `agents://catalog` — validated Agent catalog and revision.
- `agents://agents/~{sourceId}/~{relativePath}` — one canonical Agent source and inspection record.
- `agents://renders/~{sourceId}/~{relativePath}/~{tool}` — deterministic preview for one supported tool; read-only and bounded.

All URI segments use the existing strict percent-encoding approach. Invalid, non-normalized, oversized, linked, or unregistered paths fail closed.

### Tool Parity

The server exposes these Agent equivalents:

1. `agents_search`
2. `agents_get`
3. `agents_list_files`
4. `agents_get_file`
5. `agents_installed`
6. `agents_plan_install`
7. `agents_install`
8. `agents_install_with_dependencies`
9. `agents_find_and_install`
10. `agents_update`
11. `agents_disable`
12. `agents_enable`
13. `agents_uninstall`
14. `agents_list_sources`
15. `agents_add_local_source`
16. `agents_add_github_source`
17. `agents_refresh_source`
18. `agents_remove_source`
19. `agents_refresh_all`
20. `agents_source_status`
21. `agents_recommend`
22. `agents_submit_draft`
23. `agents_list_drafts`
24. `agents_get_draft`
25. `agents_create_draft`
26. `agents_edit_draft`
27. `agents_get_library`
28. `agents_get_insights`
29. `agents_list_folders`
30. `agents_create_folder`
31. `agents_rename_folder`
32. `agents_move_folder`
33. `agents_delete_folder`
34. `agents_assign_folder`
35. `agents_set_favorite`
36. `agents_save_collection`
37. `agents_save_smart_folder`
38. `agents_save_profile`
39. `agents_version_history`
40. `agents_delete_collection`
41. `agents_delete_smart_folder`
42. `agents_delete_profile`
43. `agents_set_update_policy`
44. `agents_request_rollback`
45. `agents_set_preferred_source`
46. `agents_request_publisher_trust`
47. `agents_request_batch_collection`
48. `agents_submit_approval`
49. `agents_list_approvals`

For a single-file Agent, `agents_list_files` returns its one canonical source entry. `agents_get_file` returns that bounded source. This preserves client workflow parity without pretending an Agent is a multi-file Skill package.

### Authorization

- Reads remain enabled by default.
- Agent source, install, and destructive classes have separate per-client settings and default to disabled.
- Existing Skills permissions do not imply Agent permissions.
- Project mutations require an exact canonical allowlisted project and capability-relative filesystem operations.
- Draft publication is desktop-only.
- MCP overwrite, update, rollback, source removal, folder deletion, collection deletion, profile deletion, and uninstall requests enter the desktop approval inbox rather than executing silently.
- Every attempt is audited before a terminal result is returned.

### Audit

Audit rows contain client, tool, action, phase, success, canonical project identity when applicable, and timestamp. They never contain prompt bodies, source file contents, credentials, bearer tokens, or publisher private material.

## Desktop Workspace

`AgentsWorkspace.svelte` remains the section orchestration owner but must not absorb the entire parity implementation. It is already near the existing 800-line workspace guideline.

Focused Agent components own:

- source management;
- library sidebar and nested folders;
- Agent list and collision provenance;
- detail, source, rendered preview, and security tabs;
- creator/editor draft flow;
- organizer flow;
- install-plan review;
- draft and approval inboxes.

Existing generic components remain authoritative:

- `DeploymentTargetGrid`;
- `Modal`;
- `DestructiveConfirm`;
- `Button`;
- `Input`;
- `EmptyState`;
- `LoadingState`.

All new visible copy enters the English locale baseline. Partial locales use the existing fallback mechanism. Keyboard navigation, focus restoration, accessible names, non-color state text, and reduced-motion behavior are required.

## Activity Integration

Activity records successful and failed Agent operations for:

- source add, refresh, and removal;
- draft create, edit, publish, and rejection;
- folder and collection mutations;
- install, update, disable, enable, rollback, tracking, and uninstall;
- batch operations;
- MCP approvals and terminal outcomes.

Durable MCP audit entries remain distinct from the clearable local Activity journal and cannot be erased by clearing local history.

## Persistence and Migration

### New state

Agent sources, drafts, organization, approvals, and version history use separate versioned JSON documents beneath the existing app-data state directory.

### Existing install ledger

Migration is load-time, idempotent, and atomic:

1. Read the current ledger without rewriting it.
2. Resolve legacy slugs against the built-in corpus index when possible.
3. Add source identity, relative path, and lifecycle defaults in memory.
4. Preserve unknown legacy rows with a synthetic legacy reference.
5. Write the migrated ledger to a temporary sibling.
6. Sync and atomically replace the original only after all rows validate.
7. Preserve the original ledger as a recoverable migration backup until the new ledger completes a successful reconciliation.

No migration step moves, rewrites, disables, or deletes an installed Agent.

## Error Handling

- Source registration failure leaves the source registry unchanged.
- Source refresh failure retains the last valid generation.
- Invalid drafts retain diagnostics and remain non-installable.
- Identity and destination conflicts stop before writes.
- Install-plan blockers stop the entire operation.
- Backup failure stops update, rollback, disable, or uninstall before content removal.
- Publication or ledger failure restores the prior managed destination and state.
- Audit failure prevents MCP mutation success from being reported.
- Corrupt policy or state files fail closed for mutation paths and surface a repair action.
- Cross-platform unsupported destinations render as unavailable rather than attempting fallback paths.

## Security Requirements

- Reject linked or reparse source roots and entries.
- Bound source count, discovered file count, file size, draft count, folder depth, named items, history, and audit storage.
- Normalize and validate all relative paths before filesystem access.
- Use capability-relative project mutations after canonical allowlist authorization.
- Never execute Agent prompt content or source-provided scripts.
- Never log source contents, credentials, tokens, or private signing material.
- Keep network access behind existing paranoid-mode and network policy gates.
- Bind trust to exact source and content identity.
- Preserve existing foreign and modified content by default.

## Testing Strategy

### Characterization

- Capture current catalog parsing, rendered hashes, destination paths, install ledger serialization, reconciliation, and MCP Skills behavior before shared extraction.

### Unit tests

- normalized AgentReference parsing and portable collision keys;
- duplicate names and duplicate relative paths;
- nested path preservation;
- legacy ledger migration, including unknown slugs;
- folder mutations and recursive safety;
- smart-folder filtering;
- dependency ordering, ambiguity, and cycle detection;
- publisher and exact-version trust invalidation;
- seven-state reconciliation;
- lifecycle backup, disable, enable, history, rollback, and uninstall;
- MCP policy separation and audit redaction.

### Integration tests

- source add, inspect, refresh, and removal;
- draft create, duplicate, edit, approve, publish, and reject;
- custom Agent install through every implemented renderer format;
- collection all-or-rollback behavior;
- source removal followed by SourceUnavailable reconciliation and uninstall;
- migration from a pre-feature ledger without disk mutation;
- stdio and HTTP MCP initialize, tools/list, resources/list, resources/read, read tools, denied mutations, approved mutations, and audit visibility.

### Cross-platform tests

- Windows separators, drive roots, case-insensitive collisions, and reparse points;
- macOS/Linux symlinks and same-parent disable moves;
- file-unit and directory-unit destinations;
- user and project scope for every tool registry entry that supports the scope.

### Frontend verification

- pure Agent library model tests;
- Svelte type checking;
- component contract tests for creator, organizer, approval, and install-plan flows;
- production build;
- keyboard-only and screen-reader state verification;
- native Tauri smoke test.

### Completion gate

- Rust formatting and Clippy with warnings denied;
- full Rust suite;
- frontend checks, tests, and build;
- security review;
- accessibility review;
- live stdio and HTTP MCP checks;
- migration rehearsal against a copied real ledger;
- Skills-to-Agents parity matrix with no unsupported capability except the approved non-goals.

## Reuse and New-File Justification

### Reuse

- Extend the existing Agent parser, renderer, ledger, reconciliation, project registry, tool registry, Activity store, and Agents workspace.
- Reuse the Skills source transaction, trust, organization, approval, policy, audit, and MCP transport patterns through narrow extracted interfaces.
- Reuse existing generic UI primitives and the shared deployment grid.

### Why focused new modules are required

- `corpus/mod.rs` models one upstream catalog and cannot also own multiple mutable custom sources without conflating two source-of-truth contracts.
- `skills/drafts.rs` validates multi-file `SKILL.md` packages and cannot validate a single Agent Markdown file directly.
- `skills/organize.rs` persists Skills identities and filters; Agents require separate persisted identities and domain filters even when mutation algorithms are shared.
- `install/mod.rs` is already responsible for rendering, destinations, ledger, and reconciliation; adding sources, drafts, folders, and MCP there would mix unrelated responsibilities.
- `skills/mcp.rs` is Skills-specific and already large. Agent tools should reuse its transport and policy boundary through focused modules rather than doubling that file.
- `AgentsWorkspace.svelte` is already close to the workspace-size guideline; focused components are necessary to prevent one unreviewable UI file.

The implementation plans must name each new file, its single responsibility, its consumed and produced interfaces, and the existing file it extends or models.

## Feature-Parity Matrix

| Capability | Skills | Agents target |
|---|---:|---:|
| Multiple local/GitHub sources | Yes | Required |
| Source refresh/status/removal | Yes | Required |
| Create/edit/import drafts | Yes | Required |
| Desktop draft publication | Yes | Required |
| Validation and provenance | Yes | Required |
| File/source inspection | Yes | Required |
| Types/groups/tags or domain equivalent | Yes | Required |
| Nested personal folders | Yes | Required |
| Favorites and recent items | Yes | Required |
| Collections | Yes | Required |
| Smart folders | Yes | Required |
| Workspace profiles | Yes | Required |
| Portable library export/import | Yes | Required |
| Dependencies and recommendations | Yes | Required |
| Duplicate/preferred source handling | Yes | Required |
| Permission/capability manifest | Yes | Required |
| Quality score and insights | Yes | Required |
| Publisher signature and trust | Yes | Required |
| Update policies | Yes | Required |
| Install plans | Yes | Required |
| Collection batch operations | Yes | Required |
| Seven lifecycle states | Yes | Required |
| Backup, history, and rollback | Yes | Required |
| Disable and enable | Yes | Required |
| User/project destinations | Yes | Required for all supported Agent tools |
| MCP tools | 49 | 49 Agent equivalents |
| MCP resources | Catalog/package | Catalog/Agent/render equivalents |
| Default-denied mutations | Yes | Required with separate Agent policy |
| Canonical project capabilities | Yes | Required |
| Desktop approval inbox | Yes | Required |
| Durable MCP audit | Yes | Required |
| Activity integration | Yes | Required |
| Cross-platform behavior | Yes | Required |

## Definition of Done

The feature is done only when a user can create, duplicate, import, inspect, validate, organize, install, update, disable, enable, roll back, and remove a custom Agent entirely through the desktop app or authorized MCP; existing catalog Agents and installations remain intact; every destructive path is recoverable; Agent mutations remain independently default-denied; and every row of the parity matrix passes on supported platforms.
