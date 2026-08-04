# 260804_agent-foundation

## Objective

Establish the source-aware Agent foundation required to bring Agents to full Skills feature parity
without conflating Agent content, lifecycle records, or organization state with Skills.

## Outcome

- ✅ Added canonical `sourceId + relativePath` Agent identity while preserving unambiguous legacy
  slug adapters.
- ✅ Added bounded built-in, local-folder, GitHub, and app-owned published sources with transactional
  refresh and non-destructive source removal.
- ✅ Added validated one-file drafts for blank creation, Markdown import, edit, duplicate, publish,
  and reject workflows.
- ✅ Added nested folders, assignments, favorites, recent items, collections, smart folders,
  profiles, update policies, publisher trust, preferred sources, usage, and approvals.
- ✅ Added source provenance, nested library navigation, source management, and draft creation to the
  Agents workspace.
- ✅ Rust: 414 passed, 0 failed, 1 environment-dependent parity test ignored; 2 CLI tests passed.
- ✅ Clippy with `-D warnings`, Rust formatting, Svelte check, 10 frontend tests, production build,
  and native Tauri smoke passed.

## Files Modified

- `src-tauri/src/agents/mod.rs` — source registry, discovery, exact reads, refresh, and commands.
- `src-tauri/src/agents/drafts.rs` — bounded draft store and transactional publication.
- `src-tauri/src/agents/organize.rs` — Agent personal-library persistence and mutations.
- `src-tauri/src/library.rs` — shared portable reference and nested-folder invariants.
- `src-tauri/src/corpus/{mod.rs,parse.rs}` — nested Agent retention and optional Agent metadata.
- `src-tauri/src/{types.rs,lib.rs}` — Agent DTOs and Tauri command registration.
- `src/lib/components/{AgentLibrarySidebar,AgentSourceManager,AgentCreatorModal}.svelte` — new Agent
  library surfaces following existing sidebar, modal, button, dialog, and focus patterns.
- `src/lib/components/AgentsWorkspace.svelte` — source-aware browsing and draft entry points.
- `src/lib/{api.ts,types.ts}` and `src/lib/stores/agentLibrary.svelte.ts` — exact desktop contracts
  and state.

## Patterns Applied

- Extended the existing Skills GitHub validation, network authorization, bounded Git runner,
  transactional activation, atomic-write, and UI primitive patterns.
- Extracted only portable reference/folder validation shared by Skills and Agents.
- Kept Agent sources, drafts, organization, and package metadata domain-specific.
- Used exact references at every mutation boundary; duplicate display names never become identity.

## Integration Points

- The built-in Agent source is seeded from the existing corpus and uses stable ID
  `builtin:agency-agents`.
- Local and GitHub sources resolve through the Agent source facade; local unregister never removes
  user-owned content.
- Draft publication creates content only in the app-owned published source, then refreshes the same
  source registry used by the library.
- The Agents workspace reads exact package references and avoids legacy slug deployment for external
  packages.

## Scope Boundaries

Lifecycle migration/reconciliation, backup/history/rollback, dependencies, collections, Agents MCP,
final integration, commits, pushes, PRs, and releases were not part of Stage 1.

## Artifacts

- Design: `docs/superpowers/specs/2026-08-04-agents-skills-feature-parity-design.md`
- Plan: `docs/superpowers/plans/2026-08-04-agent-foundation.md`
