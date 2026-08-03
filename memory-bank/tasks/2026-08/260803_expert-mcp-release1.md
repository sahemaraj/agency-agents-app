# 260803_expert-mcp-release1

## Objective

Deliver Release 1 of the Expert MCP lifecycle so Claude Code and Codex can discover Experts,
propose lifecycle changes, request activation, execute against an immutable quality contract,
submit evidence, report blockers, and request human review without receiving direct approval
authority.

## Outcome

- ✅ Added 27 canonical Expert lifecycle MCP tools plus 2 compatibility aliases.
- ✅ Unified create, update, clone, archive, and delete proposals in one versioned approval inbox.
- ✅ Added canonical-project validation, caller ownership, idempotency, bounded persistence, and
  rejection of credentials or project paths in portable Expert definitions.
- ✅ Activation now snapshots the Expert into a scoped run and returns a run ID in the copied
  starter prompt.
- ✅ Runs accept bounded idempotent evidence and blockers, use the latest evidence per check, and
  freeze after a terminal desktop review.
- ✅ Acceptance requires every missing/failed required check to have an explicit human waiver;
  waiver reasons remain desktop-private in MCP responses.
- ✅ Added Changes and Runs views with accessible tabs, quality-contract editing, evidence review,
  and human verdict controls.
- ✅ Tests: 411 passed, 0 failed, 1 environment-gated converter parity test ignored.
- ✅ Svelte check: 0 errors, 0 warnings.
- ✅ Production frontend build and native macOS Tauri debug build succeeded.
- ✅ Release-file formatting and diff integrity checks succeeded.

## Files Modified

- `src-tauri/src/experts.rs` — portable definitions, shared change requests, activation/run link,
  lifecycle validation, persistence, and regression tests.
- `src-tauri/src/expert_runs.rs` — separate capped run store, contracts, evidence, blockers,
  review states, waivers, and desktop commands.
- `src-tauri/src/skills/mcp.rs` — Expert MCP discovery, proposal, activation, and run tools routed
  through the existing policy/audit boundary.
- `src-tauri/src/lib.rs` — registered the Expert run module and desktop commands.
- `src/lib/types.ts` — Expert contract, request, activation, evidence, waiver, and run wire types.
- `src/lib/stores/experts.svelte.ts` — run loading and desktop review operations.
- `src/lib/components/Experts.svelte` — Changes/Runs interfaces and run-aware activation prompts.

## Patterns Applied

- Reused the existing MCP `run_tool` policy/audit boundary for every new tool.
- Extended the existing Expert proposal inbox instead of adding separate mutation queues.
- Reused canonical registered-project checks before project-scoped MCP operations.
- Kept portable Expert definitions separate from project-scoped execution state.
- Stored runs separately from Expert definitions because their retention, privacy, and terminal
  lifecycle differ.

## Integration Points

- Desktop Expert activation installs through the existing transactional agent/skill paths, then
  creates a run; a run-creation failure rolls back newly installed components.
- MCP callers receive only their own client/project-scoped requests and runs.
- Desktop review remains the sole authority for Accepted, Rework, Rejected, and Cancelled verdicts.
- The Experts store loads definitions, approval requests, activation history, and runs together.

## Security Review

- No credentials or absolute user/project paths may enter portable Expert proposals.
- MCP mutations remain default-denied unless enabled by the existing policy layer.
- Destructive deletion remains a pending desktop request and uses the destructive policy class.
- Evidence, blocker, proposal, and state files are bounded; writes are atomic and lock-serialized.
- Human waiver reasons are persisted locally but redacted from MCP views.

## Scope Boundaries

No automatic approvals, remote orchestration service, background scheduler, new dependency,
production release, push, or pull request was added. The pre-existing `Cargo.toml`
`macos-private-api` change and untracked `node_modules` were preserved and excluded from this task.
