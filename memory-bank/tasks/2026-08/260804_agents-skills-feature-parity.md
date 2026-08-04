# 260804_agents-skills-feature-parity

## Objective

Bring Agents to the approved Skills feature-parity baseline across source management, personal
library organization, lifecycle operations, MCP access, desktop workflows, audit, and recovery.
Execution/orchestration, cloud sync, collaboration, and a public marketplace remain explicit
non-goals.

## Outcome

- Added exact Agent identity by `sourceId + relativePath`, multiple sources, validated one-file
  drafts, nested folders, favorites, recent items, collections, smart folders, profiles, trust,
  preferred variants, and update policies.
- Added the seven lifecycle states plus transactional install, update, disable, enable, uninstall,
  history, rollback, dependency planning, and batch planning.
- Extended the existing MCP server with exactly 49 Agent tools beside 49 Skills tools, Agent
  resources/subscriptions, a separate default-denied Agent policy, typed desktop approvals, and
  durable redacted audit.
- Completed the desktop source, create, organize, inspect, install, lifecycle, approval-inbox,
  Activity, localization, keyboard, modal, and accessibility flows.
- Hardened portable-path validation, total relative-path limits, inert prompt rendering, migration,
  transactional failure recovery, and exact external-Agent selection.

## Files Modified

- `src-tauri/src/agents/`, `src-tauri/src/library.rs`, `src-tauri/src/install/` — Agent domain,
  personal library, lifecycle, history, migration, and recovery.
- `src-tauri/src/skills/mcp.rs`, `src-tauri/src/commands/settings.rs`, `src-tauri/src/state.rs` —
  shared MCP composition, policy, approvals, audit, and transport state.
- `src-tauri/src/corpus/`, `src-tauri/src/render/mod.rs`, `src-tauri/src/types.rs` — source-aware
  parsing, deterministic rendering, and shared contracts.
- `src/lib/agents/`, `src/lib/stores/agentLibrary.svelte.ts`, `src/lib/components/Agent*.svelte` —
  source-aware Agent workspace, personal organization, lifecycle, and approvals.
- `src/lib/components/InstallModal.svelte`, `src/lib/stores/activity.svelte.ts`,
  `src/lib/i18n/locales/en.ts` — reused install surface, Activity integration, and localized UI.
- `tools/phase-c/phase-c.sh` — portable host/VM verification harness corrections.

## Patterns Applied

- Extended the ledger-plus-filesystem transaction model in
  `memory-bank/systemPatterns.md#Agent sources and personal library`.
- Reused the existing deterministic renderer and single MCP server instead of introducing parallel
  frameworks.
- Reused the shared install modal and Activity store; Agent-specific components exist only for
  cohesive Agent workflows.
- Preserved exact source references and backup-first atomic mutations established by
  `memory-bank/decisions.md#2026-08-04-agent-identity-is-source-id-plus-portable-relative-path` and
  `memory-bank/decisions.md#2026-08-04-agent-lifecycle-extends-the-existing-ledger-transaction-model`.

## Integration Points

- `src/lib/components/AgentsWorkspace.svelte` coordinates selection, layout, and focused Agent
  workflow components.
- `src/lib/api.ts` maps desktop calls to the Agent domain and the existing installation engine.
- `src-tauri/src/lib.rs` registers the Agent commands in the existing Tauri application.
- `src-tauri/src/skills/mcp.rs` composes Skills and Agent capabilities on the existing stdio and
  authenticated loopback HTTP transports.

## Feature-Parity Audit

| Capability | Result |
|---|---|
| Sources, exact identity, validation, drafts | PASS |
| Nested folders and personal organization | PASS |
| Seven lifecycle states and recoverable mutations | PASS |
| Dependencies, collections, batches, policies | PASS |
| Trust, preferred variants, approvals, audit | PASS |
| MCP tools, resources, subscriptions | PASS |
| Desktop workflows, Activity, i18n, accessibility | PASS |
| Migration, path security, inert prompt data, failure recovery | PASS |

## Verification

- Rust library: 453 passed, 0 failed, 2 intentional external-fixture ignores.
- Rust CLI: 2 passed, 0 failed; doc tests: 0 failed.
- Frontend: 0 check errors/warnings, 19 tests passed, production build passed.
- Quality: strict Clippy, Rust format, and `git diff --check` passed.
- Live MCP stdio: initialized; 98 unique tools, exactly 49 Skills + 49 Agents, no duplicates.
- Host Phase C: 6 passed, 0 failed, including the macOS release build.

## Environment Disclosures

- macOS host verification passed.
- The Ubuntu VM was absent, so no Linux VM build is claimed.
- The Windows 11 VM was reachable but had no repository share, Node, Rust, or Build Tools; no
  Windows build is claimed and no machine toolchains were installed.
- The optional upstream renderer checkout used by the older harness was absent; the repository's
  renderer parity tests remain green.

## Artifacts

- `docs/superpowers/specs/2026-08-04-agents-skills-feature-parity-design.md`
- `docs/superpowers/plans/2026-08-04-agent-foundation.md`
- `docs/superpowers/plans/2026-08-04-agent-lifecycle-parity.md`
- `docs/superpowers/plans/2026-08-04-agents-mcp.md`
- `docs/superpowers/plans/2026-08-04-agent-parity-integration.md`
- `.phase-c/runs/20260804T142713Z/phase-c-report.md` — successful host run.
- `.phase-c/runs/20260804T143029Z/phase-c-report.md` — full-matrix attempt and environment limits.

No push, pull request, release, or deployment was created.
