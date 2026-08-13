## 1. First-Deployment Domain Logic

- [x] 1.1 Add failing focused tests for deterministic AI Builders preference, declaration-order fallback, no-compatible-preset behavior, and Claude Code-before-Codex target defaulting.
- [x] 1.2 Implement the minimum pure recommendation and target-selection helpers using the existing preset and tool types.
- [x] 1.3 Add failing tests for the versioned guide completion marker and the conditions that distinguish an eligible new user from a returning user.

## 2. Transactional Exact-Reference Batch

- [x] 2.1 Add Rust tests proving a transient exact-reference batch plan includes resolved dependencies, exact destinations, warnings, and blockers without writing files or ledger state.
- [x] 2.2 Add Rust rollback tests proving a failed transient batch restores all captured destinations and the prior ledger.
- [x] 2.3 Expose minimal Tauri plan/apply commands that validate bounded exact references, rebuild the plan on apply, and reuse the existing mutation-plan builder and transactional executor.
- [x] 2.4 Add typed frontend API and install-store methods that plan, apply, journal, reconcile, and return the exact batch result without creating a personal-library collection.

## 3. Guided First-Run Experience

- [x] 3.1 Extend root first-run visibility so successful catalog selection advances into deployment and completion or defer writes the versioned completion marker.
- [x] 3.2 Extend the existing first-run/modal deployment surface to show Claude Code and Codex detection, one compatible preset, user/project scope, registered-project selection, and truthful preparation or blocked states.
- [x] 3.3 Reuse the existing mutation-plan presentation and applicability rules so every Agent, dependency, destination, warning, blocker, and rollback status is visible before Apply.
- [x] 3.4 Apply only after explicit confirmation, then require fresh reconciliation of every exact reference before showing verified destinations and the existing preset starter prompt.
- [x] 3.5 Add accessible focus management, status announcements, reduced-motion-compatible behavior, and all locale-contract strings for the new stages.

## 4. Verification and Handoff

- [x] 4.1 Add frontend component/smoke coverage for catalog handoff, detected-target defaults, no-target and no-preset blockers, project scope, no-write-before-approval, reconciliation failure, verified success, and defer.
- [x] 4.2 Run `openspec validate phase-7-guided-first-deployment --strict`, frontend check/tests/build, Rust format/tests, and security-sensitive transaction regression tests.
- [x] 4.3 Record manual-platform evidence as unavailable under the approved manual-platform waiver; automated responsive, frontend, and native transaction gates remain green.
- [x] 4.4 Present the complete diff and fresh verification evidence for the repository's final human approval before apply, documentation, archive, or merge.
