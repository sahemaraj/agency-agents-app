# Nightly Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILLS: use test-driven development for every behavior change and verification-before-completion before reporting a task complete. The user granted final approval for this nightly run; do not pause for another approval gate.

**Goal:** Add one coherent local-first control surface for review, readiness, playbooks, catalog change awareness, subscriptions, recovery, security presets, and the five requested install targets without adding a runtime, telemetry, silent mutation, or arbitrary shell execution.

**Architecture:** Reuse the existing Activity, Projects, Experts/Runbooks, Settings, corpus, install, reconciliation, history, and SQLite document boundaries. Add one capped `control-center` state document for project baselines, subscriptions, and the active-corpus change feed. Extend the existing Agent install ledger only where artifact/roster lifecycle truth belongs. All writes remain previewed, journaled, path-contained, recoverable, and committed to state only after byte verification.

**Tech Stack:** Rust 2021, Tauri 2, Tokio, Serde, rusqlite, Svelte 5, TypeScript 5.6, Vitest 4.1.

---

## Understanding Lock

- The approved scope is all eight capabilities; there is no scope cut or further human approval checkpoint in this run.
- The app remains a local-first installer/control plane, not an Agent runtime.
- Reads are bounded and deterministic. Mutations are explicit, reviewed, reversible, and never triggered by a subscription or catalog refresh.
- Existing uncommitted files in the original `main` checkout are user-owned and must remain untouched.
- Work only in `/Users/home/.config/superpowers/worktrees/agency-agents-app/nightly-control-plane` on `feat/nightly-control-plane`.

## Reuse Analysis

- Extend `src/lib/components/ActivityHistory.svelte` and the existing Agent, Skill, and Expert stores for unified review; a new approval service would duplicate domain authority.
- Extend `src/lib/components/Projects.svelte`, the registered-project APIs, Workspace Pack parser, Doctor checks, and MCP inventory for readiness; no second project registry.
- Extend `src/lib/components/Runbooks.svelte`, `src/lib/stores/runbooks.svelte.ts`, and `src-tauri/src/corpus/mod.rs` for playbooks; no new navigation section or markdown dependency.
- Extend the existing SQLite `DocumentSpec`/`PERSISTENCE_INVENTORY` pattern for one `control-center` document; separate JSON/localStorage stores would violate the control-plane architecture.
- Extend the existing settings lock/document for presets; applying multiple current commands would permit partial policy updates.
- Extend the existing install ledger, filesystem journal, history, and reconciliation paths for multi-artifact and roster targets; a parallel installer would create contradictory lifecycle truth.
- No production change is needed for Antigravity: `skill-md` rendering, destinations, and installability already exist in `src-tauri/src/render/mod.rs` and `src-tauri/src/registry.rs`. Add parity and flow evidence only.
- A new plan file is required because none of the existing phase plans cover this cross-domain program; it follows the established `docs/superpowers/plans` convention.

## Decision Log

| Decision | Alternatives considered | Review objection | Resolution |
|---|---|---|---|
| No hot database restore | Replace SQLite while the app runs | WAL, process lease, and cached state can diverge | Recovery aggregates safe existing actions and labels database restore offline/manual |
| Artifact manifest is lifecycle authority | Keep one primary destination/hash | Secondary files could drift invisibly | Every artifact participates in reconcile/history/disable/rollback/uninstall/recovery |
| Strict playbook sandbox | Trust `Runbook.doc` strings | Catalog-controlled traversal/oversize reads | Fixed roots, normalized `.md`, no links, 256 KiB per file, bounded inventory |
| Explicit readiness baseline | Infer readiness from installed state | Inference cannot prove intended state | Store exact source-aware requirements; unknown/unverifiable is never Ready |
| Recommendations are not approvals | Add a fourth approval engine | Would duplicate authority and audit semantics | Re-resolve into existing reviewed install plans; separate counts and labels |
| Complete preset matrix | Toggle global flags only | Client overrides could retain mutation access | Strict/Local Development clear overrides and set paranoid plus all six flags atomically |
| Separate roster lifecycle truth | Pretend Aider/Windsurf are one Agent | Aggregate files have many exact sources | Project-scoped roster records with deterministic membership and foreign-file refusal |
| OpenClaw file truth is honest | Run `openclaw` automatically | Arbitrary command boundary and gateway lifecycle | Install files only; report registration/restart required |
| Durable feed before cursors | Store feed in Activity/localStorage | Clear/crash could lose changes | Persist old snapshot → new feed/snapshot → recommendations → cursor advance |
| Antigravity validation only | Add new renderer/UI | Target already works | Prove current production flow and avoid regression-prone duplicate code |

Structured review disposition: **APPROVED**. The skeptic, constraint guardian, and user advocate objections were accepted and resolved; the independent arbiter confirmed every exit criterion.

## Global Safety and UX Contracts

- `control-center` cap: 4 MiB; 64 project baselines; 256 Agent refs + 256 Skill refs + 32 instruction + 32 MCP + 32 tool requirements per project; 64 subscriptions (one unique subscription per baseline); 10,000 active snapshot items; 100 feed batches and 2,000 feed items; ordinary strings <=256 characters and normalized relative paths <=512.
- Roster ledger caps: 128 records, 512 exact Agent refs and 8 artifacts per record. Raise an existing byte cap only to the measured serialized maximum plus bounded headroom.
- Multi-artifact paths come only from registry templates, are normalized relative paths without absolute/parent components, and remain below a validated no-link/reparse supported root.
- Every multi-artifact write preflights all targets, records exact prior bytes in the existing prepared filesystem journal, writes and verifies all bytes, commits ledger last, and rolls every file back on failure.
- Review source state is independently Loading, Ready, or Unavailable. A failed source is never counted as zero; totals say `partial` until every source is Ready.
- Readiness overall precedence: Not configured; Unavailable if required inspection failed; Needs attention if any row fails or is Unverifiable; Ready only when every required row is Ready. Empty groups are Not required.
- Failed refresh visibly retains the last successful timestamp, offers Retry, appends no feed batch, and generates no recommendations.
- New mode switches use ordinary visible-focus `aria-pressed` buttons and a polite live-region update, not incomplete ARIA tabs.

## Task 1: Control-Center Persistence and Catalog Change Feed

**Files:**

- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/state_db.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/corpus/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/components/SettingsSectionCatalog.svelte`
- Modify: existing Rust and frontend tests beside these modules

**Steps:**

- [ ] Write failing validator, cap, migration-inventory, deterministic-diff, rename, crash-order, stale-retention, and DTO tests.
- [ ] Add the single versioned `control-center` document and register its import validator and backup inventory.
- [ ] Snapshot the active Agent corpus by category + normalized relative path + source/body hashes.
- [ ] On successful explicit pull/refresh, atomically persist the new snapshot and bounded typed feed before any cursor can advance. Infer rename only for one unambiguous matching hash pair.
- [ ] Expose bounded list/refresh DTOs and render batches plus stale/error/Retry state in Catalog Settings.
- [ ] Run focused Rust tests and the Settings component tests.

**Acceptance:** add/update/remove/rename feed is deterministic and durable; refresh failure preserves visibly stale truth and cannot emit a recommendation.

## Task 2: Project Readiness and Opt-In Subscriptions

**Files:**

- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/commands/doctor.rs`
- Modify: `src-tauri/src/commands/mcp_clients.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/components/Projects.svelte`
- Modify: `src/lib/components/Teams.svelte`
- Modify: existing stores/tests supporting Projects and Teams

**Steps:**

- [ ] Write failing tests for no baseline, every readiness state/precedence, exact source identities, opaque Workspace Pack requirements, failed independent checks, subscription opt-in, cursor ordering, dismissal, supersession, stale ref blocking, and zero destination writes during evaluation.
- [ ] Persist exact per-project baselines and subscription preferences inside `control-center`; reuse Workspace Pack parsing but never mark opaque requirements Ready.
- [ ] Derive one explainable report from fresh Agent/Skill reconciliation, instruction inspection, MCP inventory, and tool detection.
- [ ] Derive bounded recommendations only from a successful durable feed. Opening one re-resolves exact refs and enters the existing plan/apply UI; only surfaced recommendations advance the cursor.
- [ ] Add Readiness and subscription controls to existing project/team detail surfaces with evidence and existing repair/configuration links.

**Acceptance:** readiness never overclaims; subscription evaluation is local/read-only and cannot bypass the existing reviewed deployment path.

## Task 3: Unified Review Center and Recovery Center

**Files:**

- Modify: `src/lib/components/ActivityHistory.svelte`
- Modify: `src/lib/components/AgentApprovalInbox.svelte`
- Modify: `src/lib/components/Experts.svelte`
- Modify: `src/lib/components/Settings.svelte`
- Modify: the existing Agent/Skill/Expert stores and `src/lib/stores/ui.svelte.ts`
- Modify: `src/lib/components/SettingsSectionDoctor.svelte` or the closest existing recovery surface
- Modify: `src/lib/api.ts`, `src/lib/types.ts`, and existing frontend tests

**Steps:**

- [ ] Write failing aggregation tests covering Agent, Skill, Expert change, Expert run, Expert activation, partial errors, retries, counts, and recommendation separation.
- [ ] Add Review/History modes to Activity, delegating each action/deep-link to the existing owning domain.
- [ ] Reuse Agent version history/rollback, Skill backup/rollback, and verified storage backup/reveal in one Settings recovery view; state explicitly that database restore is offline/manual.
- [ ] Preserve focus on return, announce async state, use semantic buttons/headings, and keep errors domain-specific.

**Acceptance:** one view reveals every pending shape without creating a second approval engine; all recovery actions retain their existing safety boundary.

## Task 4: Bounded Playbook Library

**Files:**

- Modify: `src-tauri/src/corpus/mod.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/stores/runbooks.svelte.ts`
- Modify: `src/lib/components/Runbooks.svelte`
- Modify: existing corpus/Runbooks tests

**Steps:**

- [ ] Write failing tar-retention, root/path/link/reparse, extension, UTF-8, per-file byte, count, sort, search, and safe-display tests.
- [ ] Retain only supported strategy/example markdown in managed snapshots and expose a bounded read-only catalog/read API.
- [ ] Add Runbooks/Playbooks modes, local search, safe preformatted text, copy, empty/error/retry states, and source-relative provenance.
- [ ] Add no markdown/HTML dependency.

**Acceptance:** cloned and managed catalogs expose the same safe docs; catalog content cannot read outside the allowed roots or execute markup.

## Task 5: Atomic Security Posture Presets

**Files:**

- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/stores/settings.svelte.ts`
- Modify: `src/lib/components/SettingsSectionMcp.svelte`
- Modify: existing settings/state/frontend tests

**Steps:**

- [ ] Write failing classification and apply tests for Strict, Local Development, Custom, corrupt settings, retained allowlist, cleared Skill/Agent client overrides, unrelated field preservation, serialization with concurrent settings writes, and no partial state.
- [ ] Add one backend preset command that uses the existing settings write lock, loads the latest document, changes the complete policy matrix, persists once, and refreshes cache once.
- [ ] Add complete before/after preview and explicit apply. Any non-exact shape is Custom.

**Acceptance:** Strict cannot leave an override-enabled mutation path; failed persistence changes nothing.

## Task 6: Multi-Artifact Kimi and OpenClaw Lifecycle

**Files:**

- Modify: `src-tauri/src/registry.rs`
- Modify: `src-tauri/src/render/mod.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src/lib/types.ts`
- Modify: existing install/render/frontend tests

**Steps:**

- [ ] Start with failing exact-render parity tests against the checked-out upstream converter and failure-injection tests for every artifact lifecycle operation.
- [ ] Replace the one-string internal render assumption with the smallest artifact-set representation while preserving existing single-file public behavior.
- [ ] Persist an artifact manifest backward-compatibly and aggregate state with Modified > Missing > Outdated > Current precedence; Disabled and SourceUnavailable retain existing semantics.
- [ ] Route install/update/disable/enable/uninstall/history/rollback/recovery through the existing journal and exact backup mechanisms for every artifact.
- [ ] Report OpenClaw file success separately from required external registration/restart; execute no CLI.

**Acceptance:** deleting or modifying any secondary artifact changes the installation state; any injected failure restores every prior artifact and ledger row.

## Task 7: Project Roster Targets and Antigravity Proof

**Files:**

- Modify: `src-tauri/src/registry.rs`
- Modify: `src-tauri/src/render/mod.rs`
- Modify: `src-tauri/src/types.rs`
- Modify: `src-tauri/src/install/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/components/InstallModal.svelte`
- Modify: related stores and existing tests

**Steps:**

- [ ] Write failing Aider/Windsurf aggregate parity, deterministic exact-reference ordering, project-only scope, foreign-file refusal, update/rollback/reconcile/recovery, caps, and no-partial-write tests.
- [ ] Add project-scoped roster plan/apply/lifecycle commands using the existing install ledger document and filesystem journal; never encode the roster as one Agent record.
- [ ] Expose roster targets only when multiple exact selected Agents can be reviewed; show aggregate destination and membership before apply.
- [ ] Add Antigravity parity and end-to-end install/reconcile/uninstall tests without production changes.

**Acceptance:** Aider/Windsurf aggregate exactly the reviewed roster and never overwrite foreign project rules; Antigravity remains unchanged and proven.

## Task 8: Integration, Security, Accessibility, and E2E Validation

**Files:**

- Modify only defects found by validation.
- Update: `memory-bank/activeContext.md`, `memory-bank/progress.md`, `memory-bank/tasks/2026-08/README.md`
- Create the approved completion record under `memory-bank/tasks/2026-08/`.

**Steps:**

- [ ] Run focused tests after each task, then the full 117+ frontend and 581+ Rust baselines with new tests included.
- [ ] Run `npm run check`, `npm run build`, strict Rust format, strict Clippy, `git diff --check`, dependency audit, OpenSpec strict validation, and prohibited shell/path scans.
- [ ] Exercise the web shim end to end for Review, Readiness, Playbooks, Feed, Recommendations, Recovery, Presets, and target planning; run native smoke where the environment supports it.
- [ ] Verify keyboard focus, semantic states, live announcements, 375 px layout where browser evidence is available, and honest Unavailable waivers otherwise.
- [ ] Dispatch final whole-diff spec and quality/security reviewers; fix every blocking/important finding and re-run affected gates.
- [ ] Persist final evidence and commit the isolated branch. Do not merge or modify the original dirty `main` checkout unless separately requested.

**Acceptance:** all eight capabilities have runnable evidence, no blocking review findings remain, and the branch is integration-ready without touching user-owned changes.

## Verification Commands

```bash
npm run test:frontend
npm run check
npm run build
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo test --manifest-path src-tauri/Cargo.toml --lib
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
openspec validate --all --strict
```
