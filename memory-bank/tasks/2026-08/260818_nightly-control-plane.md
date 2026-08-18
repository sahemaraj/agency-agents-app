# 260818_nightly-control-plane

## Objective

Build and validate one local-first control plane spanning Unified Review, Project Readiness,
Playbooks, Catalog Change Feed, project subscriptions, Recovery, security posture presets, and
exact Antigravity, Aider, Windsurf, OpenClaw, and Kimi target lifecycles without adding an Agent
runtime, telemetry, silent mutation, arbitrary shell execution, or a second approval engine.

## Outcome

- Added a bounded versioned SQLite control-center document for durable catalog snapshots, typed
  change batches, exact project baselines, explicit subscriptions, and recommendation cursors.
- Added source-aware project readiness across Agent, aggregate roster, Skill, instruction, MCP,
  and tool evidence with conservative overall precedence and reviewed workflow handoffs.
- Unified every existing approval shape in Activity while retaining the exact owning domain, and
  aggregated existing Agent, Skill, and storage recovery controls without hot database restore.
- Added a bounded plain-text Playbook library under fixed catalog roots with deterministic local
  search, normalized provenance, no-follow path containment, and inert markup display.
- Added complete Strict, Local Development, and Custom posture classification plus one serialized,
  previewed, persist-once settings transaction that clears conflicting client overrides.
- Added exact multi-artifact Kimi/OpenClaw lifecycle truth and project-scoped Aider/Windsurf roster
  lifecycle truth through the existing journal, history, rollback, verification, and recovery paths.
- Kept OpenClaw file installation distinct from external registration/restart and blocked every
  OpenClaw executable boundary, including generic version probing.
- Preserved the existing Antigravity implementation and proved parity instead of adding a duplicate.
- Closed final audit findings for passive evidence fail-closed behavior, managed/user-clone Playbook
  parity, roster-aware project removal, mobile navigation persistence, focus, contrast, reflow, and
  roster reconciliation failure recovery.

## Verification

- Frontend: 182/182 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; the static
  production build passed.
- Backend: 790/790 executed Rust library tests passed with 5 intentional environment-gated ignores;
  2/2 binary tests passed; documentation tests passed with no cases.
- Rust quality: strict Clippy with warnings denied, formatting, and a fresh development build passed.
- Upstream parity: 3/3 ignored parity suites passed against upstream commit
  `ebe9c99acb5c96f9468de368d8bead775387d1a7`; Kimi/OpenClaw transforms and multi-artifact output
  each completed 1,620 exact byte comparisons, and both Aider/Windsurf roster aggregates matched.
- Browser E2E: Review, Readiness, Playbooks, Catalog Feed, Recommendations, Recovery, Presets, and
  target planning passed at 375 and 1440 CSS pixels with no runtime/page errors, horizontal overflow,
  or axe violations. Mobile sidebar open/navigation/resize behavior preserved the desktop preference.
- Contrast evidence: official OKLab-to-sRGB calculations measured heading 18.10:1, muted text 6.54:1,
  body text 8.20:1, and warning text 5.93:1 against their rendered backgrounds.
- Dependencies and boundaries: `npm audit` reported 0 vulnerabilities; locked Cargo metadata parsed;
  RustSec audit tooling was unavailable and is not represented as green. Command-boundary inspection
  and tests found no OpenClaw execution path.
- OpenSpec: all 37 implementation tasks are complete; eight delta specifications were synced into
  canonical specs and strict validation passed before archival.
- Independent review: target/roster parity, backend/playbook parity, accessibility/reflow, and final
  roster-retry reviews all returned PASS after their material findings were repaired and retested.
- Native evidence: native backend lifecycle and filesystem boundaries are covered by the Rust suite
  and build. A fresh packaged Tauri GUI smoke was unavailable in this closeout environment and is
  recorded as unavailable rather than passed.

## Integration Points

- `src-tauri/src/state.rs`, `state_db.rs`, and `control_center.rs` own bounded durable control state.
- `src-tauri/src/corpus/mod.rs` owns deterministic feed and bounded Playbook catalog behavior.
- `src-tauri/src/install/mod.rs`, `render/mod.rs`, and `registry.rs` own exact artifact and roster
  lifecycle truth.
- `src/lib/components/ActivityHistory.svelte`, `Projects.svelte`, `Runbooks.svelte`, and the existing
  Settings sections expose the eight capabilities without adding a navigation domain.
- `openspec/specs/` contains the eight canonical capability contracts.

## Architectural Decisions

- Reused existing domain approval and recovery authority instead of creating cross-domain services.
- Kept subscriptions and readiness evaluation read-only; every mutation re-enters an existing exact
  plan/review/apply boundary.
- Kept database restore offline/manual and OpenClaw registration/restart external.
- Reused the existing single-file Antigravity path and existing persistence progress surfaces.

## Artifacts

- Implementation branch: `feat/nightly-control-plane`
- Implementation range: `b00f7d2..7bc2310`
- Final audit repair commits: `7d524e7`, `7bc2310`
- Plan: `docs/superpowers/plans/2026-08-17-nightly-control-plane.md`
- OpenSpec archive: `openspec/changes/archive/2026-08-18-nightly-control-plane/`
- Original `main` checkout: untouched and unmerged
