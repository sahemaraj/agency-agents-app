# 260805_skill-publishing-mcp

## Objective

Complete revision-bound Skill draft publication through MCP and correct the Skills workspace
popover and filter-layout behavior.

## Outcome

- Added `skills_request_publish_draft` using the existing authenticated, default-denied MCP source
  policy and desktop approval inbox.
- Bound each publication approval to the pending draft's exact tree hash and returned stale
  approvals to Pending with an operation error.
- Reused the existing Agent and Expert publication paths; no duplicate endpoints were added.
- Accepted legitimate root-level Skill references while retaining source ID and path validation.
- Made Manage Sources and Approval Inbox close on outside clicks while preserving inside clicks.
- Kept the search and filter controls inside the fixed-width package-list column.
- Published the 59-file `primavera-p6-eppm-hybrid` package through the approved MCP-to-desktop
  workflow in 2.61 seconds.

## Files Modified

- `src-tauri/src/types.rs` — typed Skill draft-publication approval action.
- `src-tauri/src/skills/mcp.rs` — MCP request tool, policy classification, inventory, and stdio test.
- `src-tauri/src/skills/organize.rs` — revision validation, desktop execution, stale-request recovery,
  and root Skill reference validation.
- `src-tauri/src/library.rs` — reused source ID validation.
- `src/lib/types.ts` — frontend approval union.
- `src/lib/components/SkillsWorkspace.svelte` — approval label, outside-click dismissal, and filter
  width containment.
- `src/lib/smoke.test.ts` — real-component popover regression coverage.

## Patterns Applied

- Extended the existing draft publisher and typed desktop approval boundary.
- Reused the existing document-click popover behavior from the Agents workspace.
- Constrained the filter grid's internal track with `minmax(0, 1fr)` so intrinsic control widths
  cannot extend into the detail pane.
- Kept publication human-approved and revision-bound; MCP cannot bypass desktop review.
- Added no dependency or production abstraction.

## Integration Points

- MCP `skills_request_publish_draft` → pending typed approval → desktop execution → existing Skill
  draft publisher → published local source.
- Skills Manage Sources / Approval Inbox `<details>` → one document click listener → close only
  when the click target is outside the open popover.

## Verification

- Rust library: 477 passed, 0 failed, 2 intentional ignores.
- Frontend: 0 check errors/warnings, 21 tests passed, production build passed.
- Quality: strict Clippy, Rust format, and `git diff --check` passed.
- Live UI: inside clicks preserved each popover; outside clicks closed Manage Sources and Approval
  Inbox; opening one closed the other; search and filter controls stayed inside the package list.
- Live publication: approval `e8d8ac82-6a1a-44c9-a935-8c6337f747a9` completed and draft
  `81453ee6-4007-4bba-91a6-67656a71cf47` was re-read as Published at tree hash
  `f79e186866bd389345fa3546bd618b97436ca4d74a65e50e6dfc273fc3cbd197`.
- Published source: `527341c7-9ba0-4058-810b-6ad56e9f279b` at
  `/Users/home/Library/Application Support/com.zerologic.agency-agents-app/skills/published/primavera-p6-eppm-hybrid/SKILL.md`.

## Deliberate Limit

Publication still requires explicit desktop approval. No remote push, pull request, release, or
deployment was created.
