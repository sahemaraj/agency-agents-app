## 1. Control-Center Persistence and Catalog Feed

- [ ] 1.1 Register and validate the bounded versioned control-center SQLite document and import/backup inventory.
- [ ] 1.2 Persist deterministic active-catalog snapshots and typed add/update/remove/unambiguous-rename batches in crash-safe order.
- [ ] 1.3 Expose bounded feed and stale/Retry state through the existing Catalog Settings surface.
- [ ] 1.4 Verify bounds, deterministic diffing, failed-refresh retention, source transitions, and zero recommendation output on failure.

## 2. Project Readiness and Catalog Subscriptions

- [ ] 2.1 Persist exact source-aware per-project baselines and one explicit subscription per eligible registered project.
- [ ] 2.2 Derive independent Agent, roster, Skill, instruction, MCP, and tool evidence with conservative overall precedence.
- [ ] 2.3 Persist recommendation sets before cursors and implement dismissal, supersession, and stale-reference blocking.
- [ ] 2.4 Route eligible recommendations into existing reviewed plans and block incompatible aggregate roster recommendations before opening.
- [ ] 2.5 Verify evaluation is bounded, local, read-only, source-aware, and incapable of destination mutation.

## 3. Unified Review and Recovery

- [ ] 3.1 Aggregate Agent, Skill, Expert change, Expert run, and Expert activation requests in Activity Review with independent loading/errors.
- [ ] 3.2 Delegate every Review action to the exact owning domain and keep catalog recommendations separately labelled and counted.
- [ ] 3.3 Aggregate Agent history, Skill backup/history, and verified storage backup/reveal in Recovery without hot database restore.
- [ ] 3.4 Verify keyboard mode controls, live announcements, exact return focus, partial failure, Retry, and revision-bound recovery behavior.

## 4. Playbook Library

- [ ] 4.1 Retain and index only bounded Markdown documents under fixed approved catalog roots.
- [ ] 4.2 Enforce normalized paths, no links/reparse points, UTF-8, extension, depth, count, and byte limits for list and read.
- [ ] 4.3 Add deterministic local search and inert preformatted-text display to the existing Runbooks surface.
- [ ] 4.4 Verify managed/clone parity, unsafe path rejection, markup inertness, and empty/error/stale/Retry states.

## 5. Security Posture Presets

- [ ] 5.1 Classify Strict, Local Development, and Custom from the complete global and client-override policy matrix.
- [ ] 5.2 Apply a preset through one serialized load-latest, persist-once, cache-after-success settings transaction.
- [ ] 5.3 Preserve unrelated settings and project allowlist while clearing conflicting client overrides.
- [ ] 5.4 Verify complete preview, no pre-Apply write, corrupt-settings failure, concurrent-write serialization, announcements, and focus.

## 6. Kimi, OpenClaw, and Antigravity Targets

- [ ] 6.1 Store and reconcile complete Kimi/OpenClaw artifact manifests with documented aggregate state precedence.
- [ ] 6.2 Route every multi-artifact lifecycle and recovery action through exact preflight, journal, byte verification, and rollback.
- [ ] 6.3 Block every OpenClaw executable boundary, including generic version probing, and report external registration/restart separately.
- [ ] 6.4 Verify exact upstream renderer parity, injected failure rollback, secondary-file drift, history, and lifecycle round trips.
- [ ] 6.5 Verify unchanged Antigravity production plan/install/reconcile/uninstall parity without adding a second implementation.

## 7. Aider and Windsurf Roster Targets

- [ ] 7.1 Persist bounded project-scoped roster records with deterministic ordered exact Agent references and artifact manifests.
- [ ] 7.2 Implement revision-bound plan/apply/reconcile/update/history/rollback/disable/enable/uninstall/recovery with foreign-file refusal.
- [ ] 7.3 Route or explicitly block roster targets at every generic per-Agent, Workspace Pack, readiness, team, and recommendation boundary.
- [ ] 7.4 Include roster truth in project inventory and prevent remove-only orphaning; uninstall verified rosters before remove-and-uninstall unregisters.
- [ ] 7.5 Verify caps, path authority, crash recovery, no-partial-write behavior, project registration requirements, and aggregate UI review.

## 8. Accessibility, Security, and Completion Evidence

- [ ] 8.1 Fix and verify async plan focus/live announcement, valid Settings tabs with roving keyboard navigation, and AA button contrast.
- [ ] 8.2 Verify the application shell, dense project controls, and Settings reflow at 375 CSS pixels without clipped inaccessible controls.
- [ ] 8.3 Run full frontend tests, Svelte diagnostics, production build, Rust tests, format, strict Clippy, and diff checks.
- [ ] 8.4 Run browser E2E/axe flows for all eight capabilities and record native-only checks or explicit unavailable evidence honestly.
- [ ] 8.5 Run npm dependency audit, available Rust dependency metadata/security checks, command-boundary audit, and strict OpenSpec validation.
- [ ] 8.6 Complete the implementation-plan checklist, Memory Bank completion record, progress update, and final independent integration audit.
