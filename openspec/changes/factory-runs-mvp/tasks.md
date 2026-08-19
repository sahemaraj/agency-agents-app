## 1. Factory Run model and lifecycle

- [x] 1.1 Add failing Rust tests in `src-tauri/src/expert_runs.rs` for legacy runs without Factory data, bounded work-order validation, and exact Factory serialization round trips; run the focused `expert_runs` tests.
- [x] 1.2 Extend the existing Expert Run types with the optional bounded Factory workflow, work-contract digest, fixed phases, blocker overlay, attempts, approvals, claims, artifacts, evidence, review, delivery, terminal decision, and inert improvement proposal until task 1.1 passes.
- [x] 1.3 Add failing transition tests for expected-revision concurrency, allowed/forbidden phase edges, blocker pause/resume, attempt invalidation, three-attempt exhaustion, terminal immutability, and cancellation; implement the minimum transition helpers until they pass.
- [x] 1.4 Add failing tests for claim expiry/release/generation, two-hour renewal, idempotent retries, conflicting keys, latest-check evidence, stale binding rejection, non-zero pass rejection, and distinct reviewer enforcement; implement those rules inside the existing transactional mutation authority.
- [x] 1.5 Add persistence and retention tests proving restart restores Factory state, only terminal runs are pruned, and active-capacity exhaustion fails closed; update existing retention logic without changing the 4 MiB or 500-run limits.

## 2. Creation, readiness, and desktop authority

- [x] 2.1 Expose the existing readiness calculation from `src-tauri/src/install/mod.rs` as an internal reusable helper, with focused tests proving the command and Factory preflight return the same result.
- [x] 2.2 Add failing activation tests in `src-tauri/src/experts.rs` for optional Factory work orders, Ready-only creation, blocked activation rollback, canonical contract hashing, and unchanged normal Expert activation.
- [x] 2.3 Thread the optional work order through the existing Expert activation transaction and recovery path until task 2.2 passes; do not add a second creation or persistence path.
- [x] 2.4 Add failing command tests for exact plan approve/reject, final accept/rework/reject, explicit review/check waivers, cancellation, and claim release, including stale-revision no-ops.
- [x] 2.5 Implement the desktop-only Factory commands in existing Rust modules and register them in `src-tauri/src/lib.rs`; verify no approval, waiver, or final-decision operation is exposed through MCP.

## 3. Pull-based MCP worker protocol

- [x] 3.1 Add failing tests in `src-tauri/src/skills/mcp.rs` for one canonical `claudeCode`/`claude` identity, preserved `codex`, rejected unknown/generic HTTP mutation identities, and server-context precedence over spoofed payload actors.
- [x] 3.2 Add failing router tests for exact project-scoped discovery, no global queue, claimable-phase filtering, immutable current-claim contracts, and no cross-project or cross-claim disclosure.
- [x] 3.3 Add failing authorization/audit tests proving Factory reads use existing read authority, mutations use default-denied source authority, project allowlists/client overrides/paranoid mode still apply, and every attempted mutation gets bounded terminal audit evidence.
- [x] 3.4 Register and implement the seven bounded Factory tools in the existing MCP router: discover work, claim phase, read claim contract, submit artifact, submit evidence, submit blocker, and complete phase.
- [x] 3.5 Add integration tests for concurrent claims, lease reassignment, idempotent submissions, stale plan/base/head/attempt rejection, review separation, bounded inputs, and absence of approval/execution tools.
- [x] 3.6 Run the live MCP tool inventory and an authorized/denied protocol smoke test, confirming the router exposes only the planned seven Factory operations and performs no repository or network execution.

## 4. Shared frontend contract and control room

- [x] 4.1 Add failing TypeScript/store tests for optional Factory run parsing, phase/attempt/blocker projections, current human-action projection, latest evidence, terminal receipts, and legacy non-Factory behavior.
- [x] 4.2 Extend `src/lib/types.ts`, `src/lib/api.ts`, `src/lib/stores/experts.svelte.ts`, and `src/lib/stores/activity.svelte.ts` with the minimum optional Factory contract and existing-command wrappers until task 4.1 passes.
- [x] 4.3 Add failing component tests in `src/lib/smoke.test.ts` for bounded Factory creation, phase summary/detail, client-reported provenance, plan/final actions, cancellation warning, claim release, delivery/limitations, and inert improvement proposals.
- [x] 4.4 Extend `src/lib/components/Experts.svelte` as the Factory control and decision authority, reusing existing sections, controls, disclosures, status text, and live regions; create no new route or production component unless tests demonstrate a distinct reusable unit.
- [x] 4.5 Add failing Activity tests for plan/final review projections, stale-revision refresh, delegation to Experts, exact initiating-focus restoration, terminal receipt detail, and inert HTTPS evidence.
- [x] 4.6 Extend `src/lib/components/ActivityHistory.svelte` and the existing receipt union until task 4.5 passes without duplicating approval state or automatically opening a delivery URL.

## 5. Integrated lifecycle verification

- [x] 5.1 Add one Rust integration-style lifecycle test covering create, plan claim/submission, desktop approval, build, current evidence, distinct review, delivery, and desktop acceptance with exact revision/head bindings.
- [x] 5.2 Add lifecycle regression cases for plan drift, late failing evidence, review rework, expired claimant submission, cancellation during external work, attempt exhaustion, and Activity receipt failure without repeated terminal decisions.
- [x] 5.3 Run `PATH=/Users/home/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`, full `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`; resolve every introduced failure or warning.
- [x] 5.4 Run `npm run verify:frontend` and `npm audit`; resolve every introduced type, test, build, or dependency-security failure without adding dependencies.
- [x] 5.5 Run desktop browser/E2E smoke coverage at 375 px and 1440 px plus keyboard and axe checks for creation, monitoring, both human gates, cancellation, and receipt focus restoration.
- [x] 5.6 Audit command and MCP boundaries to prove the app cannot launch models, shell/Git/tests, mutate repositories, contact pull-request services, merge, deploy, leak private paths/raw content/waiver reasons, or mislabel client-reported evidence.
- [x] 5.7 Run `openspec validate factory-runs-mvp --strict --no-interactive` and the full canonical OpenSpec suite, then perform independent code, security, and goal-backward verification before presenting the implementation for human approval.
