## Context

See `proposal.md` for motivation and `specs/workspace-packs/spec.md` for behavior. The current Agentfile v1 path in `install/mod.rs` exports slugs plus tool and absolute project paths, while import mutates immediately and silently skips failures. The project already has exact Agent and Skill identities, complete mutation plans, source inspection, registered project truth, deterministic hashing, transactional per-artifact lifecycle operations, Expert activation rollback/recovery precedent, Teams file controls, and Phase 12 receipts.

## Goals / Non-Goals

**Goals:**

- Turn the existing file exchange path into a bounded exact-reference review-and-apply workflow.
- Reuse one backend trust boundary for parsing, validation, planning, revision binding, application, and recovery.
- Keep pack paths portable by binding logical project scope during import rather than serializing source-machine paths.
- Carry future instruction and MCP dependencies now without prematurely implementing their managers.

**Non-Goals:**

- Editing or adopting instruction files, discovering/configuring MCP servers, executing runbooks or Agents, remote sharing, marketplace installation, catalog refresh, or cross-device source transfer.
- Updating, repairing, overwriting, or deleting pre-existing managed or unmanaged content during pack application.
- Generalizing every existing lifecycle into a new orchestration framework.

## Decisions

### Extend Agentfile commands into a versioned Workspace Pack boundary

Keep the existing loadout integration point and add a distinct `workspacePack: 1` manifest using exact `sourceId` plus `relativePath` references. The reader accepts both Workspace Pack v1 and legacy `agentfile: 1`; the writer emits only Workspace Pack v1. Legacy slugs are resolved during planning, never during an immediate mutation loop.

Alternative considered: create a parallel pack service and leave Agentfile restore untouched. Rejected because it duplicates file parsing, install orchestration, Teams controls, and leaves the unsafe immediate restore path available.

### Export one logical scope at a time

The export command accepts either global scope or one canonical registered project. It includes only managed Agent and Skill rows in that selected scope, strips the concrete project path, sorts/deduplicates exact entries, and writes atomically. Project-scope import requires the user to bind the pack to one current registered project before planning.

Alternative considered: export all projects with their absolute paths. Rejected because it is not portable and leaks local directory structure. Anonymous multi-project aliases are deferred until users demonstrate a need beyond one workspace per pack.

### Keep declarative requirements passive in Phase 13

Runbook, instruction, and MCP requirements are bounded strings/records shown in review. Known runbooks may be identified locally, but instructions and MCP requirements remain explicitly unverified and unapplied. Their future managers can extend export and validation without changing the manifest envelope.

Alternative considered: configure MCP and compose instruction files while applying a pack. Rejected because those are the next two roadmap capabilities with separate trust, adoption, diff, and approval rules.

### Backend owns the complete plan and revision

The backend returns a typed pack document plus one plan containing Agent and Skill subplans, exact destinations, current/no-op states, declarative requirements, blockers, rollback scope, and a SHA-256 revision over the normalized plan. Apply reloads the file, rebinds the selected project, rebuilds the plan, and requires exact revision equality.

Alternative considered: parse JSON and compose individual plans in Svelte. Rejected because it would duplicate bounds and identity validation at an untrusted UI boundary and permit plan/apply drift.

### Apply only missing safe entries and roll back only this run

Current exact deployments are no-ops. Missing or absent safe destinations use existing exact Agent and Skill install operations. Outdated, modified, foreign, disabled, source-unavailable, ambiguous, unsupported, or collision states block the whole pack before writes. On failure, newly created skills and Agents are uninstalled in reverse order; pre-existing rows are never rollback targets.

The existing SQLite filesystem-operation journal records the normalized pack operation and its created exact identities. Recovery follows the Expert activation precedent so a prepared run can be aborted and an applied run can complete metadata/cleanup idempotently.

Alternative considered: continue after per-item failure. Rejected because a pack represents one reviewed workspace configuration; partial success is harder to reason about and unsafe to present as installed.

### Reuse Teams review and Activity evidence

Teams keeps its existing file buttons and hosts one inline modal state machine: choose scope/file, inspect, bind project when needed, review, apply, results. This avoids a new top-level route or component abstraction. Completion logs one existing receipt containing only attempted mutations and exact destinations, while the retained result also lists no-op items.

## Risks / Trade-offs

- [Legacy slug can be ambiguous across Agent sources] → Resolve only one exact installable match; otherwise block with no mutation.
- [Cross-domain rollback can itself fail] → Preserve the original and rollback errors, keep the durable operation record, reconcile both ledgers, and never claim success.
- [A source disappears between review and apply] → Fresh plan revision mismatch yields zero writes and a refreshed review.
- [Logical project scope loses the source machine's folder identity] → Require explicit current-project binding; this is the portability boundary, not a backup/restore format.
- [Passive instruction/MCP requirements may feel incomplete] → Label them as requirements only; Phases 14 and 15 own safe adoption/configuration.
- [Inline Teams state increases component size] → Prefer the existing surface for the MVP; extract only if a second caller appears.

## Migration Plan

1. Add Workspace Pack types, bounds, deterministic serialization, legacy parsing, plan, apply, and recovery tests behind the existing loadout commands.
2. Change export to Workspace Pack v1 and change restore to return a plan before any mutation; preserve legacy Agentfile v1 input.
3. Extend the existing Teams file surface and frontend store/types for scope selection, review, explicit approval, progress/results, and exact Activity handoff.
4. Verify fresh-state blocking, no-write planning, rollback, crash recovery, deterministic export, path privacy, and legacy compatibility.
5. If regression requires rollback, revert the UI to hide Workspace Pack actions while keeping legacy files untouched; no persisted application state or schema migration is introduced.
