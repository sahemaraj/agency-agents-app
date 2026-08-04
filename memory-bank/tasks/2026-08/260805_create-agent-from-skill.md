# 260805_create-agent-from-skill

## Objective

Make creation of a specialized Agent from an existing validated Skill a first-class desktop and
MCP workflow while preserving exact source identity, editable drafts, and desktop-only publication.

## Outcome

- Added deterministic Skill-to-Agent generation from the exact Skill `sourceId + relativePath`.
- Added structured Agent `required-skills` metadata and surfaced it in Agent inspection.
- Reused the Agent creator modal for an editable desktop preview launched from Skill details.
- Added `agents_create_from_skill` and `agents_request_publish_draft`, bringing the Agent MCP
  inventory to 51 tools alongside the existing 49 Skills and 29 Expert tools.
- Routed MCP publication requests through the existing typed desktop approval inbox with a bound
  source-hash revision; stale requests are rejected.
- Revalidated the published Agent through the canonical source parser before reporting success.

## Files Modified

- `src-tauri/src/agents/drafts.rs` — shared generator, preview command, publication revalidation,
  and focused tests.
- `src-tauri/src/agents/mcp.rs`, `src-tauri/src/skills/mcp.rs` — two MCP tools, policy/audit
  classification, exact inventory, and live stdio coverage.
- `src-tauri/src/agents/organize.rs`, `src-tauri/src/types.rs` — typed draft-publication approval
  with revision binding and desktop execution.
- `src-tauri/src/corpus/parse.rs`, `src-tauri/src/agents/mod.rs` — `required-skills` parsing and
  validated Agent result metadata.
- `src/lib/components/SkillsWorkspace.svelte`, `AgentCreatorModal.svelte`,
  `AgentApprovalInbox.svelte`, and `AgentDetailTabs.svelte` — desktop creation, review, approval,
  and dependency display.
- `src/lib/api.ts`, `src/lib/types.ts`, `src/lib/agents/libraryModel.ts`, and English messages —
  typed desktop integration and approval presentation.

## Patterns Applied

- Extended the existing validated Agent draft pipeline instead of adding a second creator.
- Reused the existing typed approval inbox and default-denied Agent MCP source policy.
- Preserved exact source identity and hash-bound mutation rules from
  `memory-bank/systemPatterns.md#Agent sources and personal library`.
- Kept the generated Agent as a small wrapper that requires the Skill instead of copying Skill
  instructions and creating a second source of truth.

## Integration Points

- Skill detail → shared Agent preview → existing validated draft inbox → human publish.
- MCP `agents_create_from_skill` → shared generator → validated draft.
- MCP `agents_request_publish_draft` → typed pending approval → desktop execution → published
  Agent source → existing Agent search/get.

## Verification

- Rust library: 474 passed, 0 failed, 2 intentional external-fixture ignores.
- Rust CLI: 2 passed, 0 failed.
- Frontend: 0 check errors/warnings, 20 tests passed, production build passed.
- Quality: strict Clippy, Rust format, and `git diff --check` passed.
- Live MCP stdio: 129 tools — 49 Skills, 51 Agents, 29 Experts — with both new tools present.
- Live desktop: latest Tauri development app rebuilt and launched successfully.

## Deliberate Limit

The Agent declares its required Skill but does not automatically install that Skill across Agent
runtimes. Add cross-runtime installation only when Skills and Agents share equivalent runtime
support.

No push, pull request, release, or deployment was created.
