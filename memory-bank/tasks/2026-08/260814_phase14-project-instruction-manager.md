# 260814_phase14-project-instruction-manager

## Objective

Manage bounded app-owned snippets in known project instruction files with exact diff review, explicit revision-bound approval, byte-preserving adoption, verified backups, and honest recovery.

## Outcome

- Added local read-only inspection for `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and `.github/copilot-instructions.md` beneath exact canonical registered projects.
- Added strict versioned ownership markers that create, replace, or remove one named snippet while preserving all unowned bytes and exposing first insertion as adoption.
- Added complete deterministic current/proposed plans, no-op and blocker evidence, exact revisions, fresh apply-time replanning, and zero-write stale approval handling.
- Reused the existing private backup directory, atomic writer, project registry lock, and durable filesystem-operation journal for one-target apply and idempotent startup recovery.
- Extended the existing Projects detail surface with inspection, composition/removal, complete line diff, explicit apply, progress, retained results, accessible announcements, focus restoration, and redacted local Activity.

## Verification

- OpenSpec: 15/15 tasks complete; strict change validation passed; canonical specs validate 8/8 after sync.
- Frontend: 107/107 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend: 562 library tests discovered; 559 passed with 3 existing environment-gated ignores; 2/2 binary tests passed; 12/12 focused project-instruction tests passed.
- Rust quality: formatting and strict Clippy passed.
- Safety: allowlist, exact registration, path/link/special-file, byte/UTF-8, marker, count/content, credential, stale-revision, verified-backup, atomic-write, rollback, recovery-error retention, passive-content, no-network, no-execution, no-MCP-mutation, dependency, route, and diff audits passed.

## Integration Points

- `src-tauri/src/install/mod.rs` owns bounded inspection, marker parsing/composition, deterministic planning, revision-bound apply, verified backup, rollback, and recovery.
- `src-tauri/src/state.rs` and `src-tauri/src/state_db.rs` connect `project_instruction_apply` to the existing startup filesystem journal.
- `src/lib/stores/projects.svelte.ts` bridges inspect, plan, apply, and existing-format redacted Activity.
- `src/lib/components/Projects.svelte` hosts the inline inspect-compose-review-apply-result workflow and reuses the current line-diff utility.
- `openspec/specs/project-instructions/spec.md` is the canonical capability contract.

## Security and Safety

- Requests accept only four fixed target identities and exact registered canonical project roots; existing target components must be real directories/files, never links or reparse points.
- App-owned snippets are bounded and passive. Malformed/nested/duplicate markers, control characters, traversal identities, marker injection, oversized content, invalid UTF-8, and obvious credentials fail closed.
- Existing target bytes are revalidated, copied exactly to private storage, verified, and atomically replaced only after current-plan confirmation; drift produces no project or backup write.
- Startup recovery revalidates project and backup path identities, never follows unsafe target ancestors, and retains explicit journal errors when exact rollback cannot be proven.

## Artifacts

- Implementation commit: `ccb6408`
- Branch: `feat/phase-14-project-instructions`
- OpenSpec archive: `openspec/changes/archive/2026-08-14-phase-14-project-instruction-manager/`
