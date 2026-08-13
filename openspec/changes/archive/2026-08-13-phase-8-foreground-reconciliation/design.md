## Context

See `proposal.md` for motivation and `specs/foreground-reconciliation/spec.md` for the behavior contract. The app root already starts Agent reconciliation on mount, while the Skills workspace starts Skill reconciliation after loading its registered project paths. Agent reconciliation has a module-level single-flight promise; Skill reconciliation has a scope-aware in-flight queue. Both stores already preserve prior rows and expose error, attempt, and terminal state consumed by existing retry surfaces.

## Goals / Non-Goals

**Goals:**

- Make the root lifecycle request both Agent and Skill reconciliation on mount and after a debounced window-focus event.
- Reuse the current store guards, stale-data behavior, and visible retry controls.
- Keep all foreground work local and read-only.

**Non-Goals:**

- Refresh catalogs, Skill sources, GitHub state, tool versions, or other network-backed data.
- Add a background timer, Tauri focus plugin, persisted snapshot, new dependency, or new user-facing surface.
- Change reconciliation commands, installation schemas, or mutation behavior.

## Decisions

### Keep orchestration in the existing root layout

Add a small focus scheduler beside the existing mount reconciliation in `src/routes/+layout.svelte`. It calls `install.reconcile()` and `skillSources.reconcileInstalls()` with the project paths already held by the existing `projects` store. This extends the root-owned lifecycle pattern without introducing a service or helper used once.

Alternative considered: a new foreground-sync store or Tauri window plugin. Rejected because native browser `focus`, `setTimeout`, and cleanup cover the requirement with no dependency or cross-process state.

### Debounce only focus signals

Use one root-owned 250 ms timer. Each focus event resets it; cleanup clears it. Mount reconciliation remains immediate, preserving startup behavior. No polling or visibility listener is added because window focus is the required signal and the existing page already relies on it for foreground refresh behavior.

Alternative considered: periodic polling. Rejected because it performs unnecessary disk work while the app remains active and is explicitly unnecessary under the behavior contract.

### Reuse store-level concurrency and failure contracts

Do not add an orchestration lock. Agent calls share `reconcileInflight`; Skill calls share or queue by canonical project scope through the existing reconciliation state machine. If the Skills workspace discovers a newer project-path set while a root request runs, its existing latest-scope queue completes under one visible reconciling interval.

Alternative considered: a new global promise spanning both stores. Rejected because it would duplicate existing locks, couple independent failure states, and risk dropping the Skill store's latest-scope request.

### Preserve independent ledger outcomes

Root requests start both stores without awaiting one before starting the other. A failure in one ledger therefore cannot suppress the other scan. Existing store logic retains rows and existing Agent and Skill warning/Retry surfaces report the affected failure.

Alternative considered: fail-fast sequential orchestration. Rejected because one local scan failure would leave the other ledger needlessly stale.

## Risks / Trade-offs

- [The root may initially scan Skills with an empty project list before the Skills workspace loads registered projects] → The existing scope-aware Skill queue accepts the later canonical path set without presenting multiple loading cycles.
- [A focus event can arrive immediately after a fast mount scan completes] → The 250 ms debounce limits event bursts; a later completed scan is a valid refresh rather than concurrent duplicate work.
- [Agent and Skill scans finish at different times] → Preserve separate store state and existing ledger-specific warnings instead of hiding one result behind a combined status.

## Migration Plan

1. Add the root focus scheduler and Skill reconciliation request without changing backend commands or persisted data.
2. Verify debounce, mount overlap, local-command allowlist, stale-row retention, and cleanup with the existing Vitest harness.
3. Run frontend checks, tests, and production build; run the existing Rust suite because IPC contracts remain part of the regression surface.
4. Roll back by removing the root focus listener and Skill root call; no data migration is required.
